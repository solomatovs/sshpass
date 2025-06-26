use std::{fmt::Debug, str};
use std::os::raw::c_char;
use std::sync::{Arc, Mutex, MutexGuard};
use nix::sys::eventfd::EventFd;
use nix::libc::{gettimeofday, localtime_r, strftime, timeval, suseconds_t, tm};

use thiserror::Error;
use heapless::spsc::Queue;

pub const LOG_TIMESTAMP_SIZE: usize = 20;
pub const LOG_MICROS_SIZE: usize = 6;
pub const LOG_LEVEL_SIZE: usize = 8;
pub const LOG_DELIMITERS: usize = 5;

use crate::constants::{LOG_QUEUE_MAX_LEN, LOG_MESSAGE_MAX_LEN};

// Итоговая длинна записи в логе (включает timestamp, level, delimiters, message)
pub const LOG_MESSAGE_LEN: usize = LOG_TIMESTAMP_SIZE + LOG_MICROS_SIZE + LOG_LEVEL_SIZE + LOG_DELIMITERS + LOG_MESSAGE_MAX_LEN;

/// Errors related to log entry creation and formatting.
#[derive(Debug, Error)]
pub enum LogError {
    /// Ошибка вызова `gettimeofday` — не удалось получить текущее время.
    #[error("Failed to get current time using gettimeofday()")]
    GetTimeOfDayError,
    
    /// Ошибка блокировки мьютекса
    #[error("Failed to lock mutex: {0}")]
    MutexLockError(String),
}

/// Уровни логирования
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum LogLevel {
    Trace = 0,
    Debug = 1,
    Info = 2,
    Warning = 3,
    Error = 4,
    Critical = 5,
}

impl LogLevel {
    fn as_bytes(&self) -> &'static [u8] {
        match self {
            LogLevel::Trace => b"trace",
            LogLevel::Debug => b"debug",
            LogLevel::Info => b"info",
            LogLevel::Warning => b"warning",
            LogLevel::Error => b"error",
            LogLevel::Critical => b"critical",
        }
    }
}

/// Запись в лог
#[derive(Debug, Clone)]  // Добавляем Clone для возможности копирования
#[repr(C)]
pub struct LogEntryStack {
    timestamp: Option<timeval>,
    level: Option<LogLevel>,
    message_len: usize,
    message: [u8; LOG_MESSAGE_MAX_LEN],
}

impl LogEntryStack {
    pub fn new_with_timeval(timestamp: Option<timeval>, level: Option<LogLevel>, message: &[u8]) -> Self {
        let mut res = [0u8; LOG_MESSAGE_MAX_LEN];
        let len = message.len().min(LOG_MESSAGE_MAX_LEN);
        res[..len].copy_from_slice(&message[..len]);
        
        Self {
            timestamp,
            level,
            message: res,
            message_len: len,
        }
    }

    pub fn get_timestamp() -> Result<timeval, LogError> {
        let mut timestamp = timeval {
            tv_sec: 0,
            tv_usec: 0,
        };

        let timestamp = unsafe {
            if gettimeofday(&mut timestamp, std::ptr::null_mut()) == -1 {
                return Err(LogError::GetTimeOfDayError);
            }

            timestamp
        };

        Ok(timestamp)
    }

    fn get_tm_struct(tm: Option<timeval>) -> Option<tm> {
        if let None = tm {
            return None;
        }

        let timestamp = tm.unwrap();

        let tm_struct = unsafe {
            // локальное время
            let mut tm_struct: tm = std::mem::zeroed();
            localtime_r(&timestamp.tv_sec, &mut tm_struct);
            tm_struct
        };

        Some(tm_struct)
    }

    fn get_time_buffer(tm: &Option<tm>) -> ([u8; LOG_TIMESTAMP_SIZE], usize) {
        if let None = tm {
            return ([0; LOG_TIMESTAMP_SIZE], 0);
        }

        let tm_struct = tm.unwrap();

        // формат даты-времени
        let mut time_buf = [0u8; LOG_TIMESTAMP_SIZE];
        let fmt = b"%Y-%m-%d %H:%M:%S\0";

        let len = unsafe {
                strftime(
                time_buf.as_mut_ptr() as *mut c_char,
                time_buf.len(),
                fmt.as_ptr() as *const i8,
                &tm_struct,
            )
        };

        (time_buf, len)
    }

    fn get_time_milliseconds_buffer(tm: &Option<timeval>) -> ([u8; LOG_MICROS_SIZE], usize) {
        if let None = tm {
            return ([0; LOG_MICROS_SIZE], 0);
        }

        let tm_struct = tm.unwrap();

        Self::format_usec_6digits(tm_struct.tv_usec)
    }

    fn get_level_buffer(level: &Option<LogLevel>) -> ([u8; LOG_LEVEL_SIZE], usize) {
        if let None = level {
            return ([0; LOG_LEVEL_SIZE], 0);
        }
        let level = level.unwrap();
        let level = level.as_bytes();
        let len = level.len().min(LOG_LEVEL_SIZE);

        let mut res = [0u8; LOG_LEVEL_SIZE];
        res[..len].copy_from_slice(&level[..len]);
        (res, len)
    }

    pub fn message_format(&self) -> ([u8; LOG_MESSAGE_LEN], usize) {
        let tm = Self::get_tm_struct(self.timestamp);

        // формат даты-времени
        let (time_buf, time_buf_len) = Self::get_time_buffer(&tm);

        // формат микросекунд
        let (micros_buf, micros_buf_len) = Self::get_time_milliseconds_buffer(&self.timestamp);

        // уровень как текст
        let (level_buf, level_buf_len) = Self::get_level_buffer(&self.level);

        // собираем всё
        let mut offset = 0;
        let mut buf = [0u8; LOG_MESSAGE_LEN];

        if time_buf_len > 0 {
            buf[offset..offset + time_buf_len].copy_from_slice(&time_buf[..time_buf_len]);
            offset += time_buf_len;

            buf[offset] = b'.';
            offset += 1;

            buf[offset..offset + micros_buf_len].copy_from_slice(&micros_buf[..micros_buf_len]);
            offset += micros_buf_len;

            buf[offset..offset + 2].copy_from_slice(b" [");
            offset += 2;

            buf[offset..offset + level_buf_len].copy_from_slice(&level_buf[..level_buf_len]);
            offset += level_buf_len;

            buf[offset] = b']';
            offset += 1;

            buf[offset] = b' ';
            offset += 1;
        }

        buf[offset..offset + self.message_len].copy_from_slice(&self.message[..self.message_len]);
        offset += self.message_len;

        (buf, offset)
    }

    fn format_usec_6digits(usec: suseconds_t) -> ([u8; LOG_MICROS_SIZE], usize) {
        // Ограничим только первые LOG_MICROS_SIZE знака
        // Пример: 123456 -> 123, 4 -> 004
        let digits = [
            ((usec / 100_000) % 10) as u8,
            ((usec / 10_000) % 10) as u8,
            ((usec / 1_000) % 10) as u8,
            ((usec / 100) % 10) as u8,
            ((usec / 10) % 10) as u8,
            ((usec / 1) % 10) as u8,
        ];
    
        // Преобразуем цифры в ASCII-байты
        ([
            b'0' + digits[0],
            b'0' + digits[1],
            b'0' + digits[2],
            b'0' + digits[3],
            b'0' + digits[4],
            b'0' + digits[5],
        ], LOG_MICROS_SIZE)
    }
}

// Внутренняя структура для хранения данных LogBufferStack
#[derive(Debug, Clone)]
struct LogBufferStackInner {
    inner: Queue<LogEntryStack, LOG_QUEUE_MAX_LEN>,
    event_fd: Option<Arc<EventFd>>,
}

#[derive(Debug, Clone)]
pub struct LogBufferStack {
    // Используем Mutex для защиты внутреннего состояния
    inner: Arc<Mutex<LogBufferStackInner>>,
}

impl LogBufferStack {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(LogBufferStackInner {
                inner: Queue::new(),
                event_fd: None,
            })),
        }
    }

    pub fn len(&self) -> usize {
        match self.inner.lock() {
            Ok(inner) => inner.inner.len(),
            Err(e) => {
                eprintln!("Failed to lock log buffer: {}", e);
                0
            }
        }
    }

    pub fn peek(&self) -> Option<LogEntryStack> {
        match self.inner.lock() {
            Ok(inner) => inner.inner.peek().cloned(),
            Err(e) => {
                eprintln!("Failed to lock log buffer: {}", e);
                None
            }
        }
    }

    pub fn dequeue(&self) -> Option<LogEntryStack> {
        match self.inner.lock() {
            Ok(mut inner) => inner.inner.dequeue(),
            Err(e) => {
                eprintln!("Failed to lock log buffer: {}", e);
                None
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        match self.inner.lock() {
            Ok(inner) => inner.inner.is_empty(),
            Err(e) => {
                eprintln!("Failed to lock log buffer: {}", e);
                true
            }
        }
    }

    pub fn enqueue_or_drop(&self, entry: LogEntryStack) -> Result<(), LogError> {
        let mut inner = self.inner.lock()
            .map_err(|e| LogError::MutexLockError(e.to_string()))?;
        
        if let Err(entry) = inner.inner.enqueue(entry) {
            // Удалить старейший элемент
            let _ = inner.inner.dequeue();
            // Повторно добавить (гарантированно влезет)
            let _ = inner.inner.enqueue(entry);
        }

        Self::notify_event_fd(inner)?;
        
        Ok(())
    }

    fn notify_event_fd(inner: MutexGuard<'_, LogBufferStackInner>) -> Result<(), LogError> {
        if let Some(event_fd) = &inner.event_fd {
            // Используем безопасный метод write из EventFd
            if let Err(e) = event_fd.write(1) {
                eprintln!("Failed to write to event_fd: {}", e);
            }
        }
        
        Ok(())
    }

    pub fn set_notify_event_fd(&self, event_fd: Option<Arc<EventFd>>) -> Result<(), LogError> {
        let mut inner = self.inner.lock()
            .map_err(|e| LogError::MutexLockError(e.to_string()))?;
            
        inner.event_fd = event_fd;
        
        Ok(())
    }

    pub fn log(&self, level: LogLevel, msg: &str) -> Result<(), LogError> {
        let mut timestamp = Some(LogEntryStack::get_timestamp()?);
        let mut level = Some(level);
        let bytes = msg.as_bytes();
        let mut offset = 0;
    
        while offset < bytes.len() {
            let remaining = &bytes[offset..];
            let remaining_len = remaining.len();
            let is_last_chunk = remaining_len <= LOG_MESSAGE_MAX_LEN;
            let is_first_chunk = offset == 0;
    
            let chunk_len = remaining_len.min(LOG_MESSAGE_MAX_LEN);
            let chunk = &remaining[..chunk_len];

            if !is_first_chunk {
                timestamp = None;
                level = None;
            }
    
            // Если это последний кусок и он меньше максимальной длины, то добавляем перенос строки
            if is_last_chunk && chunk_len < LOG_MESSAGE_MAX_LEN {
                let mut buffer = [0u8; LOG_MESSAGE_MAX_LEN];
                buffer[..chunk_len].copy_from_slice(chunk);
                buffer[chunk_len] = b'\n';
                let entry = LogEntryStack::new_with_timeval(timestamp, level, &buffer[..chunk_len + 1]);
                self.enqueue_or_drop(entry)?;
            // Если это не последний кусок, то добавляем перенос строки
            } else if is_last_chunk && chunk_len == LOG_MESSAGE_MAX_LEN {
                let entry = LogEntryStack::new_with_timeval(timestamp, level, chunk);
                self.enqueue_or_drop(entry)?;
    
                let entry = LogEntryStack::new_with_timeval(timestamp, level, b"\n");
                self.enqueue_or_drop(entry)?;
            // Если это не последний кусок, то добавляем перенос строки
            } else {
                let entry = LogEntryStack::new_with_timeval(timestamp, level, chunk);
                self.enqueue_or_drop(entry)?;
            }
    
            offset += chunk_len;
        }
    
        Ok(())
    }

    pub fn trace(&self, msg: &str) -> Result<(), LogError> {
        self.log(LogLevel::Trace, msg)
    }

    pub fn debug(&self, msg: &str) -> Result<(), LogError> {
        self.log(LogLevel::Debug, msg)
    }
    pub fn info(&self, msg: &str) -> Result<(), LogError> {
        self.log(LogLevel::Info, msg)
    }

    pub fn warn(&self, msg: &str) -> Result<(), LogError> {
        self.log(LogLevel::Warning, msg)
    }

    pub fn error(&self, msg: &str) -> Result<(), LogError> {
        self.log(LogLevel::Error, msg)
    }

    pub fn critical(&self, msg: &str) -> Result<(), LogError> {
        self.log(LogLevel::Critical, msg)
    }
    
    // Новый метод для получения всех сообщений из буфера
    pub fn get_all_entries(&self) -> Vec<LogEntryStack> {
        match self.inner.lock() {
            Ok(inner) => {
                let mut entries = Vec::with_capacity(inner.inner.len());
                for i in 0..inner.inner.len() {
                    if let Some(entry) = inner.inner.iter().nth(i) {
                        entries.push(entry.clone());
                    }
                }
                entries
            },
            Err(e) => {
                eprintln!("Failed to lock log buffer: {}", e);
                Vec::new()
            }
        }
    }
    
    // Новый метод для получения всех отформатированных сообщений
    pub fn get_all_formatted(&self) -> Vec<String> {
        let entries = self.get_all_entries();
        entries.iter().map(|entry| {
            let (buf, len) = entry.message_format();
            String::from_utf8_lossy(&buf[..len]).to_string()
        }).collect()
    }
    
    // Новый метод для очистки буфера
    pub fn clear(&self) -> Result<(), LogError> {
        let mut inner = self.inner.lock()
            .map_err(|e| LogError::MutexLockError(e.to_string()))?;
            
        while inner.inner.dequeue().is_some() {}
        
        Ok(())
    }
}

#[macro_export]
macro_rules! trace {
    ($logger:expr, $($arg:tt)*) => {{
        $logger.log_buffer.trace(&format!($($arg)*)).unwrap_or_else(|e| {
            eprintln!("Failed to write trace log: {:?}", e);
        });
    }}
}

#[macro_export]
macro_rules! debug {
    ($logger:expr, $($arg:tt)*) => {{
        $logger.log_buffer.debug(&format!($($arg)*)).unwrap_or_else(|e| {
            eprintln!("Failed to write debug log: {:?}", e);
        });
    }}
}

#[macro_export]
macro_rules! info {
    ($logger:expr, $($arg:tt)*) => {{
        $logger.log_buffer.info(&format!($($arg)*)).unwrap_or_else(|e| {
            eprintln!("Failed to write info log: {:?}", e);
        });
    }}
}

#[macro_export]
macro_rules! warn {
    ($logger:expr, $($arg:tt)*) => {{
        $logger.log_buffer.warn(&format!($($arg)*)).unwrap_or_else(|e| {
            eprintln!("Failed to write warn log: {:?}", e);
        });
    }}
}

#[macro_export]
macro_rules! error {
    ($logger:expr, $($arg:tt)*) => {{
        $logger.log_buffer.error(&format!($($arg)*)).unwrap_or_else(|e| {
            eprintln!("Failed to write error log: {:?}", e);
        });
    }}
}

#[macro_export]
macro_rules! critical {
    ($logger:expr, $($arg:tt)*) => {{
        $logger.log_buffer.critical(&format!($($arg)*)).unwrap_or_else(|e| {
            eprintln!("Failed to write critical log: {:?}", e);
        });
    }}
}
