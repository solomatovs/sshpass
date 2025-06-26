use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::{AppShutdown, LogBufferStack, UnixPoll, AppContext};

/// Потокобезопасная структура для управления флагом перезагрузки конфигурации
#[derive(Debug, Clone)]
pub struct ReloadConfig {
    flag: Arc<AtomicBool>,
}

impl ReloadConfig {
    /// Создает новый экземпляр ReloadConfig
    pub fn new() -> Self {
        Self {
            flag: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Проверяет, установлен ли флаг перезагрузки
    pub fn is_reload_needed(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }

    /// Устанавливает флаг перезагрузки
    pub fn set_reload_needed(&self) {
        self.flag.store(true, Ordering::SeqCst);
    }

    /// Сбрасывает флаг перезагрузки
    pub fn reset_reload_flag(&self) {
        self.flag.store(false, Ordering::SeqCst);
    }

    /// Устанавливает флаг перезагрузки в указанное значение
    pub fn set_reload_flag(&self, value: bool) {
        self.flag.store(value, Ordering::SeqCst);
    }

    /// Атомарно проверяет и сбрасывает флаг перезагрузки
    /// Возвращает true, если флаг был установлен и был сброшен
    pub fn check_and_reset(&self) -> bool {
        // Атомарно меняем true на false и возвращаем предыдущее значение
        self.flag.swap(false, Ordering::SeqCst)
    }
}

impl Default for ReloadConfig {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
pub struct UnixContext {
    // Для poll используем UnixPoll, который мы уже сделали потокобезопасным
    pub poll: UnixPoll,
    // Для shutdown используем AppShutdown, который мы уже сделали потокобезопасным
    pub shutdown: AppShutdown,
    // Для логов используем LogBufferStack, который мы уже сделали потокобезопасным
    pub log_buffer: LogBufferStack,
    // Новая потокобезопасная структура для управления перезагрузкой конфигурации
    pub reload_config: ReloadConfig,
}

impl UnixContext {
    pub fn new(poll_timeout: i32) -> Self {
        // Создаем контейнер для дескрипторов, который будет опрашиваться через poll
        Self {
            poll: UnixPoll::new(poll_timeout),
            shutdown: AppShutdown::default(),
            log_buffer: LogBufferStack::new(),
            reload_config: ReloadConfig::new(),
        }
    }

    /// Проверяет, нужно ли перезагрузить конфигурацию, и сбрасывает флаг
    pub fn check_and_reset_reload(&self) -> bool {
        self.reload_config.check_and_reset()
    }

    /// Устанавливает флаг необходимости перезагрузки конфигурации
    pub fn set_reload_needed(&self) {
        self.reload_config.set_reload_needed();
    }
}

impl AppContext for UnixContext {
}
