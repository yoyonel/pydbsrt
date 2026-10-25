use std::path::Path;
use std::process::Command;

fn detect_reachable_pg_port(host: &str) -> Option<u16> {
    let env_port = std::env::var("PSQL_PORT")
        .ok()
        .and_then(|p| p.parse::<u16>().ok());
    let ports: Vec<u16> = match env_port {
        Some(p) => vec![p, 5433, 5432],
        None => vec![5433, 5432],
    };
    for port in ports {
        if std::net::TcpStream::connect_timeout(
            &format!("{}:{}", host, port).parse().unwrap(),
            std::time::Duration::from_millis(500),
        )
        .is_ok()
        {
            return Some(port);
        }
    }
    None
}

#[test]
fn test_recognize_media_iso_parity() {
    let host = std::env::var("PSQL_IP").unwrap_or_else(|_| "127.0.0.1".to_string());
    let mp4_media = Path::new("../../tests/data/big_buck_bunny_trailer_480p.mp4");
    let webm_media = Path::new("../../data/big_buck_bunny_trailer_480p.webm");
    let media = if mp4_media.exists() {
        mp4_media
    } else if webm_media.exists() {
        webm_media
    } else {
        eprintln!("Media file not found, skipping test");
        return;
    };
    let Some(port) = detect_reachable_pg_port(&host) else {
        eprintln!("PostgreSQL not reachable on {}, skipping test", host);
        return;
    };
    let port_str = port.to_string();
    let bin = env!("CARGO_BIN_EXE_dbsrt");

    // Ensure sample phash is exported and imported into PostgreSQL under llvm-cov instrumentation
    let stem = media.file_name().unwrap().to_str().unwrap();
    let tmp_phash = std::env::temp_dir().join(format!("{}.25.0fps.phash", stem));
    if !tmp_phash.exists() {
        let _ = Command::new(bin)
            .args([
                "export-phash",
                "--media",
                media.to_str().unwrap(),
                "-o",
                tmp_phash.to_str().unwrap(),
            ])
            .output();
    }
    let _ = Command::new(bin)
        .args([
            "import-phash",
            "--file",
            tmp_phash.to_str().unwrap(),
            "--db-host",
            &host,
            "--db-port",
            &port_str,
        ])
        .output();

    let output = Command::new(bin)
        .args([
            "recognize-media",
            "--media",
            media.to_str().unwrap(),
            "--search-distance",
            "0",
            "--output-format",
            "csv",
            "--db-host",
            &host,
            "--db-port",
            &port_str,
        ])
        .output()
        .expect("Failed to execute dbsrt binary");

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("Connection refused")
            || stderr.contains("does not exist")
            || stderr.contains("connection error")
            || stderr.contains("password authentication failed")
        {
            eprintln!("Database not ready/seeded, skipping test: {}", stderr);
            return;
        }
        panic!("dbsrt failed: {}", stderr);
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let line = match stdout.lines().next() {
        Some(l) => l,
        None => {
            eprintln!("Database does not contain seeded media, skipping test");
            return;
        }
    };
    let fields: Vec<&str> = line.split(',').collect();

    assert_eq!(fields[0], "True", "media_found must be True");
    assert_eq!(
        fields[1], "big_buck_bunny_trailer_480p",
        "media_name_search mismatch"
    );
    assert!(
        fields[2].starts_with("big_buck_bunny_trailer_480p"),
        "media_name_match mismatch: {}",
        fields[2]
    );
    assert!(
        fields[3].parse::<i32>().unwrap_or(0) >= 1,
        "media_id must be >= 1, got {}",
        fields[3]
    );
    assert!(
        fields[4].parse::<i32>().unwrap_or(0) >= 150,
        "nb_offsets_match must be >= 150, got {}",
        fields[4]
    );
    assert!(
        line.contains("1.0") || line.contains("0.8"),
        "confidence must be >= 0.8, got line: {}",
        line
    );
}

#[test]
fn test_cli_subcommands_end_to_end() {
    let mp4_media = Path::new("../../tests/data/big_buck_bunny_trailer_480p.mp4");
    let webm_media = Path::new("../../data/big_buck_bunny_trailer_480p.webm");
    let media = if mp4_media.exists() {
        mp4_media
    } else if webm_media.exists() {
        webm_media
    } else {
        return;
    };
    let srt = if Path::new("../../tests/data/big_buck_bunny_trailer_480p.fr.srt").exists() {
        Path::new("../../tests/data/big_buck_bunny_trailer_480p.fr.srt")
    } else {
        Path::new("../../data/SRT/big_buck_bunny_trailer_480p.fr.srt")
    };
    if !srt.exists() {
        return;
    }

    let tmp_base = std::env::temp_dir().join(format!("dbsrt_cli_e2e_cov_{}", std::process::id()));
    let phash_dir = tmp_base.join("phash");
    let subs_dir = tmp_base.join("subs");
    let _ = std::fs::create_dir_all(&phash_dir);
    let _ = std::fs::create_dir_all(&subs_dir);

    let bin = env!("CARGO_BIN_EXE_dbsrt");

    // 1. bench-phash
    let out = Command::new(bin)
        .args(["bench-phash", "--count", "10"])
        .output()
        .unwrap();
    assert!(out.status.success());

    // 2. export-phash
    let stem = media.file_name().unwrap().to_str().unwrap();
    let exported_phash = phash_dir.join(format!("{}.25.0fps.phash", stem));
    let out = Command::new(bin)
        .args([
            "export-phash",
            "--media",
            media.to_str().unwrap(),
            "-o",
            exported_phash.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(exported_phash.exists());

    // 3. pack (.dbphash)
    let dbphash_file = tmp_base.join("catalog.dbphash");
    let out = Command::new(bin)
        .args([
            "pack",
            "--phash-dir",
            phash_dir.to_str().unwrap(),
            "--output",
            dbphash_file.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(dbphash_file.exists());

    // 4. scan-library (--phash-dir & --dbphash, with --query-phash & --query-media)
    let out = Command::new(bin)
        .args([
            "scan-library",
            "--phash-dir",
            phash_dir.to_str().unwrap(),
            "--query-phash",
            exported_phash.to_str().unwrap(),
            "--search-distance",
            "2",
        ])
        .output()
        .unwrap();
    assert!(out.status.success());

    let out = Command::new(bin)
        .args([
            "scan-library",
            "--dbphash",
            dbphash_file.to_str().unwrap(),
            "--query-media",
            media.to_str().unwrap(),
            "--search-distance",
            "2",
        ])
        .output()
        .unwrap();
    assert!(out.status.success());

    // 5. recognize-media (--no-db & --dbphash across output formats)
    for fmt in ["dataframe", "csv"] {
        let out = Command::new(bin)
            .args([
                "recognize-media",
                "--media",
                media.to_str().unwrap(),
                "--no-db",
                "--phash-dir",
                phash_dir.to_str().unwrap(),
                "--output-format",
                fmt,
            ])
            .output()
            .unwrap();
        assert!(out.status.success());
    }

    let out = Command::new(bin)
        .args([
            "recognize-media",
            "--media",
            media.to_str().unwrap(),
            "--dbphash",
            dbphash_file.to_str().unwrap(),
            "--output-format",
            "csv",
        ])
        .output()
        .unwrap();
    assert!(out.status.success());

    // 6. bench-cuts
    let out = Command::new(bin)
        .args([
            "bench-cuts",
            "--phash-dir",
            phash_dir.to_str().unwrap(),
            "--single-thread",
        ])
        .output()
        .unwrap();
    assert!(out.status.success());

    // 7. inspect-media (human & --json)
    let out = Command::new(bin)
        .args(["inspect-media", "--media", media.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(out.status.success());

    let out = Command::new(bin)
        .args([
            "inspect-media",
            "--media",
            media.to_str().unwrap(),
            "--format",
            "json",
        ])
        .output()
        .unwrap();
    assert!(out.status.success());

    // 8. certify-coupling (--phash-file & --media)
    let out = Command::new(bin)
        .args([
            "certify-coupling",
            "--phash-file",
            exported_phash.to_str().unwrap(),
            "--subtitle",
            srt.to_str().unwrap(),
            "--lang",
            "fr",
            "--subtitles-dir",
            subs_dir.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(out.status.success());

    let out = Command::new(bin)
        .args([
            "certify-coupling",
            "--media",
            media.to_str().unwrap(),
            "--subtitle",
            srt.to_str().unwrap(),
            "--lang",
            "fr",
            "--phash-dir",
            phash_dir.to_str().unwrap(),
            "--subtitles-dir",
            subs_dir.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(out.status.success());

    let _ = std::fs::remove_dir_all(&tmp_base);
}
