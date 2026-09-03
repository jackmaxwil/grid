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
    /// plan failed validation
    InvalidPlan = -32006,
    /// unknown plan id
    UnknownPlan = -32007,
    /// unknown trace id
    UnknownTrace = -32008,
}

impl WireError {
    /// The JSON-RPC error code.
    #[must_use]
    pub const fn code(self) -> i32 {
        self as i32
    }

    /// The variant carrying `code`, or `None` for a code this build does not
    /// name.
    ///
    /// Conservatism: a ninth contract code, and every JSON-RPC code outside the
    /// server-error range, is `None` rather than a match arm that panics — a
    /// caller keeps the integer it was sent.
    #[must_use]
    pub const fn from_code(code: i32) -> Option<Self> {
        match code {
            -32001 => Some(Self::CorruptSegment),
            -32002 => Some(Self::NoSnapshot),
            -32003 => Some(Self::UnknownSnapshot),
            -32004 => Some(Self::SurveyFailed),
            -32005 => Some(Self::UnsupportedLang),
            -32006 => Some(Self::InvalidPlan),
            -32007 => Some(Self::UnknownPlan),
            -32008 => Some(Self::UnknownTrace),
            _ => None,
        }
    }

    /// The contract's own message for this code.
    ///
    /// Claim: C47 — this file is outside the generator's reach, so the drift a
    /// contract edit could leave here is caught by a test over
    /// `components.errors` rather than by an emitter.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::CorruptSegment => "segment failed validation on load; store integrity is suspect",
            Self::NoSnapshot => "no snapshot published yet; run grid.survey.run",
            Self::UnknownSnapshot => "unknown or expired snapshot",
            Self::SurveyFailed => "survey failed",
            Self::UnsupportedLang => "no language adapter for file",
            Self::InvalidPlan => "plan failed validation",
            Self::UnknownPlan => "unknown plan id",
            Self::UnknownTrace => "unknown trace id",
        }
    }
}
