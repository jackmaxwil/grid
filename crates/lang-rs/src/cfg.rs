//! `#[cfg]` predicate evaluation against what one compile ran with.
//!
//! Three-valued on purpose. `Some(true)` is "this compile saw the item",
//! `Some(false)` is "it provably did not", and `None` is "this evaluator
//! cannot say" — a `cfg_attr`, a custom key such as `kani`, a `target_*`
//! fact the record does not carry. Unknown is never false: the consumer
//! (`grid check --compile`) gates on a proven exclusion and only warns on an
//! unproven one, and collapsing unknown into false here would turn every
//! custom cfg into a false hold.
use syn::punctuated::Punctuated;
use syn::{Meta, Token};

/// What a compile was asked for: enough to decide `test`, `feature = "…"`,
/// and the `target_*` family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CfgEnv<'a> {
    /// Features enabled, after defaults were applied.
    pub features: &'a [String],
    /// `--all-features`: every `feature = "…"` is true.
    pub all_features: bool,
    /// `--tests` / `--all-targets`: `cfg(test)` items were compiled.
    pub tests: bool,
    /// The target's cfg set as `rustc --print cfg` reports it, one entry per
    /// line (`unix`, `target_os="macos"`, …). Empty when nobody asked
    /// rustc, and then every target fact is unknown.
    pub cfgs: &'a [String],
}

/// Evaluate a predicate as written inside `#[cfg(...)]`.
///
/// A predicate that does not parse as a `syn::Meta` is unknown, not false.
#[must_use]
pub fn eval_cfg(predicate: &str, env: &CfgEnv) -> Option<bool> {
    let meta: Meta = syn::parse_str(predicate).ok()?;
    eval(&meta, env)
}

fn eval(meta: &Meta, env: &CfgEnv) -> Option<bool> {
    let key = meta.path().get_ident()?.to_string();
    match meta {
        Meta::Path(_) => match key.as_str() {
            "test" => Some(env.tests),
            "unix" | "windows" => target_fact(env, &key),
            _ => None,
        },
        Meta::List(list) => {
            let args = list
                .parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
                .ok()?;
            let values: Vec<Option<bool>> = args.iter().map(|m| eval(m, env)).collect();
            match key.as_str() {
                // Kleene logic: a decided false wins `all` and a decided true
                // wins `any` regardless of unknowns beside them.
                "all" => {
                    if values.contains(&Some(false)) {
                        Some(false)
                    } else if values.contains(&None) {
                        None
                    } else {
                        Some(true)
                    }
                }
                "any" => {
                    if values.contains(&Some(true)) {
                        Some(true)
                    } else if values.contains(&None) {
                        None
                    } else {
                        Some(false)
                    }
                }
                "not" => match values.as_slice() {
                    [one] => one.map(|v| !v),
                    _ => None,
                },
                _ => None,
            }
        }
        Meta::NameValue(nv) => {
            let syn::Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Str(lit),
                ..
            }) = &nv.value
            else {
                return None;
            };
            let want = lit.value();
            match key.as_str() {
                "feature" => Some(env.all_features || env.features.contains(&want)),
                "target_os"
                | "target_family"
                | "target_arch"
                | "target_env"
                | "target_vendor"
                | "target_pointer_width"
                | "target_endian" => target_fact(env, &format!("{key}=\"{want}\"")),
                _ => None,
            }
        }
    }
}

/// A fact about the target, answered only from rustc's own cfg listing —
/// no triple parsing, so an absent listing is honestly unknown.
fn target_fact(env: &CfgEnv, entry: &str) -> Option<bool> {
    if env.cfgs.is_empty() {
        return None;
    }
    Some(env.cfgs.iter().any(|c| c == entry))
}

#[cfg(test)]
mod tests {
    use super::{eval_cfg, CfgEnv};

    fn env<'a>(features: &'a [String], tests: bool, cfgs: &'a [String]) -> CfgEnv<'a> {
        CfgEnv {
            features,
            all_features: false,
            tests,
            cfgs,
        }
    }

    #[test]
    fn test_and_feature_decide() {
        let tui = ["tui".to_owned()];
        assert_eq!(eval_cfg("test", &env(&[], false, &[])), Some(false));
        assert_eq!(eval_cfg("test", &env(&[], true, &[])), Some(true));
        assert_eq!(
            eval_cfg("feature=\"tui\"", &env(&[], false, &[])),
            Some(false)
        );
        assert_eq!(
            eval_cfg("feature=\"tui\"", &env(&tui, false, &[])),
            Some(true)
        );
        let all = CfgEnv {
            all_features: true,
            ..env(&[], false, &[])
        };
        assert_eq!(eval_cfg("feature=\"anything\"", &all), Some(true));
    }

    #[test]
    fn combinators_are_three_valued() {
        let e = env(&[], false, &[]);
        assert_eq!(eval_cfg("not(test)", &e), Some(true));
        // A decided false wins `all` even beside an unknown.
        assert_eq!(eval_cfg("all(kani,test)", &e), Some(false));
        assert_eq!(eval_cfg("all(kani,not(test))", &e), None);
        // A decided true wins `any` even beside an unknown.
        assert_eq!(eval_cfg("any(kani,not(test))", &e), Some(true));
        assert_eq!(eval_cfg("any(kani,test)", &e), None);
    }

    /// Conservatism: nothing this evaluator cannot decide is ever false.
    #[test]
    fn unknown_is_never_false() {
        let e = env(&[], false, &[]);
        for p in [
            "kani",
            "cfg_attr(kani,allow(dead_code))",
            "debug_assertions",
            "target_os=\"macos\"",
            "unix",
            "not(kani)",
            "not a predicate",
        ] {
            assert_eq!(eval_cfg(p, &e), None, "{p}");
        }
    }

    #[test]
    fn target_facts_come_from_rustc_not_the_triple() {
        let cfgs = ["unix".to_owned(), "target_os=\"macos\"".to_owned()];
        let e = env(&[], false, &cfgs);
        assert_eq!(eval_cfg("unix", &e), Some(true));
        assert_eq!(eval_cfg("windows", &e), Some(false));
        assert_eq!(eval_cfg("target_os=\"macos\"", &e), Some(true));
        assert_eq!(eval_cfg("target_os=\"linux\"", &e), Some(false));
    }
}
