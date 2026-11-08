use core::fmt;

/// Errors reported during configuration, port I/O and format loading.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Error {
    /// Invalid configuration or unsupported device selection.
    Config(&'static str),
    /// A port access attempted to move machine time backwards.
    BackwardsTime,
    /// The bounded command queue is full; no hardware state was changed.
    QueueFull,
    /// DMA completion events must be polled before advancing further.
    IrqQueueFull,
    /// A bounded synthesis trace overflowed and cannot be exported as complete.
    CaptureOverflow,
    /// A music file is malformed or uses an unsupported encoding.
    Format(&'static str),
    /// A timestamp, sample count or file size cannot be represented.
    TooLarge,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config(reason) | Self::Format(reason) => f.write_str(reason),
            Self::BackwardsTime => f.write_str("machine time cannot move backwards"),
            Self::QueueFull => {
                f.write_str("sound command queue is full; render audio before retrying")
            }
            Self::IrqQueueFull => {
                f.write_str("DMA completion queue is full; poll completion events before retrying")
            }
            Self::CaptureOverflow => f.write_str(
                "synthesis capture overflowed; drain it more often or increase capacity",
            ),
            Self::TooLarge => f.write_str("time or file size exceeds the supported range"),
        }
    }
}

impl std::error::Error for Error {}

/// Result type used by this crate.
pub type Result<T> = core::result::Result<T, Error>;
