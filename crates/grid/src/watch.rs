//! Watch mode: drift maintained as signed deltas, not recomputed as a list.
//!
//! The DBSP steal (`docs/research/ANALYSIS.md` §3) — the discipline
//! hand-derived, never the framework. Drift-suspects is a *bilinear* query:
//! one equi-join of inbound edges against the changed-interface set, plus an
//! antijoin against the changed-dependent set. DBSP's cost model then says
//! exactly what live maintenance must hold as state, and that nothing else is
//! needed:
//!
//! - the **edge table** — the base snapshot's resident segments already are
//!   the integral of every edge delta;
//! - the **changed-sig set** since the session's base — small, in memory,
//!   and the only thing that grows.
//!
//! Per event the three-term incremental-join form falls out: only the
//! definitions whose changed-ness *moved* are probed (a reverse-adjacency
//! probe each), and only the dependents whose touched-ness moved are
//! re-decided. A definition that was already changed and still is contributes
//! nothing to the delta, which is the whole point.
//!
//! Output is signed: `+1` a suspect appeared, `-1` it was retracted. A
//! consumer holds a running sum, and the sum after any prefix of events
//! equals a fresh `grid drift` — which is checkable by summation, and is
//! what the four algebra tests check.
//!
//! Closures are explicitly **not** maintained here. That is the recursive
//! (nested-stream) case: correct to incrementalise, expensive to, and the
//! budgeted invalidate-and-rewalk in `grid cache` stays the right call.
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use grid_chart::Segment;
use grid_wire::{DriftDelta, DriftSuspect, Grade, Plan, SnapshotId, SnapshotOrRev};

use crate::chart_provider::{self, ChartDelta};
use crate::drift;

/// How many moved definitions an event may carry before the session stops
/// probing and simply recomputes.
///
/// Past this, the incremental form is doing more work than the whole query:
/// a burst that touches half the tree is a cold start wearing a costume. The
/// fallback is correct by the cold-start property — feeding everything as one
/// event equals a survey plus a drift — and it says so on stderr.
pub const BURST: usize = 256;

/// The key a suspect row is held under: dependent, changed definition, and
/// the relation between them.
type Key = (String, String, u8);

/// A live drift session over one base snapshot.
///
/// Holds exactly the two integrals the cost model calls for, plus the running
/// suspect set the signs are computed against.
pub struct Session {
    root: std::path::PathBuf,
    base: SnapshotOrRev,
    floor: Grade,
    plan: Option<Plan>,
    /// The per-file survey cache. This is what makes a file event cost one
    /// file's parse: everything else comes back from content hashes.
    cache: grid_survey::Cache,
    /// The discovered file set, held across events. Discovery — `git
    /// ls-files` plus a read of every tracked file — does not change when a
    /// file's *contents* do, so an event refreshes the entries it names and
    /// keeps the rest. A path the list does not hold means discovery itself
    /// moved, and the whole list is rebuilt.
    sources: Option<(Vec<grid_survey::SourceFile>, grid_survey::files::CrateRoots)>,
    /// Definitions whose interface has moved since base — the second
    /// integral, and the only state that grows with the session.
    changed: BTreeMap<String, drift::Changed>,
    /// Definitions whose datum has moved since base, by callsign: the
    /// antijoin set.
    touched: BTreeSet<String>,
    /// The suspect set as the consumer's running sum should have it.
    live: BTreeMap<Key, DriftSuspect>,
    /// File events declined because a frontend could not parse them.
    declined: u64,
}

/// What one event produced.
pub struct Emitted {
    /// The signed changes, in key order so a session is reproducible.
    pub deltas: Vec<DriftDelta>,
    /// True when the event was large enough that the session recomputed
    /// instead of probing. Correct either way; slower, and worth saying.
    pub recomputed: bool,
    /// Paths the frontend rejected. No deltas are emitted for an event that
    /// carried one: an unparseable file is a file grid cannot see, and
    /// treating it as empty would retract every suspect it holds.
    pub declined: Vec<String>,
}

impl Session {
    /// Open a session measured from `base`.
    ///
    /// # Errors
    /// [`chart_provider::Error`] when the base snapshot does not resolve.
    pub fn open(
        root: &std::path::Path,
        base: SnapshotOrRev,
        floor: Grade,
        plan: Option<Plan>,
    ) -> Result<Self, chart_provider::Error> {
        // Resolve now, so a session cannot be opened against a base that is
        // not there and discover it one keystroke later.
        ChartDelta::open(root)?.epoch_of(&base)?;
        Ok(Self {
            root: root.to_path_buf(),
            base,
            floor,
            plan,
            cache: grid_survey::Cache::new(),
            sources: None,
            changed: BTreeMap::new(),
            touched: BTreeSet::new(),
            live: BTreeMap::new(),
            declined: 0,
        })
    }

    /// The suspect set as it stands — the running sum the emitted signs are
    /// computed against.
    ///
    /// Test-only: nothing in the stream needs it, because the stream *is* the
    /// answer. It exists so the algebra tests can check that the session's
    /// own state agrees with the sum a consumer would hold, which is the one
    /// thing a stream cannot show about itself.
    #[cfg(test)]
    #[must_use]
    pub fn suspects(&self) -> Vec<DriftSuspect> {
        self.live.values().cloned().collect()
    }

    /// How many file events were declined because a frontend rejected them.
    #[must_use]
    pub const fn declined(&self) -> u64 {
        self.declined
    }

    /// Fold one file event into the session.
    ///
    /// `paths` are the repo-relative files that changed. They are used for
    /// two things and nothing else: to refuse an event grid cannot read, and
    /// to keep the re-survey honest about what it is being told.
    ///
    /// # Errors
    /// [`drift::Error`] when the base does not resolve, the tree cannot be
    /// surveyed, or an inbound walk runs past its fuel.
    pub fn event(&mut self, paths: &[String]) -> Result<Emitted, drift::Error> {
        // Conservatism first: a file the frontend cannot parse is a file grid
        // cannot see. The survey would treat it as defining nothing, which
        // would retract every suspect it holds — an all-clear invented out of
        // a syntax error. Refuse the event instead, and count it.
        let declined = self.unparseable(paths);
        if !declined.is_empty() {
            self.declined = self.declined.saturating_add(declined.len() as u64);
            return Ok(Emitted {
                deltas: Vec::new(),
                recomputed: false,
                declined,
            });
        }

        let chart = ChartDelta::open(&self.root).map_err(drift::Error::Chart)?;
        let base_epoch = chart.epoch_of(&self.base).map_err(drift::Error::Chart)?;
        // The changed files' read and parse, and nothing else: discovery is
        // held across events, and the per-file cache answers for every file
        // whose bytes did not move.
        let survey = self.resurvey(paths)?;
        let head = Segment::parse(survey.image).map_err(|e| {
            drift::Error::Chart(chart_provider::Error::Survey(
                grid_survey::SurveyError::Chart(e),
            ))
        })?;
        let head = [head];

        let diff = grid_chart::diff(
            SnapshotId(base_epoch.snapshot_id().as_str().to_owned()),
            base_epoch.segments(),
            // The worktree is not a published snapshot and must not claim to
            // be one; the id is only carried through the delta, never stored.
            SnapshotId("snap:worktree".to_owned()),
            &head,
        );
        let touched_now: BTreeSet<String> = drift::touched(&diff)
            .into_iter()
            .map(str::to_owned)
            .collect();
        let changed_now: BTreeMap<String, drift::Changed> =
            drift::changed_set_of(&diff, base_epoch.segments(), &head)
                .into_iter()
                .map(|c| (c.callsign.clone(), c))
                .collect();

        // The three-term form, as sets. `moved` is the changed-interface
        // delta: the definitions to re-probe. The dependent side splits by
        // direction, because the two are not symmetric in cost:
        //
        // - a dependent that *became* touched can only lose rows, and the
        //   rows it is filed under are already in hand;
        // - a dependent that stopped being touched (an edit reverted) can
        //   gain rows from any changed definition that reaches it, which
        //   without a forward index means asking all of them. That is the
        //   recompute path, taken deliberately and said out loud rather than
        //   answered wrong.
        let moved: Vec<String> = symmetric_difference(&self.changed, &changed_now);
        let newly_touched: Vec<&String> = touched_now.difference(&self.touched).collect();
        let untouched_again = self.touched.difference(&touched_now).next().is_some();

        let recomputed = untouched_again || moved.len().saturating_add(newly_touched.len()) > BURST;
        let observed = if drift::rank(self.floor) >= drift::rank(Grade::Observed) {
            drift::observed_inbound(&self.root, base_epoch.segments())?
        } else {
            HashMap::new()
        };

        // The definitions to re-probe. Incrementally: the ones whose
        // changed-ness moved, plus the ones a flipped dependent points at.
        // On a burst: all of them, which is the cold-start path.
        let probe: Vec<&drift::Changed> =
            self.probe_set(recomputed, &moved, &newly_touched, &changed_now);

        let touched_refs: HashSet<&str> = touched_now.iter().map(String::as_str).collect();
        let mut rebuilt: BTreeMap<Key, DriftSuspect> = BTreeMap::new();
        let mut budget_rejected = 0u32;
        for change in &probe {
            let Some(found) = drift::inbound_of(
                base_epoch.segments(),
                change,
                self.floor,
                &observed,
                grid_cache::FUEL,
            ) else {
                budget_rejected = budget_rejected.saturating_add(1);
                continue;
            };
            drift::fold_rows(
                &chart,
                change,
                found,
                &touched_refs,
                self.plan.as_ref(),
                &mut rebuilt,
            );
        }
        if budget_rejected > 0 {
            return Err(drift::Error::BudgetRejected(budget_rejected));
        }

        let deltas = self.reconcile(rebuilt, &probe, recomputed);
        self.changed = changed_now;
        self.touched = touched_now;
        Ok(Emitted {
            deltas,
            recomputed,
            declined: Vec::new(),
        })
    }

    /// Turn a rebuilt slice of the suspect set into signed deltas, and adopt
    /// it as the new running sum.
    ///
    /// Only keys this event could have moved are considered: what the probe
    /// rebuilt, plus what the session held for a definition it re-probed.
    /// Everything else is untouched by construction and contributes no delta,
    /// which is what makes the stream incremental rather than a list diff
    /// wearing a sign.
    fn reconcile(
        &mut self,
        rebuilt: BTreeMap<Key, DriftSuspect>,
        probe: &[&drift::Changed],
        recomputed: bool,
    ) -> Vec<DriftDelta> {
        let probed: BTreeSet<&str> = probe.iter().map(|c| c.callsign.as_str()).collect();
        let mut deltas = Vec::new();
        for (key, suspect) in &rebuilt {
            if !self.live.contains_key(key) {
                deltas.push(DriftDelta {
                    suspect: suspect.clone(),
                    weight: 1,
                });
            }
        }
        let dropped: Vec<Key> = self
            .live
            .keys()
            .filter(|key| {
                (recomputed || probed.contains(key.1.as_str())) && !rebuilt.contains_key(*key)
            })
            .cloned()
            .collect();
        for key in dropped {
            if let Some(suspect) = self.live.remove(&key) {
                deltas.push(DriftDelta {
                    suspect,
                    weight: -1,
                });
            }
        }
        for (key, suspect) in rebuilt {
            self.live.insert(key, suspect);
        }
        deltas
    }

    /// Re-survey the tree, paying for the named files and nothing else.
    ///
    /// Falls back to full discovery when the list does not hold a named path
    /// — a new or deleted file is a *discovery* change, and guessing would
    /// leave the answer describing a tree that is not there.
    ///
    /// ponytail: the event still costs a whole-tree phase 2/3 — index every
    /// definition, resolve every reference — because an edge is a fact about
    /// two files and grid never caches one (the incremental-equals-full rule,
    /// claim C13). Measured at ~46ms on this repo against the plan's 5ms
    /// budget. The upgrade is a resolver that can invalidate by reference
    /// rather than rebuild, which is a survey-architecture change, not a
    /// watch-mode one.
    fn resurvey(&mut self, paths: &[String]) -> Result<grid_survey::Survey, chart_provider::Error> {
        let fresh = match &mut self.sources {
            Some((sources, _)) => paths
                .iter()
                .all(|path| grid_survey::files::reread(&self.root, sources, path)),
            None => false,
        };
        if !fresh {
            self.sources = Some(grid_survey::files::sources(&self.root)?);
        }
        let Some((sources, roots)) = &self.sources else {
            return Err(chart_provider::Error::NoSnapshot);
        };
        Ok(grid_survey::run_sources(sources, roots, &mut self.cache)?)
    }

    /// The definitions this event must re-probe.
    ///
    /// Incrementally: the ones whose changed-ness moved, plus the ones a
    /// dependent that just became touched is filed under — those rows have to
    /// be re-decided, and the rebuild will not produce them again. On a
    /// burst: all of them, which is the cold-start path.
    fn probe_set<'a>(
        &self,
        recomputed: bool,
        moved: &'a [String],
        newly_touched: &[&String],
        changed_now: &'a BTreeMap<String, drift::Changed>,
    ) -> Vec<&'a drift::Changed> {
        if recomputed {
            return changed_now.values().collect();
        }
        let mut names: BTreeSet<&String> = moved.iter().collect();
        let gone: BTreeSet<&str> = newly_touched.iter().map(|s| s.as_str()).collect();
        for (dependent, changed, _) in self.live.keys() {
            if gone.contains(dependent.as_str()) {
                names.insert(changed);
            }
        }
        names
            .into_iter()
            .filter_map(|n| changed_now.get(n))
            .collect()
    }

    /// The paths a frontend cannot parse. A path with no frontend at all is
    /// not a refusal — grid never claimed to read it.
    fn unparseable(&self, paths: &[String]) -> Vec<String> {
        paths
            .iter()
            .filter(|path| {
                let Some(lang) = grid_survey::files::lang_of(path) else {
                    return false;
                };
                let Ok(source) = std::fs::read_to_string(self.root.join(path)) else {
                    // A deleted file is an event grid can act on; only a file
                    // that is there and does not parse is a refusal.
                    return false;
                };
                grid_survey::definitions(lang, &source).is_err()
            })
            .cloned()
            .collect()
    }
}

/// Keys present in exactly one of two maps, or present in both with a moved
/// interface. Everything else contributed no change and is not probed.
fn symmetric_difference(
    before: &BTreeMap<String, drift::Changed>,
    after: &BTreeMap<String, drift::Changed>,
) -> Vec<String> {
    let mut out: BTreeSet<String> = BTreeSet::new();
    for (name, change) in after {
        match before.get(name) {
            Some(prior) if prior.datum == change.datum => {}
            _ => {
                out.insert(name.clone());
            }
        }
    }
    for name in before.keys() {
        if !after.contains_key(name) {
            out.insert(name.clone());
        }
    }
    out.into_iter().collect()
}

#[cfg(test)]
mod tests {
    //! The four algebra tests DBSP's theory asks for, over a real repository.
    //!
    //! They are the whole warrant for a signed stream: a differential says
    //! one step is right, but state only corrupts *across* steps, so the
    //! sequence-integration property is the one that finds a session that
    //! quietly drifts from the truth. The group-inverse and cold-start
    //! properties pin the two things a signed stream promises and a list
    //! never can.
    use super::{Emitted, Session};
    use grid_wire::{DriftDelta, DriftSuspect, GitRev, Grade, SnapshotOrRev};
    use std::path::Path;
    use std::process::Command;

    fn git(dir: &Path, args: &[&str]) {
        let ok = Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_AUTHOR_NAME", "grid")
            .env("GIT_AUTHOR_EMAIL", "grid@example.invalid")
            .env("GIT_COMMITTER_NAME", "grid")
            .env("GIT_COMMITTER_EMAIL", "grid@example.invalid")
            .output()
            .expect("run git")
            .status
            .success();
        assert!(ok, "git {args:?} failed");
    }

    fn write(dir: &Path, path: &str, text: &str) {
        let full = dir.join(path);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).expect("mkdir");
        }
        std::fs::write(full, text).expect("write");
    }

    const BEFORE: &str = "def window(retries=3):\n    return retries\n";
    const AFTER: &str = "def window(retries=5):\n    return retries\n";

    /// A repo whose `pkg.use` module calls `pkg.api.window`, surveyed and
    /// published so a session has a base to measure from.
    fn repo() -> (tempfile::TempDir, SnapshotOrRev) {
        let dir = tempfile::tempdir().expect("tempdir");
        git(dir.path(), &["init", "-q", "-b", "main"]);
        write(dir.path(), "pkg/__init__.py", "");
        write(dir.path(), "pkg/api.py", BEFORE);
        write(
            dir.path(),
            "pkg/use.py",
            "from pkg.api import window\n\n\ndef go():\n    return window()\n",
        );
        git(dir.path(), &["add", "-A"]);
        git(dir.path(), &["commit", "-qm", "base"]);
        let mut chart = crate::rpc::Chart::new(dir.path());
        let (snapshot, _) = chart
            .survey_run(&grid_wire::methods::SurveyRunParams {
                git_rev: None,
                worktree: None,
            })
            .expect("publish a base");
        // Pin the base by *id*. A rev is not enough: surveying the worktree
        // publishes under the same rev, so `HEAD` would move underneath both
        // the session and the reference it is checked against.
        let base = SnapshotOrRev::Snapshot {
            snapshot: snapshot.snapshot_id,
        };
        (dir, base)
    }

    fn session(dir: &Path, base: &SnapshotOrRev) -> Session {
        Session::open(dir, base.clone(), Grade::Observed, None).expect("open a session")
    }

    /// The suspect set a fresh `grid drift` reports for the same question —
    /// the reference every property here is measured against.
    fn full(dir: &Path, base: &SnapshotOrRev) -> Vec<String> {
        // Drift's literal-`HEAD` rule surveys the worktree, which is exactly
        // what the session is measuring.
        let head = SnapshotOrRev::GitRev {
            git_rev: GitRev("HEAD".into()),
        };
        let report = crate::rpc::drift(
            &grid_wire::methods::DriftParams {
                base: base.clone(),
                head,
                floor: Some(Grade::Observed),
                plan: None,
                tests: None,
                compat: None,
            },
            dir,
        )
        .expect("drift answers");
        keys(&report.suspects)
    }

    fn keys(suspects: &[DriftSuspect]) -> Vec<String> {
        let mut out: Vec<String> = suspects
            .iter()
            .map(|s| {
                format!(
                    "{}<-{}",
                    s.dependent.callsign.as_ref().map_or("?", |c| c.0.as_str()),
                    s.changed.callsign.as_ref().map_or("?", |c| c.0.as_str())
                )
            })
            .collect();
        out.sort();
        out
    }

    /// The running sum a consumer would hold after applying `deltas`.
    fn integrate(sum: &mut Vec<String>, emitted: &Emitted) {
        for DriftDelta { suspect, weight } in &emitted.deltas {
            let key = keys(std::slice::from_ref(suspect)).remove(0);
            if *weight > 0 {
                sum.push(key);
            } else {
                sum.retain(|k| k != &key);
            }
        }
        sum.sort();
    }

    fn event(session: &mut Session, paths: &[&str]) -> Emitted {
        let owned: Vec<String> = paths.iter().map(|p| (*p).to_string()).collect();
        session.event(&owned).expect("the session answers")
    }

    /// Differential: the incremental answer after an edit equals the full
    /// recompute. `Q^Δ = D∘Q∘I`, checked pointwise.
    #[test]
    fn incremental_after_an_edit_equals_a_full_recompute() {
        let (dir, base) = repo();
        let mut session = session(dir.path(), &base);
        write(dir.path(), "pkg/api.py", AFTER);
        let emitted = event(&mut session, &["pkg/api.py"]);
        let mut sum = Vec::new();
        integrate(&mut sum, &emitted);
        assert_eq!(sum, full(dir.path(), &base));
        assert!(!sum.is_empty(), "a vacuous agreement proves nothing");
        assert_eq!(keys(&session.suspects()), sum, "the session agrees too");
    }

    /// Sequence-integration: after *every prefix* of a multi-edit sequence,
    /// the summed deltas equal a fresh drift. State only corrupts across
    /// steps, so a single-edit test cannot catch what this does.
    #[test]
    fn every_prefix_of_a_sequence_integrates_to_the_full_answer() {
        let (dir, base) = repo();
        let mut session = session(dir.path(), &base);
        let steps: [(&str, &str); 4] = [
            ("pkg/api.py", AFTER),
            (
                "pkg/api.py",
                "def window(retries=5, extra=1):\n    return retries\n",
            ),
            (
                "pkg/use.py",
                "from pkg.api import window\n\n\ndef go():\n    return window(1)\n",
            ),
            ("pkg/api.py", BEFORE),
        ];
        let mut sum = Vec::new();
        for (path, text) in steps {
            write(dir.path(), path, text);
            let emitted = event(&mut session, &[path]);
            integrate(&mut sum, &emitted);
            assert_eq!(
                sum,
                full(dir.path(), &base),
                "the sum after this prefix must be the whole answer"
            );
        }
    }

    /// The group inverse: an edit and its revert net every weight to zero,
    /// and a suspect resolved by updating the dependent is retracted exactly
    /// once.
    #[test]
    fn an_edit_and_its_revert_net_to_zero() {
        let (dir, base) = repo();
        let mut session = session(dir.path(), &base);
        write(dir.path(), "pkg/api.py", AFTER);
        let out = event(&mut session, &["pkg/api.py"]);
        let appeared: i64 = out.deltas.iter().map(|d| d.weight).sum();
        assert!(appeared > 0, "the edit must produce a suspect");

        write(dir.path(), "pkg/api.py", BEFORE);
        let back = event(&mut session, &["pkg/api.py"]);
        let net: i64 = out
            .deltas
            .iter()
            .chain(&back.deltas)
            .map(|d| d.weight)
            .sum();
        assert_eq!(net, 0, "a revert nets the stream to zero");
        assert!(session.suspects().is_empty());
    }

    /// A suspect resolved by updating the *dependent* is retracted exactly
    /// once: one `-1`, not a re-emitted list.
    #[test]
    fn updating_the_dependent_retracts_the_suspect_exactly_once() {
        let (dir, base) = repo();
        let mut session = session(dir.path(), &base);
        write(dir.path(), "pkg/api.py", AFTER);
        let out = event(&mut session, &["pkg/api.py"]);
        assert_eq!(out.deltas.len(), 1, "one suspect appeared");
        assert_eq!(out.deltas[0].weight, 1);

        write(
            dir.path(),
            "pkg/use.py",
            "from pkg.api import window\n\n\ndef go():\n    return window(5)\n",
        );
        let resolved = event(&mut session, &["pkg/use.py"]);
        assert_eq!(
            resolved.deltas.iter().map(|d| d.weight).collect::<Vec<_>>(),
            vec![-1],
            "exactly one retraction"
        );
        assert!(session.suspects().is_empty());
    }

    /// Cold start as one delta: feeding the whole tree as a single event
    /// equals a survey plus a drift. This collapses watch mode and the
    /// incremental-equals-full invariant into one property.
    #[test]
    fn a_cold_start_as_one_event_equals_survey_plus_drift() {
        let (dir, base) = repo();
        let mut session = session(dir.path(), &base);
        write(dir.path(), "pkg/api.py", AFTER);
        write(
            dir.path(),
            "pkg/use.py",
            "from pkg.api import window\n\n\ndef go():\n    return window()\n\n\ndef also():\n    return window()\n",
        );
        let emitted = event(&mut session, &["pkg/api.py", "pkg/use.py"]);
        let mut sum = Vec::new();
        integrate(&mut sum, &emitted);
        assert_eq!(sum, full(dir.path(), &base));
    }

    /// Conservatism: a file the frontend cannot parse emits no deltas at all
    /// and is counted. Treating it as a file with nothing in it would retract
    /// every suspect it holds — an all-clear invented out of a syntax error.
    #[test]
    fn an_unparseable_file_emits_nothing_and_is_counted() {
        let (dir, base) = repo();
        let mut session = session(dir.path(), &base);
        write(dir.path(), "pkg/api.py", AFTER);
        event(&mut session, &["pkg/api.py"]);
        let before = keys(&session.suspects());
        assert!(!before.is_empty());

        write(
            dir.path(),
            "pkg/use.py",
            "def go(:\n    this does not parse\n",
        );
        let emitted = event(&mut session, &["pkg/use.py"]);
        assert!(
            emitted.deltas.is_empty(),
            "no deltas from a file grid cannot read"
        );
        assert_eq!(emitted.declined, vec!["pkg/use.py".to_owned()]);
        assert_eq!(session.declined(), 1);
        assert_eq!(
            keys(&session.suspects()),
            before,
            "and nothing was retracted behind the caller's back"
        );
    }
}
