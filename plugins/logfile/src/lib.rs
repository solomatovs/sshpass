use nix::poll::PollFlags;
use nix::sys::eventfd::{EventFd, EfdFlags};
use nix::sys::timerfd::{ClockId, TimerFd, TimerFlags, Expiration, TimerSetTimeFlags};
use std::fs::OpenOptions;
use std::io::Write;
use std::sync::Arc;
use std::os::fd::{AsFd, RawFd};
use std::os::raw::c_int;
use std::os::unix::io::AsRawFd;
use std::time::Duration;
use std::path::Path;

use thiserror::Error;

use abstractions::LogEntryStack;
use abstractions::buffer::Buffer;
use abstractions::PluginRust;
use common::read_fd::{read_fd, ReadResult};
use common::UnixContext;

// Определяем типы ошибок, которые могут возникнуть в плагине
#[derive(Debug, Error)]
enum PluginError {
    // Ошибки файла лога
    #[error("File error: {0}")]
    FileError(String),
    // Ошибки файлового дескриптора, которые можно исправить пересозданием
    #[error("Recoverable fd error: {0}")]
    RecoverableFdError(String),
    // Критические ошибки, требующие завершения работы плагина
    #[allow(dead_code)]
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

// структура для управления файловым дескриптором и его буфером
#[derive(Debug)]
struct FdHandler {
    fd: RawFd,
    buffer: Buffer,
}

impl FdHandler {
    fn new(fd: RawFd, buffer_size: usize) -> Self {
        Self {
            fd,
            buffer: Buffer::new(buffer_size),
        }
    }

    fn register_with_poll(&self, ctx: &mut UnixContext) -> Result<(), String> {
        let flags = PollFlags::POLLIN | PollFlags::POLLERR | PollFlags::POLLHUP | PollFlags::POLLNVAL;
        ctx.poll.add_fd(self.fd, flags.bits());
        Ok(())
    }

    fn unregister_from_poll(&self, ctx: &mut UnixContext) -> bool {
        ctx.poll.remove_fd(self.fd)
    }

    fn process_signal(&mut self, ctx: &mut UnixContext) -> Result<bool, PluginError> {
        let poll_entry = ctx.poll.get_fd_mut(self.fd)
            .ok_or_else(|| PluginError::RecoverableFdError(format!("fd {} not registered", self.fd)))?;
        
        if poll_entry.revents == 0 {
            return Ok(false);
        }

        let revents = PollFlags::from_bits(poll_entry.revents)
            .ok_or_else(|| PluginError::RecoverableFdError(format!("Unknown revents: {}", poll_entry.revents)))?;

        if revents.intersects(PollFlags::POLLERR | PollFlags::POLLHUP | PollFlags::POLLNVAL) {
            return Err(PluginError::RecoverableFdError(format!("FD issue on {}", self.fd)));
        }

        if revents.contains(PollFlags::POLLIN) {
            match read_fd(self.fd, &mut self.buffer) {
                ReadResult::Success(_) => {
                    poll_entry.revents = 0;
                    self.buffer.clear();
                    return Ok(true);
                }
                ReadResult::BufferIsFull { data_len, .. } => {
                    let capacity = self.buffer.capacity();
                    if data_len < capacity  {
                        self.buffer.resize(capacity - data_len);
                    } else {
                        self.buffer.clear();
                    }
                }
                ReadResult::WouldBlock { .. } |
                ReadResult::Interrupted { .. } => {}
                ReadResult::InvalidFd { .. } => {
                    return Err(PluginError::RecoverableFdError(format!("Invalid fd {}", self.fd)));
                }
                ReadResult::Eof { .. } => {
                    return Err(PluginError::RecoverableFdError(format!("EOF on fd {}", self.fd)));
                }
                ReadResult::Fatal { msg, .. } => {
                    return Err(PluginError::Fatal(format!("Fatal on fd {}: {}", self.fd, msg)));
                }
            }
        }

        Ok(false)
    }
}

// Структура для управления таймером
#[derive(Debug)]
struct TimerHandler {
    // timer_fd: TimerFd,
    fd_handler: FdHandler,
    interval_secs: u64,
}

impl TimerHandler {
    fn new(interval_secs: u64) -> Result<Self, String> {
        let timer_fd = TimerFd::new(ClockId::CLOCK_MONOTONIC, TimerFlags::TFD_NONBLOCK | TimerFlags::TFD_CLOEXEC)
            .map_err(|e| format!("Failed to create timer fd: {}", e))?;
        
        let expiration = Expiration::Interval(Duration::from_secs(interval_secs).into());
        timer_fd.set(expiration, TimerSetTimeFlags::empty())
            .map_err(|e| format!("Failed to set timer: {}", e))?;
        
        let raw_fd = timer_fd.as_fd().as_raw_fd();
        let fd_handler = FdHandler::new(raw_fd, std::mem::size_of::<u64>());
        
        Ok(Self {
            // timer_fd,
            fd_handler,
            interval_secs,
        })
    }

    fn register(&self, ctx: &mut UnixContext) -> Result<(), String> {
        self.fd_handler.register_with_poll(ctx)
    }

    fn unregister(&self, ctx: &mut UnixContext) -> bool {
        self.fd_handler.unregister_from_poll(ctx)
    }

    fn process_signal(&mut self, ctx: &mut UnixContext) -> Result<bool, PluginError> {
        self.fd_handler.process_signal(ctx)
    }
}

// Структура для управления event fd
#[derive(Debug)]
struct EventHandler {
    event_fd: Arc<EventFd>,
    fd_handler: FdHandler,
}

impl EventHandler {
    fn new() -> Result<Self, String> {
        let event_fd = EventFd::from_value_and_flags(0, EfdFlags::EFD_NONBLOCK | EfdFlags::EFD_CLOEXEC)
            .map_err(|e| format!("Failed to create event fd: {}", e))?;
        
        let raw_fd = event_fd.as_raw_fd();
        let fd_handler = FdHandler::new(raw_fd, std::mem::size_of::<u64>());
        
        Ok(Self {
            event_fd: Arc::new(event_fd),
            fd_handler,
        })
    }

    fn register(&self, ctx: &mut UnixContext) -> Result<(), String> {
        self.fd_handler.register_with_poll(ctx)
    }

    fn unregister(&self, ctx: &mut UnixContext) -> bool {
        self.fd_handler.unregister_from_poll(ctx)
    }

    fn process_signal(&mut self, ctx: &mut UnixContext) -> Result<bool, PluginError> {
        self.fd_handler.process_signal(ctx)
    }
}

// Структура для управления файлом лога
#[derive(Debug)]
struct LogFileHandler {
    path: String,
    last_error_time: Option<std::time::Instant>,
    retry_interval: Duration,
}

impl LogFileHandler {
    fn new(path: &str) -> Self {
        Self {
            path: path.to_string(),
            last_error_time: None,
            retry_interval: Duration::from_secs(5), // Повторная попытка через 5 секунд
        }
    }

    fn open_file(&mut self) -> Result<std::fs::File, PluginError> {
        // Проверяем, существует ли директория для лога
        if let Some(parent) = Path::new(&self.path).parent() {
            if !parent.exists() {
                if let Err(e) = std::fs::create_dir_all(parent) {
                    return Err(PluginError::FileError(format!("Failed to create log directory: {}", e)));
                }
            }
        }

        // Открываем файл с обработкой ошибок
        match OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
        {
            Ok(file) => {
                // Сбрасываем время последней ошибки при успешном открытии
                self.last_error_time = None;
                Ok(file)
            }
            Err(e) => {
                // Запоминаем время ошибки для ограничения частоты повторных попыток
                let now = std::time::Instant::now();
                self.last_error_time = Some(now);
                Err(PluginError::FileError(format!("Failed to open log file '{}': {}", self.path, e)))
            }
        }
    }

    fn can_retry(&self) -> bool {
        match self.last_error_time {
            Some(time) => std::time::Instant::now().duration_since(time) >= self.retry_interval,
            None => true,
        }
    }

    fn write_entry(&mut self, entry: &LogEntryStack) -> Result<bool, PluginError> {
        // Если недавно была ошибка, и еще не прошло время для повторной попытки, пропускаем
        if !self.can_retry() {
            return Ok(false);
        }

        // Открываем файл
        let mut file = match self.open_file() {
            Ok(file) => file,
            Err(e) => {
                // Если не удалось открыть файл, возвращаем ошибку, но не критическую
                return Err(e);
            }
        };

        // Форматируем сообщение
        let (msg, len) = entry.message_format();

        // Пишем в файл
        match file.write(&msg[..len]) {
            Ok(n) if n == len => Ok(true),
            Ok(_) => {
                // Записали не все данные
                self.last_error_time = Some(std::time::Instant::now());
                Err(PluginError::FileError("Partial write to log file".to_string()))
            },
            Err(e) => {
                // Ошибка записи
                self.last_error_time = Some(std::time::Instant::now());
                Err(PluginError::FileError(format!("Failed to write to log file: {}", e)))
            },
        }
    }
}

// Определяем структуру для нашего плагина в Rust-стиле
#[derive(Debug)]
pub struct LogPlugin {
    timer: TimerHandler,    // Обработчик таймера для периодического сброса
    event: EventHandler,    // Обработчик событий для уведомления о новых логах
    log: LogFileHandler,    // Обработчик файла лога
}


impl LogPlugin {
    fn flush_all(&mut self, ctx: &mut UnixContext) -> Result<(), PluginError> {
        let mut entries_written = 0;
        let mut had_errors = false;

        // Обрабатываем все доступные записи
        while let Some(entry) = ctx.log_buffer.peek() {
            match self.log.write_entry(entry) {
                Ok(true) => {
                    // Успешно записали, удаляем из очереди
                    ctx.log_buffer.dequeue();
                    entries_written += 1;
                },
                Ok(false) => {
                    // Пропускаем запись (например, из-за недавней ошибки)
                    break;
                },
                Err(e) => {
                    // Была ошибка при записи
                    had_errors = true;
                    
                    // Если это не критическая ошибка, продолжаем работу
                    match e {
                        PluginError::FileError(_) => {
                            // Прекращаем обработку на время, но не удаляем записи
                            break;
                        },
                        _ => return Err(e),
                    }
                }
            }
        }

        // Если были ошибки, но мы все равно записали что-то, считаем это успехом
        if had_errors && entries_written > 0 {
            // info!(ctx, "Partially flushed logs: {} of {} entries written", entries_written, entries_processed);
        } else if entries_written > 0 {
            // info!(ctx, "Successfully flushed {} log entries", entries_written);
        }

        Ok(())
    }

    pub fn new(ctx: &mut UnixContext) -> Result<Self, String> {
        // info!(ctx, "Creating new LogPlugin instance");

        let log = LogFileHandler::new("application.log");
        let timer = TimerHandler::new(10)?;
        let event = EventHandler::new()?;
        
        // Регистрируем обработчики в poll
        timer.register(ctx)?;
        event.register(ctx)?;
        
        // Устанавливаем event fd для уведомлений о новых логах
        ctx.log_buffer.set_notify_event_fd(Some(event.event_fd.clone()));

        Ok(Self {
            timer,
            event,
            log,
        })
    }

    // Метод для восстановления файловых дескрипторов
    // Метод для восстановления файловых дескрипторов
    fn recover_fd(&mut self, ctx: &mut UnixContext) -> Result<(), String> {
        // info!(ctx, "Recovering file descriptors");
        
        // Удаляем старые дескрипторы из poll
        self.timer.unregister(ctx);
        self.event.unregister(ctx);
        
        // Переинициализируем обработчики
        let new_timer = TimerHandler::new(self.timer.interval_secs)?;
        let new_event = EventHandler::new()?;
        
        // Регистрируем новые обработчики
        new_timer.register(ctx)?;
        new_event.register(ctx)?;
        
        // Устанавливаем новый event_fd в лог-буфер для уведомлений
        ctx.log_buffer.set_notify_event_fd(Some(new_event.event_fd.clone()));
        
        // Обновляем обработчики в структуре плагина
        self.timer = new_timer;
        self.event = new_event;
        
        // info!(ctx, "Successfully recovered fds: timer={} event={}", self.timer.get_fd(), self.event.get_fd());
        Ok(())
    }

    // Обработка событий с детальной обработкой ошибок
    fn handle_events(&mut self, ctx: &mut UnixContext) -> Result<(), PluginError> {
        let mut should_flush = false;

        // Обрабатываем сигналы от таймера
        match self.timer.process_signal(ctx) {
            Ok(true) => {
                // info!(ctx, "Timer signal received, will flush logs");
                should_flush = true;
            },
            Ok(false) => {},
            Err(e) => {
                // warn!(ctx, "Error processing timer signal: {}", e);
                // Для ошибок таймера пытаемся восстановиться
                if let PluginError::RecoverableFdError(_) = e {
                    if let Err(recover_err) = self.recover_fd(ctx) {
                        // warn!(ctx, "Failed to recover timer fd: {}", recover_err);
                        return Err(PluginError::RecoverableFdError(recover_err));
                    }
                } else {
                    return Err(e);
                }
            }
        }

        // Обрабатываем сигналы от event fd
        match self.event.process_signal(ctx) {
            Ok(true) => {
                // info!(ctx, "Event signal received, will flush logs");
                should_flush = true;
            },
            Ok(false) => {},
            Err(e) => {
                // warn!(ctx, "Error processing event signal: {}", e);
                // Для ошибок event fd пытаемся восстановиться
                if let PluginError::RecoverableFdError(_) = e {
                    if let Err(recover_err) = self.recover_fd(ctx) {
                        // warn!(ctx, "Failed to recover event fd: {}", recover_err);
                        return Err(PluginError::RecoverableFdError(recover_err));
                    }
                } else {
                    return Err(e);
                }
            }
        }

        if should_flush {
            // Пытаемся сбросить логи, но обрабатываем ошибки
            match self.flush_all(ctx) {
                Ok(_) => {},
                Err(e) => {
                    // warn!(ctx, "Error flushing logs: {}", e);
                    // Для ошибок файла не прерываем работу, просто логируем
                    if let PluginError::FileError(_) = e {
                        // Ошибки файла не критичны, продолжаем работу
                    } else {
                        return Err(e);
                    }
                }
            }
        }

        Ok(())
    }
}

impl PluginRust<UnixContext> for LogPlugin {
    fn free(&mut self, ctx: &mut UnixContext) -> c_int {
        // Удаляем файловые дескрипторы из poll
        self.timer.unregister(ctx);
        self.event.unregister(ctx);

        // Отключаем уведомления о новых логах
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
pub extern "Rust" fn register_rust_plugin(ctx: &mut UnixContext) -> Result<Box<dyn PluginRust<UnixContext>>, String> {
    let plugin = LogPlugin::new(ctx)?;
    let plugin = Box::new(plugin);

    Ok(plugin)
}
