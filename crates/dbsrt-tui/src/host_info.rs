//! Static host, OS, CPU, and GPU identification telemetry for `dbsrt top`.
//!
//! Provides data structures for local and remote node identity, operating system distribution,
//! processor architecture/core count, and physical graphics hardware.

use serde::{Deserialize, Serialize};

/// Fallback hostname when host resolution yields empty or unavailable descriptors.
pub const FALLBACK_HOSTNAME: &str = "localhost";
/// Fallback operating system name when os-release or sysinfo cannot be determined.
pub const FALLBACK_OS_NAME: &str = "Linux";
/// Fallback CPU brand designation when processor metadata cannot be parsed.
pub const FALLBACK_CPU_BRAND: &str = "Generic CPU";
/// Standard indicator denoting absence of dedicated or integrated graphics accelerators.
pub const FALLBACK_GPU_NONE: &str = "Non détecté";
/// Minimum default core count ceiling floor.
pub const DEFAULT_MIN_CORES: usize = 1;

/// Static host and hardware identification sampled once at startup or received via telemetry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostSystemInfo {
    /// Hostname of the current node (e.g. "sapin.o2switch.net", "raspberrypi", "workstation").
    pub hostname: String,
    /// Operating system distribution description (e.g. "Debian 13 (trixie)", "CloudLinux 8.10").
    pub os_name: String,
    /// Linux kernel version string (e.g. "6.12.73", "4.18.0").
    pub kernel: String,
    /// Target CPU architecture (e.g. "x86_64", "aarch64").
    pub arch: String,
    /// Commercial CPU model name (e.g. "13th Gen Intel(R) Core(TM) i7-1355U", "Intel(R) Xeon(R) Gold 6263CY").
    pub cpu_brand: String,
    /// Total count of logical CPU threads available.
    pub cpu_cores_logical: usize,
    /// Total count of physical CPU cores (if available).
    pub cpu_cores_physical: Option<usize>,
    /// Commercial or detected GPU model name (e.g. "Iris Xe Graphics", "Broadcom VideoCore VI", "Non détecté").
    pub gpu_name: String,
    /// Application binary build version and git provenance (e.g. "v0.1.0 (e79795f 2026-10-29)").
    pub app_version: String,
    /// Git commit short hash (e.g. "e79795f").
    pub git_hash: String,
    /// Compilation ISO date (e.g. "2026-10-29").
    pub build_date: String,
}

/// Neutral placeholder for identification fields that have not been probed yet.
pub const UNKNOWN_FIELD: &str = "unknown";

/// Neutral, machine-independent placeholder used before the first probe (and in tests).
impl Default for HostSystemInfo {
    fn default() -> Self {
        let unknown = || UNKNOWN_FIELD.to_string();
        Self {
            hostname: unknown(),
            os_name: unknown(),
            kernel: unknown(),
            arch: unknown(),
            cpu_brand: unknown(),
            cpu_cores_logical: 0,
            cpu_cores_physical: None,
            gpu_name: unknown(),
            app_version: unknown(),
            git_hash: unknown(),
            build_date: unknown(),
        }
    }
}
