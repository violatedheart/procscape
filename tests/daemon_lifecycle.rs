#![cfg(target_os = "linux")]

use procscape::config::Config;
use std::{
    fs,
    path::Path,
    process::{Command, Output},
    thread,
    time::{Duration, Instant},
};

fn call(bin: &Path, args: &[&str], root: &Path) -> Output {
    Command::new(bin)
        .args(args)
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("XDG_DATA_HOME", root.join("data"))
        .env("XDG_RUNTIME_DIR", root.join("runtime"))
        .env("XDG_CURRENT_DESKTOP", "ProcScapeTest")
        .output()
        .unwrap()
}

fn wait_for(bin: &Path, root: &Path, expected: &str, timeout: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        let result = call(bin, &["status"], root);
        if String::from_utf8_lossy(&result.stdout).contains(expected) {
            return true;
        }
        thread::sleep(Duration::from_millis(100));
    }
    false
}

#[test]
fn daemon_can_start_pause_resume_and_stop() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    fs::create_dir_all(root.join("runtime")).unwrap();
    let config_dir = root.join("config/procscape");
    fs::create_dir_all(&config_dir).unwrap();
    let mut cfg = Config::default();
    cfg.refresh_seconds = 60;
    cfg.width = 640;
    cfg.height = 360;
    fs::write(
        config_dir.join("config.toml"),
        toml::to_string(&cfg).unwrap(),
    )
    .unwrap();
    let bin = Path::new(env!("CARGO_BIN_EXE_procscape"));

    let started = call(bin, &["start"], root);
    assert!(
        started.status.success(),
        "{}",
        String::from_utf8_lossy(&started.stderr)
    );
    assert!(
        wait_for(bin, root, "running (PID", Duration::from_secs(5)),
        "daemon failed to start"
    );
    assert!(call(bin, &["pause"], root).status.success());
    assert!(
        wait_for(bin, root, "and paused", Duration::from_secs(5)),
        "daemon failed to pause"
    );
    assert!(call(bin, &["resume"], root).status.success());
    assert!(
        wait_for(bin, root, "running (PID", Duration::from_secs(5)),
        "daemon failed to resume"
    );
    assert!(call(bin, &["stop"], root).status.success());
    assert!(
        wait_for(bin, root, "stopped", Duration::from_secs(10)),
        "daemon failed to stop"
    );
}

#[test]
fn invalid_configuration_fails_start_instead_of_reporting_success() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    fs::create_dir_all(root.join("config/procscape")).unwrap();
    fs::write(
        root.join("config/procscape/config.toml"),
        "not valid = [toml",
    )
    .unwrap();
    let bin = Path::new(env!("CARGO_BIN_EXE_procscape"));
    let output = call(bin, &["start"], root);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("failed to start"));
    assert!(!root.join("runtime/procscape/control.sock").exists());
    assert!(!root.join("runtime/procscape/daemon.pid").exists());
}
