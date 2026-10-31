#![cfg(target_os = "linux")]

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::Path,
    process::{Command, Output},
};

struct Installer {
    temp: tempfile::TempDir,
}

impl Installer {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let bin = root.join("mock-bin");
        fs::create_dir_all(&bin).unwrap();
        for (name, script) in [
            (
                "cargo",
                r#"#!/bin/sh
printf '%s\n' "$@" > "$MOCK_ROOT/cargo-args"
pwd > "$MOCK_ROOT/cargo-workdir"
printf '%s\n' cargo >> "$MOCK_ROOT/trace"
mkdir -p "$CARGO_INSTALL_ROOT/bin"
"#,
            ),
            (
                "systemctl",
                r#"#!/bin/sh
printf '%s\n' "$*" >> "$MOCK_ROOT/systemctl-args"
printf '%s\n' "$*" >> "$MOCK_ROOT/trace"
if [ "$*" = '--user show-environment' ] && [ "${MOCK_NO_SESSION:-0}" = 1 ]; then exit 1; fi
"#,
            ),
            (
                "pkg-config",
                r#"#!/bin/sh
test "$*" = '--exists gtk4' && test "${MOCK_NO_GTK:-0}" != 1
"#,
            ),
        ] {
            let path = bin.join(name);
            fs::write(&path, script).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        Self { temp }
    }

    fn root(&self) -> &Path {
        self.temp.path()
    }
    fn install_root(&self) -> std::path::PathBuf {
        self.root().join("cargo root")
    }
    fn command(&self) -> Command {
        let mut command = Command::new("sh");
        command
            .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("packaging/install.sh"))
            // The installer must find its sources even when launched elsewhere.
            .current_dir(self.root())
            .env(
                "PATH",
                format!("{}:/usr/bin:/bin", self.root().join("mock-bin").display()),
            )
            .env("CARGO_INSTALL_ROOT", self.install_root())
            .env("MOCK_ROOT", self.root())
            .env("XDG_CONFIG_HOME", self.root().join("config"))
            .env("XDG_DATA_HOME", self.root().join("data"));
        command
    }
    fn read(&self, path: &str) -> String {
        fs::read_to_string(self.root().join(path)).unwrap()
    }
    fn assert_no_install(&self) {
        assert!(!self.root().join("cargo-args").exists());
        assert!(
            !self
                .root()
                .join("config/systemd/user/procscape.service")
                .exists()
        );
    }
}

fn success(output: Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn installer_works_when_cargo_bin_is_missing_from_path() {
    let installer = Installer::new();
    success(installer.command().output().unwrap());
    let install = installer.install_root();
    let service = installer.read("config/systemd/user/procscape.service");
    assert!(service.contains(&format!(
        "ExecStart=\"{}/bin/procscape\" __daemon",
        install.display()
    )));
    let desktop = installer.read("data/applications/procscape.desktop");
    assert!(desktop.contains(&format!("Exec=\"{}/bin/procscape-gui\"", install.display())));
    let args = installer.read("cargo-args");
    assert!(args.contains("--locked\n"));
    assert!(args.contains(&format!("--root\n{}\n", install.display())));
    assert_eq!(
        installer.read("cargo-workdir").trim(),
        env!("CARGO_MANIFEST_DIR")
    );
    assert!(!installer.read("systemctl-args").contains("enable"));
}

#[test]
fn start_option_enables_service_after_installation_and_reload() {
    let installer = Installer::new();
    let message = success(installer.command().arg("--start").output().unwrap());
    assert_eq!(
        installer.read("trace"),
        "--user show-environment\ncargo\n--user daemon-reload\n--user enable --now procscape.service\n"
    );
    assert!(message.contains("Open ProcScape from your application menu"));
}

#[test]
fn help_does_not_build_or_change_service_settings() {
    let installer = Installer::new();
    let message = success(installer.command().arg("--help").output().unwrap());
    assert!(message.contains("--start"));
    installer.assert_no_install();
    assert!(!installer.root().join("systemctl-args").exists());
}

#[test]
fn unknown_option_fails_before_installation() {
    let installer = Installer::new();
    let output = installer.command().arg("--strat").output().unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("Unknown option: --strat"));
    installer.assert_no_install();
}

#[test]
fn missing_gtk_has_actionable_error_before_building() {
    let installer = Installer::new();
    let output = installer
        .command()
        .env("MOCK_NO_GTK", "1")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("gtk4-devel (Fedora) or libgtk-4-dev (Debian)")
    );
    installer.assert_no_install();
}

#[test]
fn missing_desktop_session_fails_before_installation() {
    let installer = Installer::new();
    let output = installer
        .command()
        .env("MOCK_NO_SESSION", "1")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("without sudo"));
    installer.assert_no_install();
}

#[test]
fn uninstall_restores_and_stops_using_absolute_binary_path() {
    let installer = Installer::new();
    success(installer.command().output().unwrap());
    let binary = installer.install_root().join("bin/procscape");
    fs::write(
        &binary,
        "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$MOCK_ROOT/controls\"\n",
    )
    .unwrap();
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o755)).unwrap();
    let output = Command::new("sh")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("packaging/uninstall.sh"))
        .env(
            "PATH",
            format!(
                "{}:/usr/bin:/bin",
                installer.root().join("mock-bin").display()
            ),
        )
        .env("CARGO_INSTALL_ROOT", installer.install_root())
        .env("MOCK_ROOT", installer.root())
        .env("XDG_CONFIG_HOME", installer.root().join("config"))
        .env("XDG_DATA_HOME", installer.root().join("data"))
        .output()
        .unwrap();
    success(output);
    assert_eq!(installer.read("controls"), "restore\nstop\n");
    assert!(
        !installer
            .root()
            .join("config/systemd/user/procscape.service")
            .exists()
    );
    assert!(
        !installer
            .root()
            .join("data/applications/procscape.desktop")
            .exists()
    );
    assert!(
        installer
            .read("cargo-args")
            .starts_with("uninstall\n--root\n")
    );
}

#[test]
fn start_option_stops_existing_daemon_before_starting_updated_service() {
    let installer = Installer::new();
    let binary = installer.install_root().join("bin/procscape");
    fs::create_dir_all(binary.parent().unwrap()).unwrap();
    fs::write(
        &binary,
        "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$MOCK_ROOT/trace\"\n",
    )
    .unwrap();
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o755)).unwrap();
    success(installer.command().arg("--start").output().unwrap());
    let trace = installer.read("trace");
    assert!(trace.contains("--user daemon-reload\nstop\n--user enable --now procscape.service\n"));
}
