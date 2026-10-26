//! Process signal dispatching and inspection logic.

use std::fs;
use std::path::PathBuf;

use crate::commands::top::constants::{
    BYTES_PER_MIB_F64, MAX_LOG_LINES_PREVIEW, MAX_OPEN_FILES_PREVIEW,
};
use crate::commands::top::model::{format_duration, ActiveJobTelemetry};

/// Toggles pause state of a job process using POSIX SIGSTOP / SIGCONT.
pub fn toggle_pause_job(pid: u32) -> (bool, String) {
    let is_stopped = check_if_stopped(pid);
    let signal = if is_stopped {
        libc::SIGCONT
    } else {
        libc::SIGSTOP
    };
    let ret = unsafe { libc::kill(pid as i32, signal) };
    if ret == 0 {
        if is_stopped {
            (true, format!("PID {pid} repris (SIGCONT)"))
        } else {
            (true, format!("PID {pid} mis en pause (SIGSTOP)"))
        }
    } else {
        let err = std::io::Error::last_os_error();
        (false, format!("Erreur signal PID {pid}: {err}"))
    }
}

/// Sends graceful termination SIGTERM to a job process.
pub fn stop_job(pid: u32) -> (bool, String) {
    let ret = unsafe { libc::kill(pid as i32, libc::SIGTERM) };
    if ret == 0 {
        (true, format!("Signal SIGTERM envoyé au PID {pid}"))
    } else {
        let err = std::io::Error::last_os_error();
        (false, format!("Erreur SIGTERM PID {pid}: {err}"))
    }
}

/// Sends immediate termination SIGKILL to a job process.
pub fn kill_job(pid: u32) -> (bool, String) {
    let ret = unsafe { libc::kill(pid as i32, libc::SIGKILL) };
    if ret == 0 {
        (true, format!("Signal SIGKILL envoyé au PID {pid}"))
    } else {
        let err = std::io::Error::last_os_error();
        (false, format!("Erreur SIGKILL PID {pid}: {err}"))
    }
}

/// Checks if process status is currently STOPPED ('T' or 't').
pub fn check_if_stopped(pid: u32) -> bool {
    let status_path = format!("/proc/{pid}/status");
    if let Ok(content) = fs::read_to_string(status_path) {
        for line in content.lines() {
            if line.starts_with("State:") {
                let state = line.trim_start_matches("State:").trim();
                return state.starts_with('T') || state.starts_with('t');
            }
        }
    }
    false
}

/// Inspects process details and returns structured text lines for full-frame inspection.
pub fn inspect_job_details(job: &ActiveJobTelemetry) -> Vec<String> {
    let mut lines = Vec::new();

    lines.push(format!(
        "PID: {:<8} │ Moteur: {:<16} │ État: {}",
        job.pid,
        job.engine,
        job.status.to_uppercase()
    ));
    lines.push(format!(
        "Écoulé: {:<8} │ Débit CIFS: {:>6.1} MB/s │ CPU: {:>4.1}% │ RAM: {:>5.1} MB",
        format_duration(job.elapsed_secs),
        job.current_rate_mib_s,
        job.cpu_percent,
        job.ram_rss_mib
    ));
    lines.push(format!(
        "Progression: {:>5.1}% │ Lus: {:>6.1} MB / Total: {:>6.1} MB",
        job.progress_ratio * 100.0,
        job.read_bytes as f64 / BYTES_PER_MIB_F64,
        job.total_bytes as f64 / BYTES_PER_MIB_F64
    ));

    if let Some(eta) = job.eta_secs {
        lines.push(format!("ETA estimé: {}", format_duration(eta)));
    }

    lines.push(String::new());
    lines.push("--- Diagnostic Moteur & Accélération Matérielle ---".to_string());
    if let Some(ref reason) = job.engine_reason {
        lines.push(format!("  • Moteur sélectionné : {}", job.engine));
        lines.push(format!("  • Diagnostic matériel: {reason}"));
    } else if job.engine.contains("Hi10P") {
        lines.push(format!("  • Moteur sélectionné : {}", job.engine));
        lines.push(
            "  • Incompatibilité ASIC: Codec H.264 Hi10P (High 10 Profile, 10-bit color)."
                .to_string(),
        );
        lines.push("  • Raison repli CPU   : Les puces GPU grand public (Intel Iris Xe, Nvidia NVDEC, AMD VCN) ne disposent".to_string());
        lines.push("                         d'aucun décodeur matériel pour le H.264 10-bit (uniquement 8-bit supporté).".to_string());
        lines.push("  • Comportement       : dbsrt bascule automatiquement en décodage logiciel CPU multi-cœurs (consommation normale ~800%).".to_string());
    } else if job.engine.contains("GPU") {
        lines.push("  • Moteur sélectionné : GPU (VA-API)".to_string());
        lines.push("  • Accélération active: Décodage matériel ASIC GPU actif via Intel QuickSync / VA-API (/dev/dri/renderD128).".to_string());
        lines.push("  • Efficacité         : Déchargement CPU quasi-total (< 20% CPU hôte) et débit de décodage maximal.".to_string());
    } else if job.engine.contains("Sub") {
        lines.push("  • Moteur sélectionné : Sub Extraction".to_string());
        lines.push("  • Opération active   : Démultiplexage et extraction brute des pistes SRT (flux texte léger, aucun décodage vidéo).".to_string());
    } else {
        lines.push(format!("  • Moteur sélectionné : {}", job.engine));
        lines.push(
            "  • Mode Décodage      : Décodage logiciel CPU (sans accélération matérielle VA-API)."
                .to_string(),
        );
    }

    if !job.cmdline.is_empty() {
        lines.push(String::new());
        lines.push("--- Ligne de Commande Exécutée ---".to_string());
        lines.push(job.cmdline.join(" "));
    }

    // Inspect open file descriptors from /proc/{pid}/fd
    lines.push(String::new());
    lines.push("--- Descripteurs Fichiers & I/O Actifs ---".to_string());
    let fd_dir = format!("/proc/{}/fd", job.pid);
    let mut open_files = Vec::new();
    if let Ok(entries) = fs::read_dir(fd_dir) {
        for entry in entries.flatten() {
            if let Ok(target) = fs::read_link(entry.path()) {
                let target_str = target.to_string_lossy().to_string();
                if !target_str.starts_with("pipe:") && !target_str.starts_with("socket:") {
                    open_files.push(target_str);
                }
            }
        }
    }
    open_files.sort();
    open_files.dedup();

    if open_files.is_empty() {
        lines.push("  • Aucun fichier physique direct ouvert détecté".to_string());
    } else {
        for f in open_files.iter().take(MAX_OPEN_FILES_PREVIEW) {
            lines.push(format!("  • {f}"));
        }
        if open_files.len() > MAX_OPEN_FILES_PREVIEW {
            lines.push(format!(
                "  • ... et {} autres fichiers ouverts",
                open_files.len() - MAX_OPEN_FILES_PREVIEW
            ));
        }
    }

    // Check potential log files
    lines.push(String::new());
    lines.push("--- Extrait Derniers Logs d'Exécution ---".to_string());
    let mut log_found = false;
    let log_dirs = [
        PathBuf::from("data/logs"),
        PathBuf::from("/tmp"),
        PathBuf::from("data/reports"),
    ];
    let candidate_prefixes = [
        format!(
            "*{}*.log",
            job.media_name.chars().take(20).collect::<String>()
        ),
        format!("*{}*.log", job.pid),
    ];

    for dir in &log_dirs {
        if dir.exists() {
            for pattern in &candidate_prefixes {
                if let Ok(entries) = fs::read_dir(dir) {
                    for entry in entries.flatten() {
                        let fname = entry.file_name().to_string_lossy().to_string();
                        let target_sub = pattern.trim_matches('*');
                        if !target_sub.is_empty()
                            && fname.contains(target_sub)
                            && fname.ends_with(".log")
                        {
                            if let Ok(content) = fs::read_to_string(entry.path()) {
                                let file_lines: Vec<&str> = content.lines().collect();
                                let tail = file_lines
                                    .iter()
                                    .rev()
                                    .take(MAX_LOG_LINES_PREVIEW)
                                    .rev()
                                    .copied();
                                lines.push(format!("Source: {}", entry.path().display()));
                                for l in tail {
                                    lines.push(format!("  {l}"));
                                }
                                log_found = true;
                                break;
                            }
                        }
                    }
                }
                if log_found {
                    break;
                }
            }
        }
        if log_found {
            break;
        }
    }

    if !log_found {
        lines.push("  (Aucun fichier log dédié trouvé pour ce PID / média)".to_string());
    }

    lines
}
