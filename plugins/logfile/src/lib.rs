
use nix::libc::pollfd;
use nix::poll::PollFlags;
use nix::sys::eventfd::{EventFd, EfdFlags};
use nix::sys::timerfd::{ClockId, TimerFd, TimerFlags, Expiration, TimerSetTimeFlags};
use std::fs::OpenOptions;
use std::io::Write;
use std::os::fd::{AsFd, RawFd};
use std::os::raw::c_int;
use std::os::unix::io::AsRawFd;
use std::time::Duration;

use thiserror::Error;

use abstractions::{PluginRegistrator, UnixContext, LogEntryStack};
use abstractions::buffer::Buffer;
use abstractions::PluginRust;
use common::read_fd::{read_fd, ReadResult};


// Определяем типы ошибок, которые могут возникнуть в плагине
#[derive(Debug, Error)]
enum PluginError {
    // Ошибки файла лога
    #[error("File error: {0}")]
    FileError(String),
    // Ошибки файлового дескриптора, которые можно исправить пересозданием
    #[error("Recoverable fd error: {0}")]
    RecoverableFdError(String),
    // Ошибки чтения
    #[error("Read error: {0}")]
    ReadError(String),
    // Критические ошибки, требующие завершения работы плагина
    #[error("Fatal error: {0}")]
    Fatal(String),
}

impl PluginError {
    // Преобразует ошибку в код возврата для функции handle
    fn to_return_code(&self) -> c_int {
        match self {
            // Для критических ошибок возвращаем 1, что приведет к удалению плагина
            PluginError::Fatal(_) => 1,
            // Для остальных ошибок возвращаем 0, чтобы продолжить работу
            _ => 0,
        }
    }
}


// Определяем структуру для нашего плагина в Rust-стиле
#[repr(C)]
pub struct LogPlugin {
    min_level: u8,               // Минимальный уровень логирования (0-5)
    timer_fd: TimerFd,           // Файловый дескриптор таймера для периодического сброса
    timer_buffer: Buffer,        // Буфер для считывания и хранения информации от timer_fd
    event_fd: EventFd,           // Файловый дескриптор события для уведомления о новых логах
    event_buffer: Buffer,        // Буфер для считывания и хранения информации от event_fd
    log_path: String,            // Путь к файлу лога
}

impl LogPlugin {
    fn flush_entry<W: Write>(&self, writer: &mut W, entry: &LogEntryStack) -> Result<bool, PluginError> {
        let (msg, len) = entry.message_format();
        match writer.write(&msg[..len]) {
            Ok(n) if n == len => Ok(true),
            Ok(_n) => {
                Ok(false)
            },
            Err(_e) => {
                Ok(false)
            },
        }
    }

    fn flush_all(&mut self, ctx: &mut UnixContext) -> Result<(), PluginError> {
        // warn!(ctx, "Flushing log entries to '{}'", self.log_path);

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log_path)
            .map_err(|e| PluginError::FileError(e.to_string()))?;

        while let Some(entry) = ctx.log_buffer.peek() {
            if self.flush_entry(&mut file, entry)? {
                // warn!(ctx, "Flushed one log entry");
                ctx.log_buffer.dequeue();
            } else {
                // warn!(ctx, "Failed to flush a log entry; stopping flush");
                break;
            }
        }
        Ok(())
    }
    
    fn init_timer_fd() -> Result<TimerFd, String> {
        let fd = TimerFd::new(ClockId::CLOCK_MONOTONIC, TimerFlags::TFD_NONBLOCK | TimerFlags::TFD_CLOEXEC)
            .map_err(|e| format!("Failed to create timer fd: {}", e))?;
        let expiration = Expiration::Interval(Duration::from_secs(10).into());
        fd.set(expiration, TimerSetTimeFlags::empty())
            .map_err(|e| format!("Failed to set timer: {}", e))?;
        Ok(fd)
    }

    fn init_event_fd() -> Result<EventFd, String> {
        EventFd::from_value_and_flags(0, EfdFlags::EFD_NONBLOCK | EfdFlags::EFD_CLOEXEC)
            .map_err(|e| format!("Failed to create event fd: {}", e))
    }

    fn register_fd(ctx: &mut UnixContext, fd: RawFd) -> Result<(), String> {
        let flags = PollFlags::POLLIN | PollFlags::POLLERR | PollFlags::POLLHUP | PollFlags::POLLNVAL;
        // info!(ctx, "Registering fd {} with poll flags {:?}", fd, flags);
        ctx.poll.add_fd(fd, flags.bits());
        Ok(())
    }
    
    fn setup_fd(ctx: &mut UnixContext, fd: RawFd, buffer: &mut Buffer) -> Result<(), String> {
        buffer.resize(std::mem::size_of::<u64>());
        Self::register_fd(ctx, fd)?;
        Ok(())
    }

    pub fn new(ctx: &mut UnixContext) -> Result<Self, String> {
        // info!(ctx, "Creating new LogPlugin instance");

        let log_path = "application.log".into();
        let timer_fd = Self::init_timer_fd()?;
        let event_fd = Self::init_event_fd()?;
        ctx.log_buffer.set_notify_event_fd(Some(event_fd.as_raw_fd()));

        let mut plugin = Self {
            min_level: 2,
            timer_buffer: Buffer::new(8),
            event_buffer: Buffer::new(8),
            timer_fd,
            event_fd,
            log_path,
        };

        Self::setup_fd(ctx, plugin.timer_fd.as_fd().as_raw_fd(), &mut plugin.timer_buffer)?;
        Self::setup_fd(ctx, plugin.event_fd.as_raw_fd(), &mut plugin.event_buffer)?;
        Ok(plugin)
    }

    // Метод для восстановления файлового дескриптора
    fn recover_fd(&mut self, ctx: &mut UnixContext) -> Result<(), String> {
        // info!(ctx, "Recovering file descriptors");
        // Удаляем старые дескрипторы из poll
        let old_timer_fd = self.timer_fd.as_fd().as_raw_fd();
        let old_event_fd = self.event_fd.as_raw_fd();
        ctx.poll.remove_fd(old_timer_fd);
        ctx.poll.remove_fd(old_event_fd);

        // Переинициализируем дескрипторы
        let new_timer_fd = Self::init_timer_fd()?;
        let new_event_fd = Self::init_event_fd()?;

        // Обновляем дескрипторы в структуре плагина
        self.timer_fd = new_timer_fd;
        self.event_fd = new_event_fd;

        // Обновляем буферы (опционально, можно только очистить)
        self.timer_buffer.clear();
        self.timer_buffer.resize(std::mem::size_of::<u64>());
        self.event_buffer.clear();
        self.event_buffer.resize(std::mem::size_of::<u64>());

        // Регистрируем новые дескрипторы
        let timer_raw_fd = self.timer_fd.as_fd().as_raw_fd();
        let event_raw_fd = self.event_fd.as_raw_fd();

        Self::register_fd(ctx, timer_raw_fd)?;
        Self::register_fd(ctx, event_raw_fd)?;

        // Устанавливаем новый event_fd в лог-буфер для уведомлений
        ctx.log_buffer.set_notify_event_fd(Some(event_raw_fd));

        // info!(ctx, "Successfully recovered fds: timer={} event={}", timer_raw_fd, event_raw_fd);
        Ok(())
    }

    fn process_signal(fd: &mut pollfd, buf: &mut Buffer) -> Result<bool, PluginError> {
        if fd.revents == 0 {
            return Ok(false);
        }

        let raw_fd = fd.fd;
        let revents = PollFlags::from_bits(fd.revents)
            .ok_or_else(|| PluginError::ReadError(format!("Unknown revents: {}", fd.revents)))?;

        if revents.intersects(PollFlags::POLLERR | PollFlags::POLLHUP | PollFlags::POLLNVAL) {
            return Err(PluginError::RecoverableFdError(format!("FD issue on {}", raw_fd)));
        }

        if revents.contains(PollFlags::POLLIN) {
            match read_fd(raw_fd, buf) {
                ReadResult::Success(_) => {
                    fd.revents = 0;
                    buf.clear();
                    return Ok(true);
                }
                ReadResult::BufferIsFull { data_len, .. } => {
                    if data_len < 8 {
                        buf.resize(8 - data_len);
                    } else {
                        buf.clear();
                    }
                }
                ReadResult::WouldBlock { .. } |
                ReadResult::Interrupted { .. } => {}
                ReadResult::InvalidFd { .. } => {
                    return Err(PluginError::RecoverableFdError(format!("Invalid fd {}", raw_fd)));
                }
                ReadResult::Eof { .. } => {
                    return Err(PluginError::Fatal(format!("EOF on fd {}", raw_fd)));
                }
                ReadResult::Fatal { msg, .. } => {
                    return Err(PluginError::ReadError(format!("Fatal on fd {}: {}", raw_fd, msg)));
                }
            }
        }

        Ok(false)
    }

    // Обработка событий с детальной обработкой ошибок
    fn handle_events(&mut self, ctx: &mut UnixContext) -> Result<(), PluginError> {
        let mut should_flush = false;

        for (fd, buf) in [
            (self.timer_fd.as_fd().as_raw_fd(), &mut self.timer_buffer),
            (self.event_fd.as_raw_fd(), &mut self.event_buffer),
        ] {
            let poll_entry = ctx.poll.get_fd_mut(fd)
                .ok_or_else(|| PluginError::RecoverableFdError(format!("fd {} not registered", fd)))?;

            if Self::process_signal(poll_entry, buf)? {
                // info!(ctx, "Signal received on fd {}, will flush logs", fd);
                should_flush = true;
            }
        }

        if should_flush {
            self.flush_all(ctx)?;
        }

        Ok(())
    }
}

impl PluginRust<UnixContext> for LogPlugin {
    fn free(&mut self, ctx: &mut UnixContext) -> c_int {
        fn try_remove_fd(ctx: &mut UnixContext, fd: RawFd, _name: &str) {
            if !ctx.poll.remove_fd(fd) {
                // let _ = warn!(ctx, "Failed to remove {} fd {} from poll", name, fd);
            } else {
                // info!(ctx, "Successfully removed {} fd {} from poll", name, fd);
            }
        }

        try_remove_fd(ctx, self.timer_fd.as_fd().as_raw_fd(), "timer");
        try_remove_fd(ctx, self.event_fd.as_raw_fd(), "event");

        ctx.log_buffer.set_notify_event_fd(None);
        // info!(ctx, "LogPlugin resources cleaned up");

        0
    }

    fn handle(&mut self, ctx: &mut UnixContext) -> c_int {
        match self.handle_events(ctx) {
            Ok(_) => 0,
            Err(e) => {
                // warn!(ctx, "Error in handle: {}", e);
                if let PluginError::RecoverableFdError(_) = e {
                    if let Err(_e) = self.recover_fd(ctx) {
                        // warn!(ctx, "Failed to recover fds: {}", e);
                        1
                    } else {
                        0
                    }
                } else {
                    e.to_return_code()
                }
            }
        }
    }
}


/// Creates a new instance of LogPlugin.
///
/// # Safety
///
/// The caller must ensure that:
/// - `ctx` is a valid, non-null pointer to a properly initialized `UnixContext`
/// - The `UnixContext` pointed to by `ctx` remains valid for the duration of the call
/// - The `UnixContext` is not being mutably accessed from other parts of the code during this call
#[no_mangle]
pub extern "Rust" fn register_rust_plugin(
    registrator: &mut PluginRegistrator<UnixContext>,
) -> Result<(), String> {
    let plugin = LogPlugin::new(registrator.get_context())?;
    let plugin = Box::new(plugin);

    registrator.add_plugin(plugin);

    Ok(())
}
