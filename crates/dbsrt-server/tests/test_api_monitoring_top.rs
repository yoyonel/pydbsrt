#![cfg(unix)]

use dbsrt_server::{create_router, top_snapshot, TopSnapshotCache};
use reqwest::StatusCode;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

/// Lays out `<dir>/dbsrt-server` (never run) next to a fake `<dir>/dbsrt` shell script.
fn fake_cli(case: &str, script: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("test_top_{}_{}", case, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let cli = dir.join("dbsrt");
    std::fs::write(&cli, script).unwrap();
    std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755)).unwrap();
    dir.join("dbsrt-server")
}

fn spawn_count(server_exe: &Path) -> usize {
    std::fs::read_to_string(server_exe.with_file_name("spawns"))
        .map(|s| s.lines().count())
        .unwrap_or(0)
}

// One sequential test: forking while another test thread holds a script open for writing
// would make exec fail with ETXTBSY.
#[tokio::test]
async fn test_top_snapshot_success_cache_and_errors() {
    let ok = fake_cli(
        "ok",
        "#!/bin/sh\n[ \"$1 $2\" = 'top --json' ] || exit 9\necho x >> \"$(dirname \"$0\")/spawns\"\nprintf '{\"ok\":1}'\n",
    );
    let cache = TopSnapshotCache::const_new(None);
    let body = top_snapshot(&ok, &cache).await.expect("fake top succeeds");
    assert_eq!(&body[..], br#"{"ok":1}"#);
    let again = top_snapshot(&ok, &cache).await.expect("cached");
    assert_eq!(again, body);
    assert_eq!(
        spawn_count(&ok),
        1,
        "second call within the TTL must reuse the cache"
    );

    let failing = fake_cli("fail", "#!/bin/sh\necho boom >&2\nexit 3\n");
    let cache = TopSnapshotCache::const_new(None);
    let (code, msg) = top_snapshot(&failing, &cache).await.unwrap_err();
    assert_eq!(code, StatusCode::BAD_GATEWAY);
    assert_eq!(msg.trim(), "boom");
    assert!(cache.lock().await.is_none(), "failures are never cached");

    let missing = std::env::temp_dir()
        .join(format!("test_top_missing_{}", std::process::id()))
        .join("dbsrt-server");
    let (code, _) = top_snapshot(&missing, &TopSnapshotCache::const_new(None))
        .await
        .unwrap_err();
    assert_eq!(code, StatusCode::INTERNAL_SERVER_ERROR);

    for exe in [&ok, &failing] {
        let _ = std::fs::remove_dir_all(exe.parent().unwrap());
    }

    // Route wiring: the test harness has no sibling `dbsrt`, so the handler maps the spawn
    // failure to 500 instead of panicking or hanging.
    let phash_dir = std::env::temp_dir().join(format!("test_phash_top_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&phash_dir);
    let app = create_router(phash_dir.clone(), PathBuf::from("../../data/SRT"));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let res = reqwest::get(format!("http://{}/v1/monitoring/top", addr))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let _ = std::fs::remove_dir_all(&phash_dir);
}
