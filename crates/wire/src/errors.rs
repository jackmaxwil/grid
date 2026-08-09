//! Wire error codes (JSON-RPC server-error range -32000..=-32099).

/// The eight wire error codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum WireError {
    /// segment failed validation on load; store integrity is suspect
    CorruptSegment = -32001,
    /// no snapshot published yet; run grid.survey.run
    NoSnapshot = -32002,
    /// unknown or expired snapshot
    UnknownSnapshot = -32003,
    /// survey failed
    SurveyFailed = -32004,
    /// no language adapter for file
    UnsupportedLang = -32005,
    /// track failed validation
    InvalidTrack = -32006,
    /// unknown track id
    UnknownTrack = -32007,
    /// unknown wake id
    UnknownWake = -32008,
}

impl WireError {
    /// The JSON-RPC error code.
    #[must_use]
    pub const fn code(self) -> i32 {
        self as i32
    }
}
