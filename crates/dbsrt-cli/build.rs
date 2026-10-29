//! Build script capturing Git commit hash, build date, and universal binary versioning.

use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/index");

    let pkg_version = env!("CARGO_PKG_VERSION");

    // 1. Get git short hash with optional dirty flag
    let git_hash = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .and_then(|out| {
            if out.status.success() {
                let mut hash = String::from_utf8_lossy(&out.stdout).trim().to_string();
                let dirty = Command::new("git")
                    .args(["diff", "--quiet"])
                    .status()
                    .map(|s| !s.success())
                    .unwrap_or(false);
                if dirty {
                    hash.push_str("-dirty");
                }
                Some(hash)
            } else {
                None
            }
        })
        .unwrap_or_else(|| "release".to_string());

    // 2. Get latest commit date or build date
    let commit_date = Command::new("git")
        .args(["log", "-1", "--format=%cs"])
        .output()
        .ok()
        .and_then(|out| {
            if out.status.success() {
                let d = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if !d.is_empty() {
                    Some(d)
                } else {
                    None
                }
            } else {
                None
            }
        })
        .unwrap_or_else(|| "2026-10-29".to_string());

    let full_version = format!("{pkg_version} ({git_hash} {commit_date})");
    let short_version = format!("v{pkg_version} ({git_hash} {commit_date})");

    println!("cargo:rustc-env=DBSRT_GIT_HASH={git_hash}");
    println!("cargo:rustc-env=DBSRT_BUILD_DATE={commit_date}");
    println!("cargo:rustc-env=DBSRT_VERSION_FULL={full_version}");
    println!("cargo:rustc-env=DBSRT_VERSION_SHORT={short_version}");
}
