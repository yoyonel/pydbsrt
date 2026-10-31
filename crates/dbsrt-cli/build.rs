//! Build script capturing Git commit hash, build date, and universal binary versioning.
//!
//! Precedence for each value:
//! 1. Explicit env override (`DBSRT_GIT_HASH`, `DBSRT_BUILD_DATE`), e.g. passed as Docker build args
//!    where `.git` is excluded from the build context.
//! 2. `SOURCE_DATE_EPOCH` (reproducible builds convention) for the date.
//! 3. Live `git` queries.
//! 4. `UNKNOWN_VALUE`.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Fallback when neither an env override nor git metadata is available.
const UNKNOWN_VALUE: &str = "unknown";
const GIT_HASH_ENV: &str = "DBSRT_GIT_HASH";
const BUILD_DATE_ENV: &str = "DBSRT_BUILD_DATE";
const SOURCE_DATE_EPOCH_ENV: &str = "SOURCE_DATE_EPOCH";
const SECONDS_PER_DAY: i64 = 86_400;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    for var in [GIT_HASH_ENV, BUILD_DATE_ENV, SOURCE_DATE_EPOCH_ENV] {
        println!("cargo:rerun-if-env-changed={var}");
    }
    emit_git_rerun_triggers();

    let pkg_version = env!("CARGO_PKG_VERSION");
    let git_hash = env_override(GIT_HASH_ENV)
        .or_else(git_short_hash)
        .unwrap_or_else(|| UNKNOWN_VALUE.to_string());
    let commit_date = env_override(BUILD_DATE_ENV)
        .or_else(|| env_override(SOURCE_DATE_EPOCH_ENV).and_then(|s| epoch_to_iso_date(&s)))
        .or_else(|| git_output(&["log", "-1", "--format=%cs"]))
        .unwrap_or_else(|| UNKNOWN_VALUE.to_string());

    let full_version = format!("{pkg_version} ({git_hash} {commit_date})");
    let short_version = format!("v{pkg_version} ({git_hash} {commit_date})");

    println!("cargo:rustc-env=DBSRT_GIT_HASH={git_hash}");
    println!("cargo:rustc-env=DBSRT_BUILD_DATE={commit_date}");
    println!("cargo:rustc-env=DBSRT_VERSION_FULL={full_version}");
    println!("cargo:rustc-env=DBSRT_VERSION_SHORT={short_version}");
}

fn env_override(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// Runs a git command and returns its trimmed stdout when successful and non-empty.
fn git_output(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!text.is_empty()).then_some(text)
}

fn git_short_hash() -> Option<String> {
    let mut hash = git_output(&["rev-parse", "--short", "HEAD"])?;
    let dirty = Command::new("git")
        .args(["diff", "--quiet"])
        .status()
        .map(|s| !s.success())
        .unwrap_or(false);
    if dirty {
        hash.push_str("-dirty");
    }
    Some(hash)
}

/// Watches the real git metadata files (resolved by git itself, so worktrees and
/// crate-relative build dirs work). Only existing paths are emitted: a missing
/// `rerun-if-changed` path makes cargo rerun the script on every build.
fn emit_git_rerun_triggers() {
    let Some(git_dir) = git_output(&["rev-parse", "--absolute-git-dir"]).map(PathBuf::from) else {
        return;
    };
    let common_dir = git_output(&["rev-parse", "--path-format=absolute", "--git-common-dir"])
        .map(PathBuf::from)
        .unwrap_or_else(|| git_dir.clone());

    let mut watched = vec![git_dir.join("HEAD"), git_dir.join("index")];
    if let Some(head_ref) = git_output(&["symbolic-ref", "-q", "HEAD"]) {
        watched.push(common_dir.join(head_ref));
    }
    watched.push(common_dir.join("packed-refs"));

    for path in watched.iter().filter(|p| Path::new(p).exists()) {
        println!("cargo:rerun-if-changed={}", path.display());
    }
}

/// Converts a Unix epoch (seconds) into a `YYYY-MM-DD` UTC civil date
/// (Howard Hinnant's days-to-civil algorithm, no external crate in build scripts).
fn epoch_to_iso_date(epoch: &str) -> Option<String> {
    let secs: i64 = epoch.parse().ok()?;
    let z = secs.div_euclid(SECONDS_PER_DAY) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    Some(format!("{year:04}-{month:02}-{day:02}"))
}
