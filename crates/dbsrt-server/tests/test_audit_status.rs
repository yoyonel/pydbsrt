use dbsrt_server::handlers::monitoring::{parse_audit_status, pid_runs_audit};
use std::process::Command;

const RUNNING: &str = r#"{"state": "running", "run": "r1", "pid": 4242, "started": "t0", "heartbeat": "t1",
  "seconds": 10.0, "cuts": 5, "eligible": 2827, "resumed_from": 100, "files_done": 1190, "answered": 5950,
  "no_reply": 0, "title_right": 5111, "episodes": 2200, "episode_right": 1793, "unknown_key": 1}"#;

#[test]
fn no_status_file_means_no_audit() {
    assert_eq!(parse_audit_status(None, |_| true).state, "none");
}

#[test]
fn an_unreadable_status_file_is_reported_as_corrupted() {
    assert_eq!(
        parse_audit_status(Some("{not json"), |_| true).state,
        "corrupted"
    );
}

#[test]
fn a_running_audit_with_a_live_audit_pid_stays_running_with_its_counts() {
    let st = parse_audit_status(Some(RUNNING), |pid| pid == 4242);

    assert_eq!(st.state, "running");
    assert_eq!((st.run.as_str(), st.pid, st.cuts), ("r1", Some(4242), 5));
    assert_eq!(
        (st.files_done, st.eligible, st.resumed_from),
        (1190, 2827, 100)
    );
    assert_eq!((st.answered, st.title_right, st.no_reply), (5950, 5111, 0));
    assert_eq!((st.episodes, st.episode_right), (2200, 1793));
}

#[test]
fn a_running_audit_whose_pid_is_gone_or_reused_is_stopped() {
    assert_eq!(
        parse_audit_status(Some(RUNNING), |_| false).state,
        "stopped"
    );
    let no_pid = RUNNING.replace(r#""pid": 4242,"#, "");
    assert_eq!(parse_audit_status(Some(&no_pid), |_| true).state, "stopped");
}

#[test]
fn a_finished_or_failed_audit_keeps_its_state_without_a_live_pid() {
    for state in ["finished", "failed"] {
        let content = RUNNING.replace("running", state);
        assert_eq!(parse_audit_status(Some(&content), |_| false).state, state);
    }
}

#[test]
fn pid_runs_audit_matches_the_audit_script_in_the_process_cmdline_only() {
    let mut audit = Command::new("sh")
        .args(["-c", "sleep 30", "o2switch_audit_remote.py"])
        .spawn()
        .expect("spawn sh");
    let mut other = Command::new("sh")
        .args(["-c", "sleep 30", "something_else.py"])
        .spawn()
        .expect("spawn sh");

    let (is_audit, is_other) = (pid_runs_audit(audit.id()), pid_runs_audit(other.id()));
    let _ = (audit.kill(), other.kill(), audit.wait(), other.wait());

    assert!(is_audit);
    assert!(!is_other);
    assert!(!pid_runs_audit(u32::MAX));
}
