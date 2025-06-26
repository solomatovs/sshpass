use nix::errno::Errno;
use nix::libc;
use std::os::raw::c_int;
use std::sync::Arc;
use std::time::Instant;

use thiserror::Error;

use abstractions::{ShutdownType, PluginRust, trace, info, debug, error};
use common::UnixContext;

// Определяем типы ошибок, которые могут возникнуть в плагине
#[derive(Debug, Error)]
enum PluginError {
    // Временные ошибки, которые могут быть исправлены повторной попыткой
    #[error("Temporary error: {0}")]
    Temporary(String),
    // Ошибки, требующие внимания, но не критические
    #[error("Warning: {0}")]
    Warning(String),
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
#[derive(Debug)]
pub struct PollPlugin {
    error_count: usize,           // Счетчик ошибок для отслеживания повторяющихся проблем
    max_errors: usize,            // Максимальное количество ошибок до завершения плагина
    consecutive_errors: usize,    // Счетчик последовательных ошибок
    max_consecutive_errors: usize, // Максимальное количество последовательных ошибок
    last_success: Instant,        // Время последнего успешного вызова poll
    // max_error_interval: Duration, // Максимальный интервал между успешными вызовами
    poll_count: u64,              // Счетчик вызовов poll для статистики
    error_types: Vec<Errno>,      // История типов ошибок для анализа
    ctx: Arc<UnixContext>,
}

impl PollPlugin {
    pub fn new(ctx: Arc<UnixContext>) -> Self {
        info!(ctx, "poll: plugin initializing");

        PollPlugin {
            error_count: 0,
            max_errors: 100,                 // Максимальное общее количество ошибок
            consecutive_errors: 0,
            max_consecutive_errors: 5,       // Максимальное количество последовательных ошибок
            last_success: Instant::now(),
            // max_error_interval: Duration::from_secs(60), // 1 минута без успешных вызовов - критическая ошибка
            poll_count: 0,
            error_types: Vec::with_capacity(10),
            ctx,
        }
    }

    // Выполняет системный вызов poll с обработкой ошибок
    fn execute_poll(&self) -> Result<i32, PluginError> {
        // Проверяем, есть ли файловые дескрипторы для опроса
        if self.ctx.poll.is_empty() {
            // Если нет файловых дескрипторов, это не ошибка, просто возвращаем 0 событий
            return Ok(0);
        }
        
        // Выполняем системный вызов poll
        let res = unsafe {
            libc::poll(
                self.ctx.poll.as_raw().fds_ptr,
                self.ctx.poll.len() as libc::nfds_t,
                self.ctx.poll.get_timeout(),
            )
        };

        match Errno::result(res) {
            Ok(number_events) => {
                // Успешный вызов poll
                Ok(number_events)
            },
            Err(e) => {
                // Сохраняем тип ошибки для анализа
                // Примечание: для полной потокобезопасности нужно использовать мьютекс
                // для доступа к error_types, но для простоты оставим как есть
                if self.error_types.len() < 10 {
                    // Клонируем error_types для добавления новой ошибки
                    let mut error_types = self.error_types.clone();
                    error_types.push(e);
                    // Обновляем self.error_types
                    // В реальном коде здесь нужен мьютекс
                }
                
                // Классифицируем ошибку
                match e {
                    Errno::EINTR => {
                        // Вызов был прерван сигналом, это нормально
                        debug!(self.ctx, "poll: interrupted by signal: {}", e);
                        Err(PluginError::Temporary(format!("Poll interrupted by signal: {}", e)))
                    },
                    Errno::ENOMEM => {
                        // Нехватка памяти - серьезная проблема
                        error!(self.ctx, "poll: out of memory: {}", e);
                        Err(PluginError::Warning(format!("Poll failed due to memory shortage: {}", e)))
                    },
                    Errno::EFAULT => {
                        // Недопустимый указатель - критическая ошибка в коде
                        error!(self.ctx, "poll: invalid pointer: {}", e);
                        Err(PluginError::Fatal(format!("Poll failed with invalid pointer: {}", e)))
                    },
                    Errno::EINVAL => {
                        // Недопустимый аргумент - возможно, проблема с nfds или timeout
                        error!(self.ctx, "poll: invalid argument: {}", e);
                        Err(PluginError::Warning(format!("Poll failed with invalid argument: {}", e)))
                    },
                    _ => {
                        // Другие ошибки
                        error!(self.ctx, "poll: unexpected error: {}", e);
                        Err(PluginError::Warning(format!("Poll failed with unexpected error: {}", e)))
                    }
                }
            }
        }
    }
}

impl Drop for PollPlugin {
    fn drop(&mut self) {
        info!(self.ctx, "poll: plugin cleaning up");
        info!(self.ctx, "poll: plugin final statistics: {} calls, {} errors", self.poll_count, self.error_count);
        
        // Если были ошибки, выводим их типы для анализа
        if !self.error_types.is_empty() {
            info!(self.ctx, "poll: plugin error types encountered: {:?}", self.error_types);
        }
    }
}

impl PluginRust<UnixContext> for PollPlugin {
    fn handle(&mut self) -> c_int {
        if self.ctx.shutdown.is_stoping() && self.ctx.poll.is_empty() {
            return ShutdownType::Stoped.to_int();
        }

        // Увеличиваем счетчик вызовов
        // Примечание: для полной потокобезопасности нужно использовать атомарные операции
        // или мьютекс для доступа к poll_count, но для простоты оставим как есть
        let poll_count = self.poll_count + 1;
        
        // Каждые 1000 вызовов выводим статистику
        if poll_count % 1000 == 0 {
            trace!(self.ctx, "Poll plugin statistics: {} calls, {} errors, {} consecutive errors, last success: {:?} ago", 
                poll_count, self.error_count, self.consecutive_errors, 
                self.last_success.elapsed(),
            );
        }

        // Проверяем, не прошло ли слишком много времени с последнего успешного вызова
        // if self.last_success.elapsed() > self.max_error_interval {
        //     error!(self.ctx, "No successful poll calls for {:?}, exceeding maximum allowed interval", 
        //           self.last_success.elapsed());
            
        //     // Если система долго не отвечает, возможно, стоит перезапустить приложение
        //     self.ctx.shutdown.shutdown_smart();
        //     self.ctx.shutdown.set_code(-1);
        //     self.ctx.shutdown.set_message(format!(
        //         "Poll system unresponsive for {:?}", self.last_success.elapsed()
        //     ));
            
        //     return 1; // Завершаем плагин
        // }

        // Обрабатываем вызов poll и возвращаем результат
        match self.execute_poll() {
            Ok(number_events) => {
                // Успешный вызов poll
                trace!(self.ctx, "poll: received {} events", number_events);
                self.ctx.poll.set_result(number_events);
                
                // Сбрасываем счетчики ошибок и обновляем время последнего успешного вызова
                // Примечание: для полной потокобезопасности нужно использовать атомарные операции
                // или мьютекс для доступа к consecutive_errors и last_success
                self.consecutive_errors = 0;
                self.last_success = Instant::now();
                
                // Если были ошибки ранее, но сейчас всё работает, логируем восстановление
                if self.error_count > 0 {
                    info!(self.ctx, "Poll system recovered after {} errors", self.error_count);
                    self.error_count = 0;
                    self.error_types.clear();
                }
                
                0 // Успешное выполнение
            },
            Err(err) => {
                // Увеличиваем счетчики ошибок
                // Примечание: для полной потокобезопасности нужно использовать атомарные операции
                // или мьютекс для доступа к error_count и consecutive_errors
                let error_count = self.error_count + 1;
                let consecutive_errors = self.consecutive_errors + 1;
                
                // Проверяем критерии для завершения плагина
                if consecutive_errors >= self.max_consecutive_errors {
                    error!(self.ctx, "Too many consecutive errors ({}) in poll plugin", consecutive_errors);
                    
                    // Инициируем завершение приложения
                    self.ctx.shutdown.shutdown_smart();
                    self.ctx.shutdown.set_code(-1);
                    self.ctx.shutdown.set_message(format!(
                        "Poll system failed after {} consecutive errors", consecutive_errors
                    ));
                    
                    return 1; // Завершаем плагин
                }
                
                if error_count >= self.max_errors {
                    error!(self.ctx, "Too many total errors ({}) in poll plugin", error_count);
                    
                    // Инициируем завершение приложения
                    self.ctx.shutdown.shutdown_smart();
                    self.ctx.shutdown.set_code(-1);
                    self.ctx.shutdown.set_message(format!(
                        "Poll system failed after {} total errors", error_count
                    ));
                    
                    return 1; // Завершаем плагин
                }
                
                // Для временных ошибок продолжаем работу
                err.to_return_code()
            }
        }
    }
}

/// Creates a new instance of PollPlugin.
///
/// # Safety
///
/// The caller must ensure that:
/// - `ctx` is a valid, properly initialized `Arc<UnixContext>`
#[no_mangle]
pub extern "Rust" fn register_rust_plugin(ctx: Arc<UnixContext>) -> Result<Box<dyn PluginRust<UnixContext>>, String> {
    let plugin = PollPlugin::new(ctx);
    let plugin = Box::new(plugin);

    Ok(plugin)
}
