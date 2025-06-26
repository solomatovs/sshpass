use nix::poll::PollFlags;
use nix::sys::eventfd::{EventFd, EfdFlags};
use nix::sys::timerfd::{ClockId, TimerFd, TimerFlags, Expiration, TimerSetTimeFlags};
use std::fs::OpenOptions;
use std::io::Write;
use std::sync::{Arc, Mutex};
use std::os::fd::{AsFd, RawFd};
use std::os::raw::c_int;
use std::os::unix::io::AsRawFd;
use std::time::Duration;
use std::path::Path;

use thiserror::Error;

use abstractions::{info, LogEntryStack, PluginRust, buffer::Buffer};
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

    fn register_with_poll(&self, ctx: &UnixContext) -> Result<(), String> {
        let flags = PollFlags::POLLIN | PollFlags::POLLERR | PollFlags::POLLHUP | PollFlags::POLLNVAL;
        ctx.poll.add_fd(self.fd, flags.bits());
        Ok(())
    }

    fn unregister_from_poll(&self, ctx: &UnixContext) -> bool {
        ctx.poll.remove_fd(self.fd)
    }

    fn process_signal(&mut self, ctx: &UnixContext) -> Result<bool, PluginError> {
        // Проверяем, есть ли наш fd в poll
        if !ctx.poll.has_fd(self.fd) {
            return Err(PluginError::RecoverableFdError(format!("fd {} not registered", self.fd)));
        }
        
        // Получаем revents для нашего fd
        let revents = match ctx.poll.get_revents(self.fd) {
            Some(revents) => revents,
            None => return Ok(false), // Нет событий
        };
        
        if revents == 0 {
            return Ok(false);
        }

        let revents_flags = match PollFlags::from_bits(revents) {
            Some(flags) => flags,
            None => return Err(PluginError::RecoverableFdError(format!("Unknown revents: {}", revents))),
        };

        if revents_flags.intersects(PollFlags::POLLERR | PollFlags::POLLHUP | PollFlags::POLLNVAL) {
            return Err(PluginError::RecoverableFdError(format!("FD issue on {}", self.fd)));
        }

        if revents_flags.contains(PollFlags::POLLIN) {
            match read_fd(self.fd, &mut self.buffer) {
                ReadResult::Success(_) => {
                    // Сбрасываем revents после обработки
                    ctx.poll.reset_revents(self.fd);
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

        // Сбрасываем revents после обработки
        ctx.poll.reset_revents(self.fd);
        Ok(false)
    }
}

// Структура для управления таймером
#[derive(Debug)]
struct TimerHandler {
    // timer_fd: TimerFd,
    fd_handler: FdHandler,
    // interval_secs: u64,
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
            // interval_secs,
        })
    }

    fn register(&self, ctx: &UnixContext) -> Result<(), String> {
        self.fd_handler.register_with_poll(ctx)
    }

    fn unregister(&self, ctx: &UnixContext) -> bool {
        self.fd_handler.unregister_from_poll(ctx)
    }

    fn process_signal(&mut self, ctx: &UnixContext) -> Result<bool, PluginError> {
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

    fn register(&self, ctx: &UnixContext) -> Result<(), String> {
        self.fd_handler.register_with_poll(ctx)
    }

    fn unregister(&self, ctx: &UnixContext) -> bool {
        self.fd_handler.unregister_from_poll(ctx)
    }

    fn process_signal(&mut self, ctx: &UnixContext) -> Result<bool, PluginError> {
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

    fn write_entry(&mut self, entry: LogEntryStack) -> Result<bool, PluginError> {
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
    log: Mutex<LogFileHandler>,    // Обработчик файла лога (защищен мьютексом для потокобезопасности)
    error_count: Mutex<usize>,     // Счетчик ошибок (защищен мьютексом)
    max_errors: usize,             // Максимальное количество ошибок
    ctx: Arc<UnixContext>,
}

impl LogPlugin {
    fn flush_all(&self) -> Result<(), PluginError> {
        let mut entries_written = 0;
        let mut had_errors = false;
        let mut log = self.log.lock().unwrap();

        // Обрабатываем все доступные записи
        while let Some(entry) = self.ctx.log_buffer.peek() {
            match log.write_entry(entry) {
                Ok(true) => {
                    // Успешно записали, удаляем из очереди
                    self.ctx.log_buffer.dequeue();
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
            // info!(self.ctx, "Partially flushed logs: {} of {} entries written", entries_written, entries_processed);
        } else if entries_written > 0 {
            // info!(self.ctx, "Successfully flushed {} log entries", entries_written);
        }

        Ok(())
    }

    pub fn new(ctx: Arc<UnixContext>) -> Result<Self, String> {
        info!(ctx, "Creating new LogPlugin instance");

        let log = LogFileHandler::new("application.log");
        let timer = TimerHandler::new(10)?;
        let event = EventHandler::new()?;
        
        // Регистрируем файловые дескрипторы в poll
        timer.register(&ctx)?;
        event.register(&ctx)?;
        
        // Устанавливаем event_fd в log_buffer для уведомлений о новых логах
        if let Err(e) = ctx.log_buffer.set_notify_event_fd(Some(event.event_fd.clone())) {
            return Err(e.to_string());
        }
        
        Ok(LogPlugin {
            timer,
            event,
            log: Mutex::new(log),
            error_count: Mutex::new(0),
            max_errors: 100,
            ctx,
        })
    }
}

impl Drop for LogPlugin {
    fn drop(&mut self) {
        info!(self.ctx, "logfile: plugin cleaning up");
        
        // Отключаем уведомления от log_buffer
        let _ = self.ctx.log_buffer.set_notify_event_fd(None);
        
        // Удаляем файловые дескрипторы из poll
        self.timer.unregister(&self.ctx);
        self.event.unregister(&self.ctx);
        
        // Сбрасываем все оставшиеся логи перед выходом
        if let Err(e) = self.flush_all() {
            // Здесь мы не можем использовать макрос error!, так как это может вызвать рекурсию
            eprintln!("Error flushing logs during shutdown: {}", e);
        }
    }
}

impl PluginRust<UnixContext> for LogPlugin {
    fn handle(&mut self) -> c_int {
        // Проверяем, нужно ли завершить работу
        if self.ctx.shutdown.is_stoping() {
            // Сбрасываем все логи перед выходом
            if let Err(e) = self.flush_all() {
                eprintln!("Error flushing logs during shutdown: {}", e);
            }
            return 1; // Сигнализируем о завершении плагина
        }
        
        // Если нет событий poll, ничего не делаем
        if self.ctx.poll.get_result() <= 0 {
            return 0;
        }
        
        let mut should_flush = false;
        
        // Проверяем события таймера
        match self.timer.process_signal(&self.ctx) {
            Ok(true) => {
                // Таймер сработал, нужно сбросить логи
                should_flush = true;
            },
            Ok(false) => {
                // Нет событий таймера
            },
            Err(e) => {
                // Ошибка обработки таймера
                eprintln!("Timer error: {}", e);
                let mut error_count = self.error_count.lock().unwrap();
                *error_count += 1;
                
                if *error_count > self.max_errors {
                    eprintln!("Too many timer errors, shutting down log plugin");
                    return 1; // Завершаем плагин
                }
                
                // Для некритических ошибок продолжаем работу
                return e.to_return_code();
            }
        }
        
        // Проверяем события eventfd
        match self.event.process_signal(&self.ctx) {
            Ok(true) => {
                // Получено уведомление о новых логах
                should_flush = true;
            },
            Ok(false) => {
                // Нет событий eventfd
            },
            Err(e) => {
                // Ошибка обработки eventfd
                eprintln!("Event error: {}", e);
                let mut error_count = self.error_count.lock().unwrap();
                *error_count += 1;
                
                if *error_count > self.max_errors {
                    eprintln!("Too many event errors, shutting down log plugin");
                    return 1; // Завершаем плагин
                }
                
                // Для некритических ошибок продолжаем работу
                return e.to_return_code();
            }
        }
        
        // Если нужно сбросить логи, делаем это
        if should_flush {
            match self.flush_all() {
                Ok(_) => {
                    // Успешно сбросили логи
                    let mut error_count = self.error_count.lock().unwrap();
                    if *error_count > 0 {
                        *error_count = 0; // Сбрасываем счетчик ошибок при успехе
                    }
                },
                Err(e) => {
                    // Ошибка при сбросе логов
                    eprintln!("Flush error: {}", e);
                    let mut error_count = self.error_count.lock().unwrap();
                    *error_count += 1;
                    
                    if *error_count > self.max_errors {
                        eprintln!("Too many flush errors, shutting down log plugin");
                        return 1; // Завершаем плагин
                    }
                    
                    // Для некритических ошибок продолжаем работу
                    return e.to_return_code();
                }
            }
        }
        
        0 // Успешное выполнение
    }
}

#[no_mangle]
pub extern "Rust" fn register_rust_plugin(ctx: Arc<UnixContext>) -> Result<Box<dyn PluginRust<UnixContext>>, String> {
    let plugin = LogPlugin::new(ctx)?;
    let plugin = Box::new(plugin);
    
    Ok(plugin)
}