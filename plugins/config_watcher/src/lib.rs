use nix::poll::PollFlags;
use std::ffi::CString;
use std::os::raw::c_int;
use std::os::unix::io::RawFd;
use nix::libc;
use thiserror::Error;

use abstractions::{error, info, trace, PluginRust, ShutdownType};
use common::read_fd::{read_fd, ReadResult};
use abstractions::buffer::Buffer;
use common::UnixContext;

// Определяем константы для inotify API
const IN_MODIFY: u32 = 0x00000002;
const IN_CLOSE_WRITE: u32 = 0x00000008;
const IN_MOVED_TO: u32 = 0x00000080;
const IN_CREATE: u32 = 0x00000100;
const IN_DELETE: u32 = 0x00000200;

// Структура для событий inotify
#[repr(C)]
struct InotifyEvent {
    wd: i32,          // Watch descriptor
    mask: u32,        // Mask of events
    cookie: u32,      // Unique cookie associating related events
    len: u32,         // Size of name field
    name: [u8; 0],    // Optional null-terminated name
}

// Определяем типы ошибок, которые могут возникнуть в плагине
#[derive(Debug, Error)]
enum PluginError {
    #[error("Read error: {0}")]
    ReadError(String),
    
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

// Определяем состояния для отслеживания паттернов редактирования
#[derive(Debug, PartialEq, Eq)]
enum EditPattern {
    // Начальное состояние
    None,
    // Паттерн 1: Редактирование "на месте"
    ModifyStarted,
    // Паттерн 2: Создание временного файла и переименование
    TempFileCreated,
    // Паттерн 3: Удаление и создание нового файла
    FileDeleted,
    // Завершенное состояние
    Completed,
}

// Определяем структуру для нашего плагина
#[derive(Debug)]
pub struct ConfigWatcherPlugin {
    inotify_fd: RawFd,
    watch_descriptor: i32,
    config_path: String,
    buf: Buffer,
    error_count: usize,
    max_errors: usize,
    edit_pattern: EditPattern,
    last_cookie: u32,  // Для отслеживания связанных событий
}

impl ConfigWatcherPlugin {
    pub fn new(ctx: &mut UnixContext) -> Result<Self, String> {
        info!(ctx, "config_watcher: plugin initializing");
        
        // Путь к файлу конфигурации
        let config_path = "config.toml".to_string();
        
        // Проверяем существование файла конфигурации
        if !std::path::Path::new(&config_path).exists() {
            return Err(format!("Config file '{}' does not exist", config_path));
        }
        
        // Инициализируем inotify
        let inotify_fd = unsafe {
            libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC)
        };
        
        if inotify_fd < 0 {
            let err = std::io::Error::last_os_error();
            return Err(format!("Failed to initialize inotify: {}", err));
        }
        
        // Добавляем сам файл конфигурации в отслеживаемые
        let c_path = CString::new(config_path.clone()).unwrap();
        let watch_descriptor = unsafe {
            libc::inotify_add_watch(
                inotify_fd,
                c_path.as_ptr(),
                IN_MODIFY | IN_CLOSE_WRITE | IN_MOVED_TO
            )
        };
        
        if watch_descriptor < 0 {
            let err = std::io::Error::last_os_error();
            unsafe { libc::close(inotify_fd) };
            return Err(format!("Failed to add watch for file '{}': {}", config_path, err));
        }
        
        info!(ctx, "Watching config file '{}' for changes", config_path);
        
        // Добавляем файловый дескриптор inotify в poll
        let flags = PollFlags::POLLIN | PollFlags::POLLERR | PollFlags::POLLHUP | PollFlags::POLLNVAL;
        ctx.poll.add_fd(inotify_fd, flags.bits());
        
        // Создаем буфер для чтения событий inotify
        // Размер буфера должен быть достаточным для нескольких событий
        let buf_size = 4096; // Обычно достаточно для нескольких событий
        let buf = Buffer::new(buf_size);
        
        Ok(ConfigWatcherPlugin {
            inotify_fd,
            watch_descriptor,
            config_path,
            buf,
            error_count: 0,
            max_errors: 5,
            edit_pattern: EditPattern::None,
            last_cookie: 0,
        })
    }
    
    // Обработка событий с детальной обработкой ошибок
    fn handle_events(&mut self, ctx: &mut UnixContext) -> Result<(), PluginError> {
        // Проверяем, есть ли события на нашем файловом дескрипторе
        let mut should_process_events = false;
        
        if let Some(fd) = ctx.poll.get_fd_mut(self.inotify_fd) {
            if fd.revents > 0 {
                if PollFlags::from_bits(fd.revents).is_none() {
                    return Err(PluginError::ReadError(format!(
                        "Unknown revents: {} on inotify fd {}",
                        fd.revents,
                        self.inotify_fd
                    )));
                }
                
                let revents = PollFlags::from_bits(fd.revents).unwrap();
                
                // Обрабатываем ошибки файлового дескриптора
                if revents.contains(PollFlags::POLLERR) {
                    return Err(PluginError::Fatal(format!(
                        "POLLERR on inotify fd {}",
                        self.inotify_fd
                    )));
                }
                if revents.contains(PollFlags::POLLNVAL) {
                    return Err(PluginError::Fatal(format!(
                        "POLLNVAL on inotify fd {}",
                        self.inotify_fd
                    )));
                }
                if revents.contains(PollFlags::POLLHUP) {
                    return Err(PluginError::Fatal(format!(
                        "POLLHUP on inotify fd {}",
                        self.inotify_fd
                    )));
                }
                
                // Обрабатываем данные, если они доступны
                if revents.contains(PollFlags::POLLIN) {
                    // Пытаемся прочитать данные
                    match read_fd(self.inotify_fd, &mut self.buf) {
                        ReadResult::Success(_) => {
                            should_process_events = true;
                        },
                        ReadResult::BufferIsFull { .. } => {
                            // Буфер заполнен, увеличиваем его размер
                            self.buf.resize(self.buf.capacity() * 2);
                            should_process_events = true;
                        },
                        ReadResult::WouldBlock { .. } => {
                            // Файловый дескриптор заблокирован, прочитаем данные в следующий раз
                        },
                        ReadResult::Interrupted { .. } => {
                            // Чтение было прервано, прочитаем данные в следующий раз
                        },
                        ReadResult::InvalidFd { .. } => {
                            return Err(PluginError::Fatal(format!(
                                "Invalid inotify fd {}",
                                self.inotify_fd
                            )));
                        },
                        ReadResult::Eof { .. } => {
                            return Err(PluginError::Fatal(format!(
                                "Inotify fd EOF {}",
                                self.inotify_fd
                            )));
                        },
                        ReadResult::Fatal { fd: _, msg } => {
                            return Err(PluginError::ReadError(format!(
                                "Inotify fd fatal {}: {}",
                                self.inotify_fd,
                                msg
                            )));
                        }
                    }
                }
            }
            fd.revents = 0;
        } else {
            // Файловый дескриптор не найден в poll
            return Err(PluginError::Fatal(format!(
                "Inotify fd {} not found in poll",
                self.inotify_fd
            )));
        }
        
        // Обрабатываем события после того, как закончили работу с fd
        if should_process_events {
            if self.process_events(ctx) {
                // Если обнаружен завершенный паттерн редактирования, устанавливаем флаг перезагрузки
                info!(ctx, "Config file change pattern detected, triggering reload");
                ctx.reload_config = true;
                self.edit_pattern = EditPattern::None; // Сбрасываем паттерн
            }
            self.buf.clear();
        }
        
        Ok(())
    }
    
    // Обработка событий inotify
    // Возвращает true, если обнаружен завершенный паттерн редактирования
    fn process_events(&mut self, ctx: &mut UnixContext) -> bool {
        let data = self.buf.as_data_slice();
        let mut offset = 0;
        
        // Размер структуры InotifyEvent без имени файла
        let event_size = std::mem::size_of::<InotifyEvent>();
        
        let mut pattern_completed = false;
        
        while offset + event_size <= data.len() {
            // Получаем указатель на структуру события
            let event = unsafe {
                &*(data.as_ptr().add(offset) as *const InotifyEvent)
            };
            
            // Проверяем, что у нас достаточно данных для имени файла
            if offset + event_size + event.len as usize > data.len() {
                break;
            }
            
            // Переходим к следующему событию после обработки текущего
            let next_offset = offset + event_size + event.len as usize;
            
            // Проверяем, что событие относится к нашему watch descriptor
            if event.wd == self.watch_descriptor {
                trace!(ctx, "Config file event: mask={:x}, cookie={}", 
                      event.mask, event.cookie);
                
                // Обновляем состояние паттерна в зависимости от типа события
                match self.edit_pattern {
                    EditPattern::None => {
                        // Начальное состояние
                        if (event.mask & IN_MODIFY) != 0 {
                            // Паттерн 1: Начало модификации "на месте"
                            self.edit_pattern = EditPattern::ModifyStarted;
                            trace!(ctx, "Pattern 1 started: IN_MODIFY");
                        }
                    },
                    EditPattern::ModifyStarted => {
                        // Ожидаем завершения модификации
                        if (event.mask & IN_CLOSE_WRITE) != 0 {
                            // Паттерн 1 завершен: Модификация + Закрытие
                            self.edit_pattern = EditPattern::Completed;
                            pattern_completed = true;
                            trace!(ctx, "Pattern 1 completed: IN_MODIFY + IN_CLOSE_WRITE");
                        }
                    },
                    EditPattern::Completed => {
                        // Уже завершено, ничего не делаем
                    },
                    _ => {
                        // Другие состояния не используются при отслеживании только файла
                    }
                }
                
                // Проверка для одиночных событий, которые могут указывать на изменение
                if (event.mask & IN_CLOSE_WRITE) != 0 && self.edit_pattern == EditPattern::None {
                    // Файл был изменен и закрыт без предварительного IN_MODIFY
                    self.edit_pattern = EditPattern::Completed;
                    pattern_completed = true;
                    trace!(ctx, "Direct write detected: IN_CLOSE_WRITE");
                }
            }
            
            // Переходим к следующему событию
            offset = next_offset;
        }
        
        pattern_completed
    }
}

impl Drop for ConfigWatcherPlugin {
    fn drop(&mut self) {
        // Удаляем watch
        let res = unsafe {
            libc::inotify_rm_watch(self.inotify_fd, self.watch_descriptor)
        };
        if res < 0 {
            let _err = std::io::Error::last_os_error();
            // error!(ctx, "Failed to remove watch: {}", err);
        }
        
        // Закрываем файловый дескриптор
        let res = unsafe {
            libc::close(self.inotify_fd)
        };
        if res < 0 {
            let _err = std::io::Error::last_os_error();
            // error!(ctx, "Failed to close inotify fd: {}", err);
        }
    }
}
impl PluginRust<UnixContext> for ConfigWatcherPlugin {
    fn free(&mut self, ctx: &mut UnixContext) -> c_int {
        info!(ctx, "config_watcher: plugin cleaning up");
        
        // Удаляем файловый дескриптор из poll
        if !ctx.poll.remove_fd(self.inotify_fd) {
            error!(ctx, "Failed to remove inotify fd {} from poll", self.inotify_fd);
        }
        
        0 // 0 означает успешное освобождение
    }
    
    fn handle(&mut self, ctx: &mut UnixContext) -> c_int {
        if ctx.shutdown.is_stoping() {
            return ShutdownType::Stoped.to_int();
        }
        
        if ctx.poll.get_result() == 0 {
            return 0;
        }
        
        // Обрабатываем события и возвращаем результат
        match self.handle_events(ctx) {
            Ok(_) => {
                // Успешная обработка, сбрасываем счетчик ошибок
                self.error_count = 0;
                0 // Успешное выполнение
            },
            Err(err) => {
                // Увеличиваем счетчик ошибок
                self.error_count += 1;
                
                // Если превышен лимит ошибок, возвращаем код ошибки
                if self.error_count > self.max_errors {
                    error!(ctx, "Too many errors ({}) in config_watcher plugin, shutting down", self.error_count);
                    return 1; // Код ошибки, который приведет к удалению плагина
                }
                
                // Логируем ошибку
                error!(ctx, "Error in config_watcher plugin: {}", err);
                
                // Возвращаем код в зависимости от типа ошибки
                err.to_return_code()
            }
        }
    }
}

#[no_mangle]
pub extern "Rust" fn register_rust_plugin(ctx: &mut UnixContext) -> Result<Box<dyn PluginRust<UnixContext>>, String> {
    let plugin = ConfigWatcherPlugin::new(ctx)?;
    let plugin = Box::new(plugin);
    
    Ok(plugin)
}
