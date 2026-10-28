use core::fmt;

/// Errors from parsing colours.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The string was not an RGB hex value.
    ParseRGB,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ParseRGB => "provided string was not RGB-like".fmt(f),
        }
    }
}

impl std::error::Error for Error {}
