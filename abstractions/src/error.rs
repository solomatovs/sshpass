#[derive(Clone, Debug)]
pub enum UnixError {
    AllocationError(String),
    PTYOpenError(String),
    PTYCommandError(String),
    SignalFdError(String),
    StdInRegisterError(String),
    TimerFdError(String),
}

impl From<UnixError> for i32 {
    fn from(err: UnixError) -> i32 {
        match err {
            UnixError::AllocationError(_) => 1,
            UnixError::PTYOpenError(_) => 2,
            UnixError::PTYCommandError(_) => 3,
            UnixError::SignalFdError(_) => 4,
            UnixError::StdInRegisterError(_) => 5,
            UnixError::TimerFdError(_) => 6,
        }
    }
}

impl std::fmt::Display for UnixError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            UnixError::AllocationError(msg) => write!(f, "Allocation Error: {}", msg),
            UnixError::PTYOpenError(msg) => write!(f, "PTY Open Error: {}", msg),
            UnixError::PTYCommandError(msg) => write!(f, "PTY Command Error: {}", msg),
            UnixError::SignalFdError(msg) => write!(f, "SignalFd Error: {}", msg),
            UnixError::StdInRegisterError(msg) => write!(f, "StdIn Register Error: {}", msg),
            UnixError::TimerFdError(msg) => write!(f, "TimerFd Error: {}", msg),
        }
    }
}

// Реализация `Into<(i32, String)>`
impl From<UnixError> for (i32, String) {
    fn from(err: UnixError) -> (i32, String) {
        let message = err.to_string();
        (err.into(), message)
    }
}

impl std::error::Error for UnixError {}
