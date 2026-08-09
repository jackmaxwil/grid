//! The method layer: one function per wire method, shared verbatim by the CLI
//! and the daemon.
//!
//! The CLI is not a second implementation of the daemon — both call these
//! functions, so `grid gate --json` and a `grid.gate.check` request over the
//! socket cannot drift.

use grid_wire::methods::{
    GateCheckParams, GateCheckResult, TrackLintParams, TrackLintResult, VersionResult,
};

use crate::provider::{self, FileDelta};
use crate::{standing, ENGINE_VERSION, WIRE_VERSION};

/// `grid.version` — versions plus the current snapshot, if any. There is no
/// snapshot store until Wave 4, so the field is honestly absent.
#[must_use]
pub fn version() -> VersionResult {
    VersionResult {
        wire: WIRE_VERSION.to_string(),
        engine: ENGINE_VERSION.to_string(),
        porcelain: Some(crate::porcelain::VERSION.to_string()),
        snapshot: None,
    }
}

/// `grid.track.lint` — pure validation, no survey, no snapshot. Pure of the
/// chart, that is: the repo's standing invariants are part of the rules a
/// promise is judged against, and reading them touches no snapshot.
///
/// # Errors
/// Returns a message when the standing invariant file exists but is invalid;
/// a broken rule file must not be mistaken for an absence of rules.
pub fn track_lint(
    params: &TrackLintParams,
    root: &std::path::Path,
) -> Result<TrackLintResult, String> {
    let track = standing::fold(&params.track, standing::load(root)?);
    Ok(grid_xte::lint::lint(&track))
}

/// `grid.gate.check` — the track against the delta at head.
///
/// # Errors
/// Propagates the provider's error (unknown snapshot, failed diff).
pub fn gate_check(
    params: &GateCheckParams,
    root: &std::path::Path,
) -> Result<GateCheckResult, provider::Error> {
    let standing = standing::load(root).map_err(provider::Error::Config)?;
    let track = standing::fold(&params.track, standing);
    grid_xte::check(&track, &params.head, &FileDelta::new(root))
}
