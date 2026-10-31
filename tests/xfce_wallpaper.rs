#![cfg(target_os = "linux")]

use procscape::config::Config;
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::Path,
    process::{Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

const INTERNAL: &str = "/backdrop/screen0/monitoreDP-1/workspace0/last-image";
const EXTERNAL: &str = "/backdrop/screen0/monitorHDMI-1/workspace1/last-image";

struct Desktop {
    temp: tempfile::TempDir,
}

impl Desktop {
    fn new(keys: &[&str]) -> Self {
        let temp = tempfile::Builder::new()
            .prefix("procscape test ")
            .tempdir()
            .unwrap();
        let root = temp.path();
        fs::create_dir_all(root.join("bin")).unwrap();
        fs::create_dir_all(root.join("config/procscape")).unwrap();
        let cfg = Config {
            width: 640,
            height: 360,
            ..Config::default()
        };
        fs::write(
            root.join("config/procscape/config.toml"),
            toml::to_string(&cfg).unwrap(),
        )
        .unwrap();
        fs::write(root.join("keys"), keys.join("\n")).unwrap();
        let script = root.join("bin/xfconf-query");
        fs::write(
            &script,
            r#"#!/usr/bin/python3
import os, pathlib, sys, time
root = pathlib.Path(os.environ['MOCK_DESKTOP'])
args = sys.argv[3:]
if args == ['-l']:
    print((root / 'keys').read_text())
    sys.exit(0)
key = args[1]
value = root / 'properties' / key.lstrip('/')
if len(args) == 2:
    if os.environ.get('MOCK_FAIL_READ'):
        sys.exit('mock read failed')
    print(value.read_text() if value.exists() else '/old wallpaper/' + key.split('/')[3] + '.png')
elif args[2] == '-s':
    if os.environ.get('MOCK_FAIL_SET') or (root / 'fail-set').exists():
        sys.exit('mock write failed')
    if (root / 'delay-set').exists() and '/wallpaper-' in args[3]:
        (root / 'set-started').write_text('1')
        time.sleep(0.5)
    value.parent.mkdir(parents=True, exist_ok=True)
    value.write_text(args[3])
    with (root / 'writes').open('a') as log:
        log.write(args[3] + '\n')
else:
    sys.exit('unexpected arguments')
"#,
        )
        .unwrap();
        fs::set_permissions(script, fs::Permissions::from_mode(0o755)).unwrap();
        let gsettings = root.join("bin/gsettings");
        fs::write(&gsettings, r#"#!/usr/bin/python3
import os, pathlib, sys
root = pathlib.Path(os.environ['MOCK_DESKTOP']) / 'gnome'
root.mkdir(exist_ok=True)
action, schema, key = sys.argv[1:4]
path = root / key
defaults = {'picture-uri': "'file:///old%20light.png'", 'picture-uri-dark': "'file:///old%20dark.png'", 'picture-options': "'zoom'"}
if action == 'get':
    print(path.read_text() if path.exists() else defaults[key])
elif action == 'set':
    path.write_text(sys.argv[4])
else:
    sys.exit('unexpected arguments')
"#).unwrap();
        fs::set_permissions(gsettings, fs::Permissions::from_mode(0o755)).unwrap();
        Self { temp }
    }

    fn root(&self) -> &Path {
        self.temp.path()
    }
    fn command(&self, command: &str, fail: Option<&str>) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_procscape"));
        cmd.arg(command)
            .env("XDG_CONFIG_HOME", self.root().join("config"))
            .env("XDG_DATA_HOME", self.root().join("data"))
            .env("XDG_RUNTIME_DIR", self.root().join("runtime"))
            .env("XDG_CURRENT_DESKTOP", "XFCE")
            .env("MOCK_DESKTOP", self.root())
            .env(
                "PATH",
                format!("{}:/usr/bin:/bin", self.root().join("bin").display()),
            );
        if let Some(fail) = fail {
            cmd.env(fail, "1");
        }
        cmd
    }
    fn call(&self, command: &str, fail: Option<&str>) -> Output {
        self.command(command, fail).output().unwrap()
    }
    fn report(&self) -> procscape::monitor::ServiceReport {
        let output = self.command("status", None).arg("--json").output().unwrap();
        assert!(output.status.success());
        serde_json::from_slice(&output.stdout).unwrap()
    }
    fn wait_for(&self, predicate: impl Fn(&Self) -> bool) {
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(5) {
            if predicate(self) {
                return;
            }
            thread::sleep(Duration::from_millis(50));
        }
        panic!("Timed out waiting for service state: {:?}", self.report());
    }
    fn set_value(&self, key: &str, value: &str) {
        let path = self
            .root()
            .join("properties")
            .join(key.trim_start_matches('/'));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, value).unwrap();
    }
    fn backup(&self) -> std::path::PathBuf {
        self.root().join("data/procscape/original-wallpaper.json")
    }
    fn value(&self, key: &str) -> String {
        fs::read_to_string(
            self.root()
                .join("properties")
                .join(key.trim_start_matches('/')),
        )
        .unwrap()
    }
}

impl Drop for Desktop {
    fn drop(&mut self) {
        if self.root().join("runtime/procscape/daemon.pid").exists() {
            let _ = self.call("stop", None);
            let deadline = Instant::now() + Duration::from_secs(3);
            while self.root().join("runtime/procscape/daemon.pid").exists()
                && Instant::now() < deadline
            {
                thread::sleep(Duration::from_millis(25));
            }
        }
    }
}

fn success(output: Output) {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn named_monitors_and_workspaces_apply_and_restore_their_own_values() {
    let desktop = Desktop::new(&[INTERNAL, EXTERNAL, "/backdrop/screen0/monitor0/image-path"]);
    success(desktop.call("refresh", None));
    let original = fs::read(desktop.backup()).unwrap();
    let active =
        fs::read_to_string(desktop.root().join("data/procscape/active-wallpaper.txt")).unwrap();
    assert!(Path::new(&active).is_file());
    assert_eq!(desktop.value(INTERNAL), active);
    assert_eq!(desktop.value(EXTERNAL), active);
    success(desktop.call("refresh", None));
    assert_eq!(fs::read(desktop.backup()).unwrap(), original);
    success(desktop.call("restore", None));
    assert_eq!(desktop.value(INTERNAL), "/old wallpaper/monitoreDP-1.png");
    assert_eq!(desktop.value(EXTERNAL), "/old wallpaper/monitorHDMI-1.png");
}

#[test]
fn legacy_image_path_is_detected_and_restored() {
    let key = "/backdrop/screen0/monitor0/image-path";
    let desktop = Desktop::new(&[key]);
    success(desktop.call("refresh", None));
    success(desktop.call("restore", None));
    assert_eq!(desktop.value(key), "/old wallpaper/monitor0.png");
}

#[test]
fn missing_properties_do_not_save_an_empty_backup() {
    let desktop = Desktop::new(&[]);
    let output = desktop.call("refresh", None);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("no XFCE wallpaper properties"));
    assert!(!desktop.backup().exists());
}

#[test]
fn failed_reads_do_not_save_a_backup_or_change_wallpaper() {
    let desktop = Desktop::new(&[INTERNAL]);
    let output = desktop.call("refresh", Some("MOCK_FAIL_READ"));
    assert!(!output.status.success());
    assert!(!desktop.backup().exists());
    assert!(!desktop.root().join("properties").exists());
}

#[test]
fn restore_reports_write_failure() {
    let desktop = Desktop::new(&[INTERNAL]);
    success(desktop.call("refresh", None));
    let output = desktop.call("restore", Some("MOCK_FAIL_SET"));
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("mock write failed"));
}

#[test]
fn old_snapshot_format_still_restores() {
    let desktop = Desktop::new(&[]);
    fs::create_dir_all(desktop.backup().parent().unwrap()).unwrap();
    fs::write(
        desktop.backup(),
        r#"{"backend":"xfce","xfce":"/old wallpaper.png"}"#,
    )
    .unwrap();
    success(desktop.call("restore", None));
    assert_eq!(
        desktop.value("/backdrop/screen0/monitor0/workspace0/last-image"),
        "/old wallpaper.png"
    );
}

#[test]
fn remembers_latest_personal_wallpaper_without_overwriting_other_monitors() {
    let desktop = Desktop::new(&[INTERNAL, EXTERNAL]);
    success(desktop.call("refresh", None));
    desktop.set_value(INTERNAL, "/my new wallpapers/sunset.png");
    success(desktop.call("refresh", None));
    success(desktop.call("restore", None));
    assert_eq!(desktop.value(INTERNAL), "/my new wallpapers/sunset.png");
    assert_eq!(desktop.value(EXTERNAL), "/old wallpaper/monitorHDMI-1.png");
}

#[test]
fn restore_pauses_running_daemon_and_survives_service_restart() {
    let desktop = Desktop::new(&[INTERNAL]);
    success(desktop.call("start", None));
    desktop.wait_for(|d| d.report().last_success.is_some());
    assert_eq!(desktop.report().state, "Running");
    success(desktop.call("restore", None));
    assert_eq!(desktop.report().state, "Paused");
    assert_eq!(desktop.value(INTERNAL), "/old wallpaper/monitoreDP-1.png");
    let last_update = desktop.report().last_success.unwrap().collected_at;
    let paused_refresh = desktop.call("refresh", None);
    assert!(!paused_refresh.status.success());
    assert!(String::from_utf8_lossy(&paused_refresh.stderr).contains("Updates are paused"));
    thread::sleep(Duration::from_millis(100));
    assert_eq!(desktop.value(INTERNAL), "/old wallpaper/monitoreDP-1.png");
    assert_eq!(
        desktop.report().last_success.unwrap().collected_at,
        last_update
    );
    success(desktop.call("stop", None));
    desktop.wait_for(|d| d.report().state == "Stopped");
    // systemd restarts __daemon directly; it must preserve the user's pause.
    let mut child = desktop
        .command("__daemon", None)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    desktop.wait_for(|d| d.report().state == "Paused");
    thread::sleep(Duration::from_millis(400));
    assert_eq!(desktop.value(INTERNAL), "/old wallpaper/monitoreDP-1.png");
    desktop.set_value(INTERNAL, "/my next wallpaper.png");
    success(desktop.call("start", None));
    desktop.wait_for(|d| {
        d.report().state == "Running" && d.value(INTERNAL) != "/my next wallpaper.png"
    });
    success(desktop.call("restore", None));
    assert_eq!(desktop.value(INTERNAL), "/my next wallpaper.png");
    success(desktop.call("stop", None));
    child.wait().unwrap();
}

#[test]
fn daemon_status_preserves_last_success_reports_errors_and_clears_on_recovery() {
    let desktop = Desktop::new(&[INTERNAL]);
    success(desktop.call("start", None));
    desktop.wait_for(|d| d.report().last_success.is_some());
    let last_success = desktop.report().last_success.unwrap().collected_at;
    fs::write(desktop.root().join("fail-set"), "").unwrap();
    let failed_refresh = desktop.call("refresh", None);
    assert!(!failed_refresh.status.success());
    assert!(String::from_utf8_lossy(&failed_refresh.stderr).contains("mock write failed"));
    desktop.wait_for(|d| d.report().last_error.is_some());
    let report = desktop.report();
    assert_eq!(report.state, "Running");
    assert_eq!(report.last_success.unwrap().collected_at, last_success);
    assert!(
        report
            .last_error
            .unwrap()
            .message
            .contains("mock write failed")
    );
    fs::remove_file(desktop.root().join("fail-set")).unwrap();
    success(desktop.call("refresh", None));
    desktop.wait_for(|d| d.report().last_error.is_none());
    success(desktop.call("stop", None));
    desktop.wait_for(|d| d.report().state == "Stopped");
}

#[test]
fn gnome_remembers_latest_light_and_dark_wallpapers_independently() {
    let desktop = Desktop::new(&[]);
    let call = |command| {
        desktop
            .command(command, None)
            .env("XDG_CURRENT_DESKTOP", "GNOME")
            .output()
            .unwrap()
    };
    success(call("refresh"));
    let original = fs::read(desktop.backup()).unwrap();
    success(call("refresh"));
    assert_eq!(fs::read(desktop.backup()).unwrap(), original);
    // User selects a new light wallpaper while the dark key still has our image.
    fs::write(
        desktop.root().join("gnome/picture-uri"),
        "'file:///new%20personal.png'",
    )
    .unwrap();
    fs::write(desktop.root().join("gnome/picture-options"), "'scaled'").unwrap();
    success(call("refresh"));
    success(call("restore"));
    assert_eq!(
        fs::read_to_string(desktop.root().join("gnome/picture-uri")).unwrap(),
        "'file:///new%20personal.png'"
    );
    assert_eq!(
        fs::read_to_string(desktop.root().join("gnome/picture-uri-dark")).unwrap(),
        "'file:///old%20dark.png'"
    );
    assert_eq!(
        fs::read_to_string(desktop.root().join("gnome/picture-options")).unwrap(),
        "'scaled'"
    );
}

#[test]
fn restore_waits_for_inflight_application_before_restoring() {
    let desktop = Desktop::new(&[INTERNAL]);
    fs::write(desktop.root().join("delay-set"), "").unwrap();
    success(desktop.call("start", None));
    desktop.wait_for(|d| d.root().join("set-started").exists());
    success(desktop.call("restore", None));
    assert_eq!(desktop.report().state, "Paused");
    assert!(desktop.report().last_success.is_some());
    assert_eq!(desktop.value(INTERNAL), "/old wallpaper/monitoreDP-1.png");
    thread::sleep(Duration::from_millis(600));
    assert_eq!(desktop.value(INTERNAL), "/old wallpaper/monitoreDP-1.png");
}

#[test]
fn concurrent_refreshes_each_acknowledge_their_own_application() {
    let desktop = Desktop::new(&[INTERNAL]);
    success(desktop.call("start", None));
    desktop.wait_for(|d| d.report().last_success.is_some());
    success(desktop.call("refresh", None)); // Wait for initial startup work before measuring.
    // Count actual backend writes; a single shared command file would collapse
    // several requests into one despite all callers reporting success.
    let log = desktop.root().join("writes");
    fs::write(&log, "").unwrap();
    let mut callers = Vec::new();
    for _ in 0..4 {
        callers.push(
            desktop
                .command("refresh", None)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap(),
        );
    }
    for child in callers {
        let output = child.wait_with_output().unwrap();
        let text = String::from_utf8_lossy(&output.stdout);
        assert!(text.contains("refreshed and applied"));
        success(output);
    }
    assert_eq!(
        fs::read_to_string(log)
            .unwrap()
            .lines()
            .filter(|line| !line.contains("-fade-"))
            .count(),
        4
    );
    let report = desktop.report();
    assert_eq!(
        report.active_wallpaper.as_deref(),
        Some(Path::new(&desktop.value(INTERNAL)))
    );
    success(desktop.call("restore", None));
    assert!(desktop.report().active_wallpaper.is_none());
}

#[test]
fn refresh_crossfades_to_final_image_without_replacing_personal_backup() {
    let desktop = Desktop::new(&[INTERNAL]);
    success(desktop.call("refresh", None));
    let original = fs::read(desktop.backup()).unwrap();
    let config_path = desktop.root().join("config/procscape/config.toml");
    let mut cfg: Config = toml::from_str(&fs::read_to_string(&config_path).unwrap()).unwrap();
    cfg.theme = "cyberpunk".into();
    fs::write(config_path, toml::to_string(&cfg).unwrap()).unwrap();
    success(desktop.call("refresh", None));
    let writes = fs::read_to_string(desktop.root().join("writes")).unwrap();
    assert!(writes.lines().any(|line| line.contains("-fade-")));
    let final_image = desktop.report().active_wallpaper.unwrap();
    assert!(!final_image.to_string_lossy().contains("-fade-"));
    assert_eq!(desktop.value(INTERNAL), final_image.to_string_lossy());
    assert_eq!(fs::read(desktop.backup()).unwrap(), original);
    success(desktop.call("restore", None));
    assert_eq!(desktop.value(INTERNAL), "/old wallpaper/monitoreDP-1.png");
}

#[test]
fn fade_can_be_disabled() {
    let desktop = Desktop::new(&[INTERNAL]);
    let path = desktop.root().join("config/procscape/config.toml");
    let mut cfg: Config = toml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    cfg.transition_ms = 0;
    fs::write(path, toml::to_string(&cfg).unwrap()).unwrap();
    success(desktop.call("refresh", None));
    success(desktop.call("refresh", None));
    let writes = fs::read_to_string(desktop.root().join("writes")).unwrap();
    assert_eq!(writes.lines().count(), 2);
    assert!(!writes.contains("-fade-"));
}
