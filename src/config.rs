//! User configuration: the ignore list of regexes in hindsight.toml.
//!
//! Location (resolved in this order):
//!   1. $XDG_CONFIG_HOME/hindsight/hindsight.toml if XDG_CONFIG_HOME is set
//!   2. ~/.config/hindsight/hindsight.toml
//!
//! Commands whose text matches any ignore pattern are not recorded (filtered in
//! `db::start`) and are soft-deleted by `hindsight prune ignore`. Patterns are
//! UNANCHORED regexes — anchor with ^ / $ yourself (e.g. `^z( |$)`).

use regex::Regex;
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Deserialize, Default)]
struct RawConfig {
    #[serde(default)]
    ignore: Vec<String>,
}

/// Resolve the path to hindsight.toml. Note: this deliberately uses ~/.config
/// (XDG) rather than dirs::config_dir(), which on macOS is
/// ~/Library/Application Support.
pub fn config_path() -> Option<PathBuf> {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        if !xdg.is_empty() {
            return Some(PathBuf::from(xdg).join("hindsight").join("hindsight.toml"));
        }
    }
    dirs::home_dir().map(|h| h.join(".config").join("hindsight").join("hindsight.toml"))
}

/// Load the raw ignore patterns from hindsight.toml. Returns an empty list if the
/// file is missing or has no `ignore` key. Errors only on unreadable/invalid TOML.
pub fn load_ignore() -> anyhow::Result<Vec<String>> {
    let Some(path) = config_path() else {
        return Ok(Vec::new());
    };
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(anyhow::anyhow!("reading {}: {e}", path.display())),
    };
    let cfg: RawConfig =
        toml::from_str(&text).map_err(|e| anyhow::anyhow!("parsing {}: {e}", path.display()))?;
    Ok(cfg.ignore)
}

/// Compile patterns. Returns the successfully compiled regexes and, separately,
/// a list of (pattern, error message) for any that failed — so callers decide
/// whether to warn (hot path) or hard-error (explicit commands).
pub fn compile(patterns: &[String]) -> (Vec<Regex>, Vec<(String, String)>) {
    let mut good = Vec::new();
    let mut bad = Vec::new();
    for p in patterns {
        match Regex::new(p) {
            Ok(re) => good.push(re),
            Err(e) => bad.push((p.clone(), e.to_string())),
        }
    }
    (good, bad)
}

/// True if `cmd` matches any of the (unanchored) patterns.
pub fn is_ignored(cmd: &str, patterns: &[Regex]) -> bool {
    patterns.iter().any(|re| re.is_match(cmd))
}

/// Convenience for the hot recording path: load + compile the ignore list,
/// warning to stderr on any problem and failing open (returns whatever
/// compiled, or an empty list). Never returns an error, so recording is never
/// blocked by a bad config.
pub fn ignore_regexes_fail_open() -> Vec<Regex> {
    let patterns = match load_ignore() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("hindsight: ignoring hindsight.toml ({e})");
            return Vec::new();
        }
    };
    let (good, bad) = compile(&patterns);
    for (pat, err) in &bad {
        eprintln!("hindsight: skipping invalid ignore pattern {pat:?}: {err}");
    }
    good
}
