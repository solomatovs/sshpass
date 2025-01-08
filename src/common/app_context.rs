use std::cell::RefCell;

use crate::common::app_shutdown::AppShutdown;

#[derive(Debug)]
pub struct AppContext {
    pub shutdown: AppShutdown,
}

impl Default for AppContext {
    fn default() -> Self {
        Self {
            shutdown: AppShutdown::new(),
        }
    }
}