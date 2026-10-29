//! Static host, OS, CPU, and GPU identification telemetry for `dbsrt top`.
//!
//! Provides dynamic discovery of local/remote node identity, operating system distribution,
//! processor architecture/core count, and physical graphics hardware without hardcoded assumptions.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::process::Command;
use sysinfo::System;

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

/// Static host and hardware identification sampled once at startup.
///
/// # Data Flow & Invariants
/// Probed at startup from system information, `/proc`, `/sys`, and `lspci`.
/// Emits clean, deterministic defaults in test environments.
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

/// Detects and returns static host system information by querying sysinfo and Linux subsystems.
///
/// # Arguments
/// None.
///
/// # Returns
/// A populated [`HostSystemInfo`] struct containing detected hardware and node parameters.
///
/// # Panics
/// Panic-free.
///
/// # Data Flow & Invariants
/// 1. Instantiates a transient `sysinfo::System` refreshed with CPU metrics.
/// 2. Delegates to [`detect_host_system_info_with_sys`].
pub fn detect_host_system_info() -> HostSystemInfo {
    let mut sys = System::new();
    sys.refresh_cpu_specifics(sysinfo::CpuRefreshKind::everything());
    detect_host_system_info_with_sys(&sys)
}

/// Probes host parameters reusing an existing initialized `sysinfo::System`.
///
/// # Arguments
/// * `sys` - Initialized system state reference.
///
/// # Returns
/// Fully resolved [`HostSystemInfo`] telemetry instance.
///
/// # Panics
/// Panic-free.
///
/// # Data Flow & Invariants
/// Aggregates results from `detect_hostname`, `detect_os_info`, `detect_cpu_info`, and `detect_gpu_model`.
pub fn detect_host_system_info_with_sys(sys: &System) -> HostSystemInfo {
    let hostname = detect_hostname(sys);
    let (os_name, kernel, arch) = detect_os_info();
    let (cpu_brand, cpu_cores_logical, cpu_cores_physical) = detect_cpu_info(sys);
    let gpu_name = detect_gpu_model();
    let app_version = env!("DBSRT_VERSION_SHORT").to_string();
    let git_hash = env!("DBSRT_GIT_HASH").to_string();
    let build_date = env!("DBSRT_BUILD_DATE").to_string();

    HostSystemInfo {
        hostname,
        os_name,
        kernel,
        arch,
        cpu_brand,
        cpu_cores_logical,
        cpu_cores_physical,
        gpu_name,
        app_version,
        git_hash,
        build_date,
    }
}

/// Resolves current node hostname with hierarchical fallbacks.
///
/// # Arguments
/// * `sys` - System state reference.
///
/// # Returns
/// Hostname string, or [`FALLBACK_HOSTNAME`] if unresolvable.
///
/// # Panics
/// Panic-free.
///
/// # Data Flow & Invariants
/// Queries `sysinfo::System::host_name()`, then `/proc/sys/kernel/hostname`,
/// then `/etc/hostname`, and finally the `$HOSTNAME` environment variable.
pub fn detect_hostname(_sys: &System) -> String {
    System::host_name()
        .or_else(|| {
            fs::read_to_string("/proc/sys/kernel/hostname")
                .ok()
                .map(|s| s.trim().to_string())
        })
        .or_else(|| {
            fs::read_to_string("/etc/hostname")
                .ok()
                .map(|s| s.trim().to_string())
        })
        .or_else(|| std::env::var("HOSTNAME").ok())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| FALLBACK_HOSTNAME.to_string())
}

/// Detects operating system distribution name, kernel release, and target architecture.
///
/// # Arguments
/// None.
///
/// # Returns
/// A tuple `(os_name, kernel, arch)`.
///
/// # Panics
/// Panic-free.
///
/// # Data Flow & Invariants
/// 1. Determines CPU architecture from `std::env::consts::ARCH`.
/// 2. Resolves kernel version from `/proc/sys/kernel/osrelease` or `System::kernel_version()`.
/// 3. Resolves distribution name from `/etc/os-release`, then `/proc/version`, and finally `System::name()`.
pub fn detect_os_info() -> (String, String, String) {
    let arch = std::env::consts::ARCH.to_string();
    let kernel = fs::read_to_string("/proc/sys/kernel/osrelease")
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| System::kernel_version().unwrap_or_else(|| "unknown".to_string()));

    let os_name = parse_os_release()
        .or_else(detect_os_from_proc_version)
        .unwrap_or_else(|| System::name().unwrap_or_else(|| FALLBACK_OS_NAME.to_string()));

    (os_name, kernel, arch)
}

/// Parses Linux distribution name from `/etc/os-release` or `/usr/lib/os-release`.
///
/// # Arguments
/// None.
///
/// # Returns
/// `Some(String)` with distribution name if found, `None` otherwise.
///
/// # Panics
/// Panic-free.
///
/// # Data Flow & Invariants
/// Searches for `PRETTY_NAME` or `NAME` key, stripping surrounding quotes.
pub fn parse_os_release() -> Option<String> {
    let candidates = ["/etc/os-release", "/usr/lib/os-release"];
    for path in &candidates {
        let content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        if let Some(name) = parse_os_release_content(&content) {
            return Some(name);
        }
    }
    None
}

/// Helper parsing `PRETTY_NAME` or `NAME` from os-release text.
fn parse_os_release_content(content: &str) -> Option<String> {
    for line in content.lines() {
        if let Some(val) = line.strip_prefix("PRETTY_NAME=") {
            let clean = val.trim().trim_matches('"').trim_matches('\'');
            if !clean.is_empty() {
                return Some(clean.to_string());
            }
        }
    }
    for line in content.lines() {
        if let Some(val) = line.strip_prefix("NAME=") {
            let clean = val.trim().trim_matches('"').trim_matches('\'');
            if !clean.is_empty() {
                return Some(clean.to_string());
            }
        }
    }
    None
}

/// Detects distribution family by inspecting `/proc/version` strings.
///
/// # Arguments
/// None.
///
/// # Returns
/// `Some(String)` if recognizable keywords are identified, `None` otherwise.
///
/// # Panics
/// Panic-free.
///
/// # Data Flow & Invariants
/// Provides crucial fallback for jailed environments (like o2switch) where `/etc/os-release`
/// is inaccessible, identifying CloudLinux, Red Hat, Debian, or Ubuntu markers.
pub fn detect_os_from_proc_version() -> Option<String> {
    classify_proc_version(&fs::read_to_string("/proc/version").ok()?)
}

/// Maps `/proc/version` text to a distribution family label.
fn classify_proc_version(content: &str) -> Option<String> {
    let version_str = content.to_lowercase();
    if version_str.contains("cloudlinux") {
        Some("CloudLinux".to_string())
    } else if version_str.contains("red hat")
        || version_str.contains(".el8.")
        || version_str.contains(".el9.")
    {
        Some("Enterprise Linux (RHEL)".to_string())
    } else if version_str.contains("ubuntu") {
        Some("Ubuntu Linux".to_string())
    } else if version_str.contains("debian") {
        Some("Debian GNU/Linux".to_string())
    } else {
        None
    }
}

/// Detects CPU brand string and core counts from `sysinfo` and `/proc/cpuinfo`.
///
/// # Arguments
/// * `sys` - Initialized system state reference.
///
/// # Returns
/// A tuple `(cpu_brand, logical_cores, physical_cores)`.
///
/// # Panics
/// Panic-free.
///
/// # Data Flow & Invariants
/// 1. Queries logical thread count from `sys.cpus().len()`.
/// 2. Reads commercial processor brand name from first available CPU core.
/// 3. If brand is blank (e.g. ARM RPi4), parses `Model` or `Hardware` from `/proc/cpuinfo`.
pub fn detect_cpu_info(sys: &System) -> (String, usize, Option<usize>) {
    let logical_cores = sys.cpus().len().max(DEFAULT_MIN_CORES);
    let physical_cores = sys.physical_core_count();

    let mut brand = sys
        .cpus()
        .first()
        .map(|c| c.brand().trim().to_string())
        .unwrap_or_default();

    if brand.is_empty() {
        if let Ok(cpuinfo) = fs::read_to_string("/proc/cpuinfo") {
            if let Some(fallback) = parse_cpuinfo_fallback(&cpuinfo) {
                brand = fallback;
            }
        }
    }

    if brand.is_empty() {
        brand = FALLBACK_CPU_BRAND.to_string();
    }

    (brand, logical_cores, physical_cores)
}

/// Helper parsing `Model` or `Hardware` identifiers from `/proc/cpuinfo` text.
fn parse_cpuinfo_fallback(cpuinfo: &str) -> Option<String> {
    let mut hardware = None;
    for line in cpuinfo.lines() {
        if line.starts_with("Model\t") || line.starts_with("Model :") {
            if let Some((_, val)) = line.split_once(':') {
                return Some(val.trim().to_string());
            }
        }
        if hardware.is_none() && (line.starts_with("Hardware\t") || line.starts_with("Hardware :"))
        {
            if let Some((_, val)) = line.split_once(':') {
                hardware = Some(val.trim().to_string());
            }
        }
    }
    hardware
}

/// Identifies active or installed GPU accelerator using `lspci` or `/sys/class/drm`.
///
/// # Arguments
/// None.
///
/// # Returns
/// Detected GPU model name string, or [`FALLBACK_GPU_NONE`].
///
/// # Panics
/// Panic-free.
///
/// # Data Flow & Invariants
/// 1. Attempts discovery via `lspci` PCI bus scan.
/// 2. Falls back to DRM sysfs driver binding inspection (`/sys/class/drm`).
/// 3. Returns fallback indicator if no graphics accelerator is detected.
pub fn detect_gpu_model() -> String {
    if let Some(gpu) = probe_gpu_via_lspci() {
        return gpu;
    }

    if let Some(gpu) = probe_gpu_via_drm_sysfs() {
        return gpu;
    }

    FALLBACK_GPU_NONE.to_string()
}

/// Probes PCI devices for VGA, 3D, and Display controllers via `lspci`.
///
/// # Arguments
/// None.
///
/// # Returns
/// `Some(String)` with clean commercial GPU name if found, `None` otherwise.
///
/// # Panics
/// Panic-free.
///
/// # Data Flow & Invariants
/// Extracts text from square brackets `[...]` or after controller prefix colon.
pub fn probe_gpu_via_lspci() -> Option<String> {
    let output = Command::new("lspci").output().ok()?;
    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        if let Some(gpu) = parse_lspci_gpu_line(line) {
            return Some(gpu);
        }
    }

    None
}

/// Helper parsing commercial GPU model from a single `lspci` line.
fn parse_lspci_gpu_line(line: &str) -> Option<String> {
    let lower = line.to_lowercase();
    let is_gpu = lower.contains("vga compatible")
        || lower.contains("3d controller")
        || lower.contains("display controller");
    if !is_gpu {
        return None;
    }

    if let Some(bracket_text) = extract_last_bracket(line) {
        return Some(bracket_text);
    }

    // ": " skips the colons of the PCI address (e.g. `0000:00:02.0`).
    let (_, desc) = line.split_once(": ")?;
    let clean = desc.trim();
    if let Some((clean_no_rev, _)) = clean.split_once("(rev") {
        Some(clean_no_rev.trim().to_string())
    } else {
        Some(clean.to_string())
    }
}

/// Extracts content of the last square bracket block in a line.
///
/// # Arguments
/// * `s` - Source string containing optional square bracket pairs.
///
/// # Returns
/// `Some(String)` if a valid bracket pair exists, `None` otherwise.
///
/// # Panics
/// Panic-free.
///
/// # Data Flow & Invariants
/// Finds last `[` and matching `]`, returning the interior trimmed string.
pub fn extract_last_bracket(s: &str) -> Option<String> {
    let open_idx = s.rfind('[')?;
    let close_idx = s[open_idx..].find(']')? + open_idx;
    let interior = s[open_idx + 1..close_idx].trim();
    if !interior.is_empty() {
        Some(interior.to_string())
    } else {
        None
    }
}

/// Inspects `/sys/class/drm` symlinks to identify bound graphics drivers.
///
/// # Arguments
/// None.
///
/// # Returns
/// `Some(String)` denoting graphics accelerator family, `None` if DRM subsystem is absent.
///
/// # Panics
/// Panic-free.
///
/// # Data Flow & Invariants
/// Inspects driver symlinks under `/sys/class/drm/card0/device/driver`, mapping kernel driver
/// identifiers (`v3d`, `vc4`, `i915`, `xe`, `nvidia`, `amdgpu`) to human-readable labels.
pub fn probe_gpu_via_drm_sysfs() -> Option<String> {
    let card0_driver = Path::new("/sys/class/drm/card0/device/driver");
    let target = fs::read_link(card0_driver).ok()?;
    drm_driver_label(&target.file_name()?.to_string_lossy())
}

/// Maps a kernel DRM driver name to a human-readable GPU label.
fn drm_driver_label(driver_name: &str) -> Option<String> {
    match driver_name {
        "v3d" | "vc4" => Some("Broadcom VideoCore VI (v3d)".to_string()),
        "i915" => Some("Iris Xe Graphics (i915)".to_string()),
        "xe" => Some("Intel Xe Graphics (xe)".to_string()),
        "nvidia" => Some("NVIDIA GPU (CUDA/NVDEC)".to_string()),
        "nouveau" => Some("NVIDIA GPU (nouveau)".to_string()),
        "amdgpu" | "radeon" => Some("AMD Radeon GPU".to_string()),
        _ => Some(format!("GPU ({driver_name})")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_lspci_gpu_line_without_brackets() {
        let sample = "0000:00:02.0 VGA compatible controller: Matrox G200eR2 (rev 01)";
        assert_eq!(
            parse_lspci_gpu_line(sample),
            Some("Matrox G200eR2".to_string())
        );
        assert_eq!(parse_lspci_gpu_line("00:1f.3 Audio device: Intel"), None);
    }

    #[test]
    fn test_extract_last_bracket() {
        let sample = "00:02.0 VGA compatible controller: Intel Corporation Raptor Lake-P [Iris Xe Graphics] (rev 04)";
        assert_eq!(
            extract_last_bracket(sample),
            Some("Iris Xe Graphics".to_string())
        );

        let sample_multi =
            "03:00.0 VGA compatible controller: [AMD/ATI] Navi 22 [Radeon RX 6700 XT] (rev c1)";
        assert_eq!(
            extract_last_bracket(sample_multi),
            Some("Radeon RX 6700 XT".to_string())
        );

        let sample_none = "00:02.0 VGA controller: Intel HD 530";
        assert_eq!(extract_last_bracket(sample_none), None);
    }

    #[test]
    fn test_host_system_info_default() {
        let def = HostSystemInfo::default();
        assert_eq!(def.hostname, UNKNOWN_FIELD);
        assert_eq!(def.gpu_name, UNKNOWN_FIELD);
        assert_eq!(def.git_hash, UNKNOWN_FIELD);
        assert_eq!(def.cpu_cores_logical, 0);
        assert_eq!(def.cpu_cores_physical, None);
    }

    #[test]
    fn test_parse_os_release_content_prefers_pretty_name() {
        let both = "NAME=\"Debian\"\nPRETTY_NAME=\"Debian 13\"\n";
        assert_eq!(parse_os_release_content(both), Some("Debian 13".into()));
        assert_eq!(
            parse_os_release_content("NAME='Alpine'\n"),
            Some("Alpine".into())
        );
        assert_eq!(parse_os_release_content("PRETTY_NAME=\"\"\nID=x\n"), None);
    }

    #[test]
    fn test_classify_proc_version() {
        let el = "Linux version 4.18.0-513.el8.x86_64 (gcc)";
        assert_eq!(
            classify_proc_version(el),
            Some("Enterprise Linux (RHEL)".into())
        );
        let cl = "Linux version 4.18.0 (CloudLinux build)";
        assert_eq!(classify_proc_version(cl), Some("CloudLinux".into()));
        assert_eq!(classify_proc_version("Linux version 6.12.0 (gcc)"), None);
    }

    #[test]
    fn test_parse_cpuinfo_fallback_prefers_model_over_hardware() {
        let rpi = "processor\t: 0\nHardware\t: BCM2835\nModel\t\t: Raspberry Pi 4 Model B\n";
        assert_eq!(
            parse_cpuinfo_fallback(rpi),
            Some("Raspberry Pi 4 Model B".into())
        );
        assert_eq!(
            parse_cpuinfo_fallback("Hardware\t: BCM2835\n"),
            Some("BCM2835".into())
        );
        assert_eq!(parse_cpuinfo_fallback("processor\t: 0\n"), None);
    }

    #[test]
    fn test_parse_lspci_gpu_line() {
        let nv = "01:00.0 3D controller: NVIDIA Corporation GA107M (rev a1)";
        assert_eq!(
            parse_lspci_gpu_line(nv),
            Some("NVIDIA Corporation GA107M".into())
        );
        let net = "02:00.0 Ethernet controller: Intel Corporation I219-V";
        assert_eq!(parse_lspci_gpu_line(net), None);
    }

    #[test]
    fn test_drm_driver_label() {
        assert_eq!(
            drm_driver_label("v3d"),
            Some("Broadcom VideoCore VI (v3d)".into())
        );
        assert_eq!(
            drm_driver_label("virtio_gpu"),
            Some("GPU (virtio_gpu)".into())
        );
    }
}
