use std::fmt;

#[derive(Debug)]
pub enum UnixError {
    StdIoError(std::io::Error),
    NixErrorno(nix::errno::Errno),
    // PollEventNotHandle,
    // FdReadOnly,
    // FdNotFound,
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
    fn from(e: nix::errno::Errno) -> Self {
        UnixError::NixErrorno(e)
    }
}


