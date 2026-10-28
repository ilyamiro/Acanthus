#[derive(Debug, thiserror::Error)]
/// Errors of the I/O toolkit.
pub enum IoError {
    #[error("io error: {0}")]
    /// A system call or file operation failed.
    Io(#[from] std::io::Error),
    #[error("cannot register event source: {0}")]
    /// An event source could not be registered.
    Register(String),
    #[error("invalid json: {0}")]
    /// JSON could not be parsed or written.
    Json(#[from] serde_json::Error),
    #[error("ipc error: {0}")]
    /// An IPC request failed.
    Ipc(String),
}

impl<T> From<calloop::InsertError<T>> for IoError {
    fn from(e: calloop::InsertError<T>) -> Self {
        IoError::Register(e.error.to_string())
    }
}

impl From<rustix::io::Errno> for IoError {
    fn from(e: rustix::io::Errno) -> Self {
        IoError::Io(e.into())
    }
}
