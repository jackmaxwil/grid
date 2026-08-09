//! grid-cache — verification verdicts keyed by what the verdict depended on.
//!
//! The cache answers one question: *has this exact thing already been verified
//! in this exact environment?* "This exact thing" is a per-kind closure over
//! the chart, frozen into a key at record time:
//!
//! - `lint` sees one span, so the key is the datum alone.
//! - `typecheck` sees signatures, so the key is the datum plus its transitive
//!   **proven** out-edge closure.
//! - `test` executes, so the key is the datum plus its transitive full
//!   out-edge closure.
//!
//! Soundness is one-directional (claim C4). A closure that cannot be fully
//! established — an unknown datum, or a walk that runs past its fuel — is
//! *rejected at record time* rather than stored under a partial key, because a
//! partial key is exactly how a stale verdict survives a change it depended
//! on. A hit therefore implies an established closure. A miss implies nothing
//! at all except "run it".

use std::collections::{BTreeSet, HashMap};
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use grid_chart::{DatumId, Ordinal, Segment};
use grid_wire::Zoc;

/// Walk budget for one key. A closure wider than this is rejected, not
/// truncated: truncation is indistinguishable from a smaller closure, and that
/// is the one confusion C4 forbids.
pub const FUEL: u32 = 100_000;

/// What a verdict was produced by. The wire has two generated enums for this
/// (check-side and record-side); both map here so a key is kind-comparable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    /// One span, no closure.
    Lint,
    /// Transitive closure over proven edges.
    Typecheck,
    /// Transitive closure over every edge.
    Test,
}

/// Which out-edges a kind's closure traverses. Three cases, named, because
/// "no walk" and "walk everything" are not two shades of one option.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Follows {
    /// The datum is the whole closure.
    Nothing,
    /// Only edges of exactly this evidence grade.
    Graded(Zoc),
    /// Every edge, whatever its grade.
    Everything,
}

impl Kind {
    const fn tag(self) -> &'static str {
        match self {
            Self::Lint => "lint",
            Self::Typecheck => "typecheck",
            Self::Test => "test",
        }
    }

    /// Which edges this kind's closure follows.
    const fn follows(self) -> Follows {
        match self {
            Self::Lint => Follows::Nothing,
            // A signature closure is only as good as the edges it trusts, so
            // typecheck follows proven edges and stops at anything weaker.
            Self::Typecheck => Follows::Graded(Zoc::Proven),
            Self::Test => Follows::Everything,
        }
    }
}

impl From<grid_wire::CacheCheckRequestKind> for Kind {
    fn from(k: grid_wire::CacheCheckRequestKind) -> Self {
        use grid_wire::CacheCheckRequestKind as K;
        match k {
            K::Lint => Self::Lint,
            K::Typecheck => Self::Typecheck,
            K::Test => Self::Test,
        }
    }
}

impl From<grid_wire::methods::CacheRecordKind> for Kind {
    fn from(k: grid_wire::methods::CacheRecordKind) -> Self {
        use grid_wire::methods::CacheRecordKind as K;
        match k {
            K::Lint => Self::Lint,
            K::Typecheck => Self::Typecheck,
            K::Test => Self::Test,
        }
    }
}

/// A recorded verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// The kind's checker accepted the definition.
    Pass,
    /// It rejected it. A recorded failure is as cacheable as a success — the
    /// input that produced it is pinned by the same key.
    Fail,
}

impl Verdict {
    const fn tag(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Fail => "fail",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        match s {
            "pass" => Some(Self::Pass),
            "fail" => Some(Self::Fail),
            _ => None,
        }
    }
}

/// Why a key could not be frozen. Every variant is a refusal to store, which
/// is the record-side face of one-directional soundness.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reject {
    /// The datum is not in the snapshot, so there is no closure to establish.
    UnknownDatum,
    /// The closure exceeded [`FUEL`]. Truncating it would forge a key.
    FuelExhausted,
}

impl std::fmt::Display for Reject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownDatum => f.write_str("closure unresolved: datum is not in this snapshot"),
            Self::FuelExhausted => write!(f, "closure exceeded fuel ({FUEL})"),
        }
    }
}

impl std::error::Error for Reject {}

/// A frozen key: the content identity of (kind, environment, closure).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Key([u8; 32]);

impl Key {
    /// Lowercase hex, the on-disk and log-facing form.
    #[must_use]
    pub fn to_hex(self) -> String {
        let mut s = String::with_capacity(64);
        for b in self.0 {
            let _ = write!(s, "{b:02x}");
        }
        s
    }

    fn parse(s: &str) -> Option<Self> {
        if s.len() != 64 {
            return None;
        }
        let mut out = [0u8; 32];
        for (i, byte) in out.iter_mut().enumerate() {
            let pair = s.get(i * 2..i * 2 + 2)?;
            if pair.bytes().any(|c| c.is_ascii_uppercase()) {
                return None;
            }
            *byte = u8::from_str_radix(pair, 16).ok()?;
        }
        Some(Self(out))
    }
}

/// The chart, as the closure walk needs to see it: a set of segments that
/// together hold the live graph.
///
/// Taking segments rather than an epoch keeps this crate off the mmap crate:
/// the walk needs bytes that parse, not a file that was mapped.
pub struct Chart<'a, B: AsRef<[u8]>> {
    segments: &'a [Segment<B>],
}

impl<'a, B: AsRef<[u8]>> Chart<'a, B> {
    /// View the live segments of one epoch.
    #[must_use]
    pub const fn new(segments: &'a [Segment<B>]) -> Self {
        Self { segments }
    }

    /// Every (segment, ordinal) a datum is interned at. A datum can appear in
    /// more than one segment; its closure is the union, never one segment's
    /// view of it.
    fn locate(&self, datum: DatumId) -> impl Iterator<Item = (&Segment<B>, Ordinal)> + '_ {
        self.segments
            .iter()
            .filter_map(move |seg| seg.ordinal(&datum).map(|ord| (seg, ord)))
    }

    /// The transitive closure of `datum` under `kind`, as a sorted set of
    /// datum digests including `datum` itself.
    ///
    /// # Errors
    /// [`Reject::UnknownDatum`] if the datum is in no segment;
    /// [`Reject::FuelExhausted`] if the walk runs past [`FUEL`].
    pub fn closure(&self, datum: DatumId, kind: Kind) -> Result<BTreeSet<DatumId>, Reject> {
        if self.locate(datum).next().is_none() {
            return Err(Reject::UnknownDatum);
        }
        let mut seen = BTreeSet::new();
        seen.insert(datum);
        let follows = kind.follows();
        if follows == Follows::Nothing {
            return Ok(seen);
        }
        let mut frontier = vec![datum];
        let mut fuel = FUEL;
        while let Some(current) = frontier.pop() {
            for (seg, ord) in self.locate(current) {
                for edge in seg.out_edges(ord) {
                    if matches!(follows, Follows::Graded(want) if edge.zoc != want) {
                        continue;
                    }
                    fuel = fuel.checked_sub(1).ok_or(Reject::FuelExhausted)?;
                    // An edge names an ordinal in its own segment, and a
                    // parsed segment answers every ordinal in bounds (C12), so
                    // the `None` arm is unreachable rather than tolerated.
                    let Some(next) = seg.datum(edge.other) else {
                        return Err(Reject::UnknownDatum);
                    };
                    if seen.insert(next) {
                        frontier.push(next);
                    }
                }
            }
        }
        Ok(seen)
    }

    /// Freeze the key for `datum` under `kind` in environment `env_hash`.
    ///
    /// # Errors
    /// Whatever [`Chart::closure`] rejects. A key is never approximated.
    pub fn key(&self, datum: DatumId, kind: Kind, env_hash: &str) -> Result<Key, Reject> {
        let closure = self.closure(datum, kind)?;
        let mut h = blake3::Hasher::new();
        // Domain separation, then length-prefixed fields: two different
        // (kind, env, closure) triples cannot share a preimage.
        h.update(b"grid-cache-key/v1\n");
        for field in [kind.tag().as_bytes(), env_hash.as_bytes()] {
            h.update(&(field.len() as u64).to_le_bytes());
            h.update(field);
        }
        h.update(&(closure.len() as u64).to_le_bytes());
        for member in &closure {
            h.update(&member.0);
        }
        Ok(Key(*h.finalize().as_bytes()))
    }
}

/// A stored verdict and its provenance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// What the checker said.
    pub verdict: Verdict,
    /// RFC 3339 UTC, second resolution — the wire's `recorded_at`.
    pub recorded_at: String,
    /// The trace this verdict came from, when a wake produced it.
    pub wake_id: Option<String>,
}

/// The verdict store: keys to entries, backed by an append-only log.
///
/// Append-only is the whole crash story. A record is one `write` of one line
/// to a file opened for append and then fsynced; a torn tail is a line that
/// does not parse, and a line that does not parse is dropped on load. There is
/// no state a partial write can corrupt, because there is no state — only
/// lines, and the last line for a key wins.
#[derive(Debug)]
pub struct Cache {
    path: PathBuf,
    entries: HashMap<Key, Entry>,
}

impl Cache {
    /// Open (or create) the cache log at `path`.
    ///
    /// # Errors
    /// I/O errors other than a missing log, which is the empty cache.
    pub fn open(path: impl AsRef<Path>) -> std::io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut entries = HashMap::new();
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                for line in text.lines() {
                    if let Some((key, entry)) = parse_line(line) {
                        entries.insert(key, entry);
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        Ok(Self { path, entries })
    }

    /// Look a key up. A hit is a verdict whose closure was fully established
    /// when it was recorded and whose closure hashes the same now.
    #[must_use]
    pub fn get(&self, key: Key) -> Option<&Entry> {
        self.entries.get(&key)
    }

    /// How many distinct keys are stored.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the cache holds nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Store a verdict under an already-frozen key, durably.
    ///
    /// # Errors
    /// I/O errors from the append or the fsync. A verdict that was not synced
    /// is not reported as recorded.
    pub fn put(&mut self, key: Key, entry: Entry) -> std::io::Result<()> {
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        writeln!(
            file,
            "{} {} {} {}",
            key.to_hex(),
            entry.verdict.tag(),
            entry.recorded_at,
            entry.wake_id.as_deref().unwrap_or("-"),
        )?;
        file.sync_data()?;
        self.entries.insert(key, entry);
        Ok(())
    }
}

fn parse_line(line: &str) -> Option<(Key, Entry)> {
    let mut fields = line.split(' ');
    let key = Key::parse(fields.next()?)?;
    let verdict = Verdict::parse(fields.next()?)?;
    let recorded_at = fields.next()?.to_string();
    let wake_id = match fields.next()? {
        "-" => None,
        id => Some(id.to_string()),
    };
    // A trailing field means a line from a newer format, not a line to guess at.
    if fields.next().is_some() {
        return None;
    }
    Some((
        key,
        Entry {
            verdict,
            recorded_at,
            wake_id,
        },
    ))
}

/// `recorded_at` for a verdict recorded now: RFC 3339 UTC, second resolution.
///
/// Written out by hand because a timestamp is not worth a dependency in the
/// TCB, and second resolution is all the wire promises.
#[must_use]
pub fn now_rfc3339() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let (days, rem) = (secs / 86_400, secs % 86_400);
    let (h, mi, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let (y, mo, d) = civil_from_days(days);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

/// Days since the Unix epoch to a civil date (Howard Hinnant's `civil_from_days`).
fn civil_from_days(days: u64) -> (u64, u64, u64) {
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z % 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}
