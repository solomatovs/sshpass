use std::fmt;

#[derive(Debug)]
pub enum UnixError {
    StdIoError(std::io::Error),

    NixErrorno(),

    // ArgumentError(String),
    ExitCodeError(i32),
    // Ok,

    // ShutdownSendError,

    // ChildTerminatedBySignal,
}

impl fmt::Display for UnixError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "NixError")
    }
}

impl std::error::Error for UnixError {}

impl From<std::io::Error> for UnixError {
    fn from(error: std::io::Error) -> Self {
        UnixError::StdIoError(error)
    }
}

impl From<nix::errno::Errno> for UnixError {
    fn from(_: nix::errno::Errno) -> Self {
        UnixError::NixErrorno()
    }
}

impl From<i32> for UnixError {
    fn from(error: i32) -> Self {
        UnixError::ExitCodeError(error)
    }
}
