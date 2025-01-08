use std::time::Instant;

#[derive(Debug)]
pub struct AppShutdown {
    is_stoped: bool,
    is_stop: bool,
    stop_time: Option<Instant>,
    stop_code: Option<i32>,
    stop_error: Option<String>,
}

impl AppShutdown {
    pub fn new() -> Self {
        Self {
            is_stoped: false,
            is_stop: false,
            stop_time: None,
            stop_code: None,
            stop_error: None,
        }
    }

    pub fn is_stop(&self) -> bool {
        self.is_stop
    }

    pub fn is_stoped(&self) -> bool {
        self.is_stoped
    }

    pub fn shutdown_starting(&mut self, stop_code: i32, error: Option<String>) {
        self.stop_time = Some(Instant::now());
        self.is_stop = true;
        self.is_stoped = false;
        self.stop_code = Some(stop_code);
        self.stop_error = error;
    }

    pub fn shutdown_complited(&mut self) {
        self.is_stop = false;
        self.is_stoped = true;
    }

    pub fn shutdown_cancel(&mut self) {
        self.is_stop = false;
        self.is_stoped = false;
        self.stop_time = None;
        self.stop_code = None;
        self.stop_error = None;
    }

    pub fn stop_code(&self) -> i32 {
        self.stop_code.unwrap_or(255)
    }
}
