//! What can go wrong opening or running an output.

/// Everything that can go wrong opening or running the output path.
#[derive(Debug)]
pub enum PlayError {
    Io(std::io::Error),
    Decode(String),
    /// No default output device is available.
    NoDevice,
    /// The device offers no output configuration canon can use for the source's channel count.
    UnsupportedFormat(String),
    /// cpal failed to build or start the stream.
    Device(String),
}

impl std::fmt::Display for PlayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlayError::Io(e) => write!(f, "io: {e}"),
            PlayError::Decode(e) => write!(f, "decode: {e}"),
            PlayError::NoDevice => write!(f, "no default audio output device"),
            PlayError::UnsupportedFormat(e) => write!(f, "unsupported output format: {e}"),
            PlayError::Device(e) => write!(f, "audio device: {e}"),
        }
    }
}

impl std::error::Error for PlayError {}
