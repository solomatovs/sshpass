use std::ffi::{c_char, CStr, CString};
use std::os::raw::c_int;
use std::ptr;
use std::sync::atomic::{AtomicI32, AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

// Enum для типа завершения
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShutdownType {
    Running = 0,
    Stoped = 1,
    SmartStop = 2,
    FastStop = 3,
    ImmediateStop = 4,
}

impl ShutdownType {
    // Получение ShutdownType из числового значения
    pub fn from_int(value: c_int) -> ShutdownType {
        match value {
            0 => ShutdownType::Running,
            1 => ShutdownType::Stoped,
            2 => ShutdownType::SmartStop,
            3 => ShutdownType::FastStop,
            4 => ShutdownType::ImmediateStop,
            _ => ShutdownType::Running, // По умолчанию
        }
    }

    // Получение числового значения из ShutdownType
    pub fn to_int(self) -> c_int {
        self as c_int
    }

    // Проверка, является ли тип "в процессе остановки"
    pub fn is_stopping(self) -> bool {
        matches!(
            self,
            ShutdownType::SmartStop | ShutdownType::FastStop | ShutdownType::ImmediateStop
        )
    }
}

// Внутренняя структура для хранения сообщения
#[derive(Debug, Clone)]
struct ShutdownMessage {
    message: Option<String>,
}

// Потокобезопасная структура для управления завершением приложения
#[derive(Debug, Clone)]
pub struct AppShutdown {
    // Тип остановки: атомарный для безопасного доступа из разных потоков
    shutdown_type: Arc<AtomicI32>,

    // Код возврата: атомарный для безопасного доступа
    code: Arc<AtomicI32>,

    // Сообщение: защищено RwLock для оптимизации чтения
    message: Arc<RwLock<ShutdownMessage>>,

    // Временные метки: атомарные для безопасного доступа
    start_time_ms: Arc<AtomicU64>,
    end_time_ms: Arc<AtomicU64>,
}

impl Default for AppShutdown {
    fn default() -> Self {
        // Создание пустого состояния
        AppShutdown {
            shutdown_type: Arc::new(AtomicI32::new(0)),
            code: Arc::new(AtomicI32::new(0)),
            message: Arc::new(RwLock::new(ShutdownMessage { message: None })),
            start_time_ms: Arc::new(AtomicU64::new(0)),
            end_time_ms: Arc::new(AtomicU64::new(0)),
        }
    }
}

impl AppShutdown {
    // Получение типа завершения как enum
    pub fn get_type(&self) -> ShutdownType {
        ShutdownType::from_int(self.shutdown_type.load(Ordering::SeqCst))
    }

    // Установка типа завершения из enum
    pub fn set_type(&self, shutdown_type: ShutdownType) {
        self.shutdown_type.store(shutdown_type.to_int(), Ordering::SeqCst);
    }

    pub fn set_code(&self, code: i32) {
        self.code.store(code, Ordering::SeqCst);
    }

    pub fn set_message(&self, message: String) {
        match self.message.write() {
            Ok(mut msg) => {
                msg.message = Some(message);
            }
            Err(e) => {
                eprintln!("Failed to set shutdown message: {}", e);
            }
        }
    }

    // Преобразование из SmartStop/FastStop/ImmediateStop в Stoped
    pub fn to_stoped(&self) {
        if self.get_type() == ShutdownType::Stoped {
            return;
        }

        self.set_type(ShutdownType::Stoped);
        self.end_time_ms.store(current_time_millis(), Ordering::SeqCst);
    }

    pub fn shutdown_smart(&self) {
        if self.get_type() == ShutdownType::SmartStop {
            return;
        }

        self.set_type(ShutdownType::SmartStop);
        self.start_time_ms.store(current_time_millis(), Ordering::SeqCst);
        self.end_time_ms.store(0, Ordering::SeqCst);
    }

    pub fn shutdown_fast(&self) {
        if self.get_type() == ShutdownType::FastStop {
            return;
        }

        self.set_type(ShutdownType::FastStop);
        self.start_time_ms.store(current_time_millis(), Ordering::SeqCst);
        self.end_time_ms.store(0, Ordering::SeqCst);
    }

    pub fn shutdown_immediate(&self) {
        if self.get_type() == ShutdownType::ImmediateStop {
            return;
        }

        self.set_type(ShutdownType::ImmediateStop);
        self.start_time_ms.store(current_time_millis(), Ordering::SeqCst);
        self.end_time_ms.store(0, Ordering::SeqCst);
    }

    // Проверки типа
    pub fn is_running(&self) -> bool {
        self.get_type() == ShutdownType::Running
    }

    pub fn is_smart_stop(&self) -> bool {
        self.get_type() == ShutdownType::SmartStop
    }

    pub fn is_fast_stop(&self) -> bool {
        self.get_type() == ShutdownType::FastStop
    }

    pub fn is_immediate_stop(&self) -> bool {
        self.get_type() == ShutdownType::ImmediateStop
    }

    pub fn is_stoped(&self) -> bool {
        self.get_type() == ShutdownType::Stoped
    }

    pub fn is_stoping(&self) -> bool {
        self.get_type().is_stopping()
    }

    // Получение полей
    pub fn get_code(&self) -> i32 {
        self.code.load(Ordering::SeqCst)
    }

    pub fn get_message(&self) -> Option<String> {
        match self.message.read() {
            Ok(msg) => msg.message.clone(),
            Err(e) => {
                eprintln!("Failed to read shutdown message: {}", e);
                None
            }
        }
    }

    pub fn get_start_time(&self) -> u64 {
        self.start_time_ms.load(Ordering::SeqCst)
    }

    pub fn get_end_time(&self) -> Option<u64> {
        let end_time = self.end_time_ms.load(Ordering::SeqCst);
        if end_time == 0 || !self.is_stoped() {
            None
        } else {
            Some(end_time)
        }
    }

    // Получение длительности (для Stoped)
    pub fn get_duration(&self) -> Option<Duration> {
        self.get_end_time()
            .map(|end| Duration::from_millis(end - self.get_start_time()))
    }
    
    // Комбинированный метод для установки кода и сообщения
    pub fn stop(&self, code: i32, message: Option<String>) {
        self.set_code(code);
        if let Some(msg) = message {
            self.set_message(msg);
        }
        self.to_stoped();
    }
    
    // Создает C-совместимую структуру для использования в C API
    pub fn as_c_struct(&self) -> CAppShutdown {
        let shutdown_type = self.shutdown_type.load(Ordering::SeqCst);
        let code = self.code.load(Ordering::SeqCst);
        let start_time_ms = self.start_time_ms.load(Ordering::SeqCst);
        let end_time_ms = self.end_time_ms.load(Ordering::SeqCst);
        
        let message_ptr = match self.get_message() {
            Some(msg) => CString::new(msg).unwrap().into_raw(),
            None => ptr::null_mut(),
        };
        
        CAppShutdown {
            shutdown_type,
            code,
            message: message_ptr,
            start_time_ms,
            end_time_ms,
        }
    }
    
    // Создает новый AppShutdown из C-совместимой структуры
    pub fn from_c_struct(c_shutdown: &CAppShutdown) -> Self {
        let message = if c_shutdown.message.is_null() {
            None
        } else {
            unsafe {
                Some(CStr::from_ptr(c_shutdown.message)
                    .to_string_lossy()
                    .into_owned())
            }
        };
        
        let shutdown = AppShutdown::default();
        shutdown.shutdown_type.store(c_shutdown.shutdown_type, Ordering::SeqCst);
        shutdown.code.store(c_shutdown.code, Ordering::SeqCst);
        shutdown.start_time_ms.store(c_shutdown.start_time_ms, Ordering::SeqCst);
        shutdown.end_time_ms.store(c_shutdown.end_time_ms, Ordering::SeqCst);
        
        if let Some(msg) = message {
            shutdown.set_message(msg);
        }
        
        shutdown
    }
}

// C-совместимая структура для FFI
#[derive(Clone, Debug)]
#[repr(C)]
pub struct CAppShutdown {
    // Тип остановки: 0 = None, 1 = Stoped, 2 = SmartStop, 3 = FastStop, 4 = ImmediateStop
    shutdown_type: c_int,

    // Код возврата
    code: c_int,

    // Сообщение (NULL если нет)
    message: *mut c_char,

    // Временные метки (миллисекунды с начала эпохи)
    start_time_ms: u64,
    end_time_ms: u64, // 0 если не применимо
}

impl Drop for CAppShutdown {
    fn drop(&mut self) {
        if !self.message.is_null() {
            unsafe {
                let _ = CString::from_raw(self.message);
            }
            self.message = ptr::null_mut();
        }
    }
}

// Вспомогательная функция для получения текущего времени в миллисекундах
fn current_time_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::from_secs(0))
        .as_millis() as u64
}
