use std::process::Command;

#[test]
fn test_server_cli_help_and_version() {
    let help_out = Command::new(env!("CARGO_BIN_EXE_dbsrt-server"))
        .arg("--help")
        .output()
        .expect("Failed to run dbsrt-server --help");
    assert!(help_out.status.success());
    let stdout = String::from_utf8_lossy(&help_out.stdout);
    assert!(stdout.contains("dbsrt REST microservice daemon"));
    assert!(stdout.contains("--host"));
    assert!(stdout.contains("--port"));
    assert!(stdout.contains("--phash-dir"));
    assert!(stdout.contains("--subtitles-dir"));

    let ver_out = Command::new(env!("CARGO_BIN_EXE_dbsrt-server"))
        .arg("--version")
        .output()
        .expect("Failed to run dbsrt-server --version");
    assert!(ver_out.status.success());
    let stdout_ver = String::from_utf8_lossy(&ver_out.stdout);
    assert!(stdout_ver.contains("dbsrt-server"));
}
