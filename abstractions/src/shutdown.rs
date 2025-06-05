use std::ffi::{c_char, CStr, CString};
use std::os::raw::c_int;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use std::ptr;

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
        matches!(self, ShutdownType::SmartStop | ShutdownType::FastStop | ShutdownType::ImmediateStop)
    }
}

// Простая C-совместимая структура
#[derive(Clone, Debug)]
#[repr(C)]
pub struct AppShutdown {
    // Тип остановки: 0 = None, 1 = Stoped, 2 = SmartStop, 3 = FastStop, 4 = ImmediateStop
    shutdown_type: c_int,
    
    // Код возврата
    code: c_int,
    
    // Сообщение (NULL если нет)
    message: *mut c_char,
    
    // Временные метки (миллисекунды с начала эпохи)
    start_time_ms: u64,
    end_time_ms: u64,  // 0 если не применимо
}


impl Default for AppShutdown {
    fn default() -> Self {
    // Создание пустого состояния
        AppShutdown {
            shutdown_type: 0,
            code: 0,
            message: ptr::null_mut(),
            start_time_ms: 0,
            end_time_ms: 0,
        }
    }
}

impl AppShutdown {
    // Получение типа завершения как enum
    pub fn get_type(&self) -> ShutdownType {
        ShutdownType::from_int(self.shutdown_type)
    }
    
    // Установка типа завершения из enum
    pub fn set_type(&mut self, shutdown_type: ShutdownType) {
        self.shutdown_type = shutdown_type.to_int();
    }
    
    pub fn set_code(&mut self, code: i32) {
        self.code = code;
    }
    
    pub fn set_message(&mut self, message: String) {
        // Освобождаем предыдущее сообщение, если оно есть
        if !self.message.is_null() {
            unsafe {
                let _ = CString::from_raw(self.message);
            }
        }
        
        self.message = CString::new(message).unwrap().into_raw();
    }
    
    // Преобразование из SmartStop/FastStop/ImmediateStop в Stoped
    pub fn to_stoped(&mut self) {
        if self.get_type() == ShutdownType::Stoped {
            return;
        }
        
        self.set_type(ShutdownType::Stoped);
        self.end_time_ms = current_time_millis();
    }
    
    pub fn to_smart_stop(&mut self) {
        if self.get_type() == ShutdownType::SmartStop {
            return;
        }
        
        self.set_type(ShutdownType::SmartStop);
        self.start_time_ms = current_time_millis();
        self.end_time_ms = 0;
    }
    
    pub fn to_fast_stop(&mut self) {
        if self.get_type() == ShutdownType::FastStop {
            return;
        }
        
        self.set_type(ShutdownType::FastStop);
        self.start_time_ms = current_time_millis();
        self.end_time_ms = 0;
    }
    
    pub fn to_immediate_stop(&mut self) {
        if self.get_type() == ShutdownType::ImmediateStop {
            return;
        }
        
        self.set_type(ShutdownType::ImmediateStop);
        self.start_time_ms = current_time_millis();
        self.end_time_ms = 0;
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
        self.code
    }
    
    pub fn get_message(&self) -> Option<String> {
        if self.message.is_null() {
            None
        } else {
            unsafe {
                CStr::from_ptr(self.message)
                    .to_string_lossy()
                    .into_owned()
                    .into()
            }
        }
    }
    
    pub fn get_start_time(&self) -> u64 {
        self.start_time_ms
    }
    
    pub fn get_end_time(&self) -> Option<u64> {
        if self.end_time_ms == 0 || !self.is_stoped() {
            None
        } else {
            Some(self.end_time_ms)
        }
    }
    
    // Получение длительности (для Stoped)
    pub fn get_duration(&self) -> Option<Duration> {
        self.get_end_time().map(|end| Duration::from_millis(end - self.start_time_ms))
    }
}

// Реализация Drop для освобождения ресурсов
impl Drop for AppShutdown {
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

