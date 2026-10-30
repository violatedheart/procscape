mod config;
mod control;
mod layout;
mod monitor;
mod render;
mod transition;
mod wallpaper;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use config::Config;
use control::{Reply, Request};
use monitor::{RefreshError, Sampler, ServiceReport, ServiceStatus, collect_snapshot};
use std::os::{
    fd::AsRawFd,
    unix::{fs::PermissionsExt, net::UnixListener},
};
use std::{
    fs,
    path::PathBuf,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

#[derive(Parser)]
#[command(
    name = "procscape",
    version,
    about = "A living Linux process wallpaper"
)]
struct Cli {
    #[command(subcommand)]
    command: CommandKind,
}

#[derive(Subcommand)]
enum CommandKind {
    #[command(name = "__daemon", hide = true)]
    Daemon,
    Start,
    Stop,
    Pause,
    Resume,
    Status {
        #[arg(long)]
        json: bool,
    },
    Refresh,
    Snapshot {
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    Metrics,
    Restore,
    Config,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let paths = Paths::new()?;
    match cli.command {
        CommandKind::Daemon => {
            let result = daemon();
            if let Err(error) = &result {
                record_error(&paths, error);
            }
            result
        }
        CommandKind::Start => start_daemon(&paths),
        CommandKind::Stop => signal_daemon(&paths, Request::Stop),
        CommandKind::Pause => signal_daemon(&paths, Request::Pause),
        CommandKind::Resume => signal_daemon(&paths, Request::Resume),
        CommandKind::Status { json } => status(&paths, json),
        CommandKind::Refresh => {
            let result = refresh_now(&paths);
            if let Err(error) = &result {
                record_error(&paths, error);
            }
            result
        }
        CommandKind::Snapshot { output } => snapshot(&paths, output),
        CommandKind::Metrics => metrics(&paths),
        CommandKind::Restore => {
            if daemon_pid(&paths).is_some() {
                println!(
                    "{}",
                    control::send(&paths.runtime.join("control.sock"), Request::Restore)?
                );
                Ok(())
            } else {
                restore_wallpaper(&paths).map(|message| println!("{message}"))
            }
        }
        CommandKind::Config => {
            paths.ensure()?;
            println!("{}", paths.config.display());
            if !paths.config.exists() {
                fs::write(&paths.config, toml::to_string_pretty(&Config::default())?)?;
            }
            Ok(())
        }
    }
}

struct Paths {
    config: PathBuf,
    data: PathBuf,
    runtime: PathBuf,
    wallpaper: PathBuf,
}
impl Paths {
    fn new() -> Result<Self> {
        let config = dirs::config_dir()
            .context("XDG config directory unavailable")?
            .join("procscape");
        let data = dirs::data_dir()
            .context("XDG data directory unavailable")?
            .join("procscape");
        let runtime = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join("procscape");
        Ok(Self {
            config: config.join("config.toml"),
            wallpaper: data.join("wallpaper.png"),
            data,
            runtime,
        })
    }
    fn ensure(&self) -> Result<()> {
        fs::create_dir_all(self.data.join("state"))?;
        fs::create_dir_all(&self.runtime)?;
        fs::create_dir_all(self.config.parent().unwrap())?;
        Ok(())
    }
    fn cfg(&self) -> Result<Config> {
        if !self.config.exists() {
            self.ensure()?;
            fs::write(&self.config, toml::to_string_pretty(&Config::default())?)?;
        }
        let mut cfg: Config = toml::from_str(&fs::read_to_string(&self.config)?)?;
        cfg.sanitize();
        Ok(cfg)
    }
    fn pid(&self) -> PathBuf {
        self.runtime.join("daemon.pid")
    }
}

fn start_daemon(p: &Paths) -> Result<()> {
    p.ensure()?;
    // Serialize concurrent starts without holding the desktop mutation lock.
    let startup_lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(p.runtime.join("startup.lock"))?;
    if unsafe { libc::flock(startup_lock.as_raw_fd(), libc::LOCK_EX) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    if daemon_pid(p).is_some() {
        println!(
            "{}",
            control::send(&p.runtime.join("control.sock"), Request::Resume)?
        );
        return Ok(());
    }
    let log = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(p.data.join("daemon.log"))?;
    let mut child = Command::new(std::env::current_exe()?)
        .arg("__daemon")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(log)
        .spawn()
        .context("could not start background service")?;
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = child.try_wait()? {
            let message = fs::read_to_string(p.data.join("last-error.json"))
                .ok()
                .and_then(|raw| serde_json::from_str::<RefreshError>(&raw).ok())
                .map(|e| e.message)
                .unwrap_or_else(|| format!("see {}", p.data.join("daemon.log").display()));
            anyhow::bail!("ProcScape failed to start ({status}): {message}");
        }
        if daemon_pid(p) == Some(child.id() as i32) && p.runtime.join("control.sock").exists() {
            if control::send(&p.runtime.join("control.sock"), Request::Ping).is_ok() {
                control::send(&p.runtime.join("control.sock"), Request::Resume)?;
                println!("ProcScape background service started and is ready.");
                return Ok(());
            }
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!(
                "ProcScape did not become ready within 10 seconds; see {}",
                p.data.join("daemon.log").display()
            );
        }
        thread::sleep(Duration::from_millis(25));
    }
}

fn daemon_pid(p: &Paths) -> Option<i32> {
    let pid: i32 = fs::read_to_string(p.pid()).ok()?.trim().parse().ok()?;
    let command = fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    if PathBuf::from(format!("/proc/{pid}")).exists()
        && command.split(|b| *b == 0).any(|a| a == b"__daemon")
    {
        Some(pid)
    } else {
        None
    }
}
fn is_paused(p: &Paths) -> bool {
    p.data.join("updates-paused").exists() || p.runtime.join("paused").exists()
}

fn set_paused(p: &Paths, paused: bool) -> Result<()> {
    p.ensure()?;
    for path in [p.data.join("updates-paused"), p.runtime.join("paused")] {
        if paused {
            fs::write(path, "")?;
        } else {
            match fs::remove_file(path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
        }
    }
    Ok(())
}

fn signal_daemon(p: &Paths, request: Request) -> Result<()> {
    if daemon_pid(p).is_some() {
        let message = control::send(&p.runtime.join("control.sock"), request)?;
        if matches!(request, Request::Stop) {
            // Wait for shutdown to release the singleton before a reinstall
            // starts a new daemon. This does not require reaping an orphan.
            let lock = fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(p.runtime.join("daemon.lock"))?;
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
                    break;
                }
                let error = std::io::Error::last_os_error();
                if error.kind() != std::io::ErrorKind::WouldBlock {
                    return Err(error.into());
                }
                anyhow::ensure!(
                    Instant::now() < deadline,
                    "daemon did not finish stopping within 5 seconds"
                );
                thread::sleep(Duration::from_millis(10));
            }
        }
        println!("{message}");
    } else {
        if matches!(request, Request::Pause | Request::Resume) {
            p.ensure()?;
            let _lock = wallpaper::lock(&p.data)?;
            set_paused(p, matches!(request, Request::Pause))?;
        }
        println!("ProcScape is not running. Use Start / resume to start it.");
    }
    Ok(())
}

fn restore_wallpaper(p: &Paths) -> Result<String> {
    p.ensure()?;
    let _lock = wallpaper::lock(&p.data)?;
    set_paused(p, true)?;
    let saved = p.data.join("original-wallpaper.json").exists();
    match wallpaper::restore(&p.data) {
        Ok(()) => {
            match fs::remove_file(p.data.join("active-wallpaper.txt")) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
            Ok(if saved {
                "Updates paused. Your saved wallpaper has been restored. Use Start / resume to enable updates again."
            } else { "Updates paused. No saved wallpaper is available yet." }.into())
        }
        Err(error) => {
            record_error(p, &error);
            Err(error)
        }
    }
}

fn report(p: &Paths) -> Result<ServiceReport> {
    let pid = daemon_pid(p);
    let state = if pid.is_none() {
        "Stopped"
    } else if is_paused(p) {
        "Paused"
    } else {
        "Running"
    };
    fn read_optional<T: serde::de::DeserializeOwned>(path: PathBuf) -> Result<Option<T>> {
        match fs::read(path) {
            Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
    Ok(ServiceReport {
        state: state.into(),
        pid,
        last_success: read_optional(p.data.join("last-render.json"))?,
        last_error: read_optional(p.data.join("last-error.json"))?,
        active_wallpaper: fs::read_to_string(p.data.join("active-wallpaper.txt"))
            .ok()
            .map(PathBuf::from)
            .filter(|path| path.is_file()),
    })
}

fn status(p: &Paths, json: bool) -> Result<()> {
    let report = report(p)?;
    if json {
        println!("{}", serde_json::to_string(&report)?);
        return Ok(());
    }
    match report.pid {
        Some(pid) => println!(
            "ProcScape is running (PID {pid}){}.",
            if report.state == "Paused" {
                " and paused"
            } else {
                ""
            }
        ),
        None => println!(
            "ProcScape is stopped{}.",
            if is_paused(p) {
                "; updates are paused until Start / resume"
            } else {
                ""
            }
        ),
    }
    if let Some(info) = report.last_success {
        println!("{}", serde_json::to_string_pretty(&info)?);
    }
    if let Some(error) = report.last_error {
        println!("Last error: {}", error.message);
    }
    Ok(())
}

fn record_error(p: &Paths, error: &anyhow::Error) {
    let entry = RefreshError {
        occurred_at: (unix_nanos() / 1_000_000_000) as u64,
        message: format!("{error:#}"),
    };
    let _ = write_json(&p.data.join("last-error.json"), &entry);
}

fn write_json(path: &std::path::Path, value: &impl serde::Serialize) -> Result<()> {
    let temporary = path.with_extension(format!("json.{}.tmp", std::process::id()));
    fs::write(&temporary, serde_json::to_vec_pretty(value)?)?;
    fs::rename(temporary, path)?;
    Ok(())
}

fn snapshot(p: &Paths, output: Option<PathBuf>) -> Result<()> {
    p.ensure()?;
    let cfg = p.cfg()?;
    let mut sampler = Sampler::default();
    let _ = collect_snapshot(&cfg, Some(&mut sampler))?;
    thread::sleep(Duration::from_millis(180));
    let sample = collect_snapshot(&cfg, Some(&mut sampler))?;
    let target = output.unwrap_or_else(|| p.wallpaper.clone());
    render::generate(&sample, &cfg, &target, &p.data.join("state/layout.json"))?;
    println!(
        "Rendered {} real processes to {}",
        sample.processes.len(),
        target.display()
    );
    Ok(())
}
fn metrics(p: &Paths) -> Result<()> {
    let cfg = p.cfg()?;
    let mut sampler = Sampler::default();
    let _ = collect_snapshot(&cfg, Some(&mut sampler))?;
    thread::sleep(Duration::from_millis(180));
    let sample = collect_snapshot(&cfg, Some(&mut sampler))?;
    println!("{}", serde_json::to_string(&sample)?);
    Ok(())
}
fn refresh_now(p: &Paths) -> Result<()> {
    if daemon_pid(p).is_some() {
        println!(
            "{}",
            control::send(&p.runtime.join("control.sock"), Request::Refresh)?
        );
        return Ok(());
    }
    p.ensure()?;
    let _lock = wallpaper::lock(&p.data)?;
    anyhow::ensure!(
        !is_paused(p),
        "Updates are paused. Use Start / resume before refreshing."
    );
    let mut sampler = Sampler::default();
    let cfg = p.cfg()?;
    let _ = collect_snapshot(&cfg, Some(&mut sampler))?;
    thread::sleep(Duration::from_millis(180));
    update_wallpaper(p, &mut sampler)?;
    println!("Rendered and applied a wallpaper from live processes.");
    Ok(())
}

// Caller holds the wallpaper lock, so restore cannot race an application.
fn update_wallpaper(p: &Paths, sampler: &mut Sampler) -> Result<()> {
    let cfg = p.cfg()?;
    let render_started = Instant::now();
    let snapshot = collect_snapshot(&cfg, Some(sampler))?;
    let target = p.data.join(format!("wallpaper-{}.png", unix_nanos()));
    render::generate(&snapshot, &cfg, &target, &p.data.join("state/layout.json"))?;
    transition::apply(&p.data, &target, &cfg)?;
    cleanup_old_wallpapers(&p.data);
    let info = ServiceStatus {
        processes: snapshot.processes.len(),
        cpu_percent: snapshot.cpu_percent,
        memory_percent: snapshot.memory_percent,
        collected_at: (unix_nanos() / 1_000_000_000) as u64,
        render_ms: render_started.elapsed().as_millis(),
    };
    write_json(&p.data.join("last-render.json"), &info)?;
    match fs::remove_file(p.data.join("last-error.json")) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    Ok(())
}

struct DaemonFiles<'a> {
    paths: &'a Paths,
}
impl Drop for DaemonFiles<'_> {
    fn drop(&mut self) {
        let _ = fs::remove_file(self.paths.pid());
        let _ = fs::remove_file(self.paths.runtime.join("control.sock"));
    }
}

#[cfg(target_os = "linux")]
fn daemon() -> Result<()> {
    let p = Paths::new()?;
    p.ensure()?;
    let singleton = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(p.runtime.join("daemon.lock"))?;
    if unsafe { libc::flock(singleton.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        let error = std::io::Error::last_os_error();
        if error.kind() == std::io::ErrorKind::WouldBlock {
            // A duplicate systemd invocation should not cause a restart loop.
            return Ok(());
        }
        return Err(error.into());
    }
    fs::write(p.pid(), std::process::id().to_string())?;
    let _files = DaemonFiles { paths: &p };
    let socket = p.runtime.join("control.sock");
    if socket.exists() {
        fs::remove_file(&socket)?;
    }
    let listener = UnixListener::bind(&socket)?;
    fs::set_permissions(&socket, fs::Permissions::from_mode(0o600))?;
    listener.set_nonblocking(true)?;
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stop_signal = stop.clone();
    ctrlc::set_handler(move || stop_signal.store(true, std::sync::atomic::Ordering::Relaxed))?;
    let mut cfg = p.cfg()?;
    let mut sampler = Sampler::default();
    let _ = collect_snapshot(&cfg, Some(&mut sampler))?;
    thread::sleep(Duration::from_millis(180));
    let mut last = Instant::now() - Duration::from_secs(cfg.refresh_seconds);
    loop {
        if stop.load(std::sync::atomic::Ordering::Relaxed) {
            break;
        }
        // Each accepted connection is processed exactly once and acknowledged
        // after its operation finishes. Paused daemons still accept controls.
        match listener.accept() {
            Ok((mut stream, _)) => {
                stream.set_read_timeout(Some(Duration::from_secs(2)))?;
                let request = control::read_message::<Request>(&mut stream);
                let mut should_stop = false;
                let result: Result<String> = match request {
                    Ok(Request::Ping) => Ok("ProcScape is ready.".into()),
                    Ok(Request::Pause) => {
                        let _lock = wallpaper::lock(&p.data)?;
                        set_paused(&p, true).map(|_| "Updates paused.".into())
                    }
                    Ok(Request::Resume) => {
                        let _lock = wallpaper::lock(&p.data)?;
                        let was_paused = is_paused(&p);
                        set_paused(&p, false).map(|_| {
                            if was_paused {
                                last = Instant::now() - Duration::from_secs(cfg.refresh_seconds);
                            }
                            "Updates resumed.".into()
                        })
                    }
                    Ok(Request::Restore) => restore_wallpaper(&p),
                    Ok(Request::Refresh) => {
                        if is_paused(&p) {
                            Err(anyhow::anyhow!(
                                "Updates are paused. Use Start / resume before refreshing."
                            ))
                        } else {
                            let _lock = wallpaper::lock(&p.data)?;
                            let result = update_wallpaper(&p, &mut sampler);
                            if let Err(error) = &result {
                                record_error(&p, error);
                            } else if let Ok(updated) = p.cfg() {
                                cfg = updated;
                            }
                            last = Instant::now();
                            result.map(|_| "Wallpaper refreshed and applied.".into())
                        }
                    }
                    Ok(Request::Stop) => {
                        should_stop = true;
                        Ok("ProcScape stopped.".into())
                    }
                    Err(error) => Err(error),
                };
                let reply = match result {
                    Ok(message) => Reply {
                        success: true,
                        message,
                    },
                    Err(error) => Reply {
                        success: false,
                        message: format!("{error:#}"),
                    },
                };
                let _ = control::write_message(&mut stream, &reply);
                if should_stop {
                    break;
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(error) => return Err(error.into()),
        }
        if !is_paused(&p) && last.elapsed() >= Duration::from_secs(cfg.refresh_seconds) {
            let _lock = wallpaper::lock(&p.data)?;
            match update_wallpaper(&p, &mut sampler) {
                Ok(()) => {
                    if let Ok(updated) = p.cfg() {
                        cfg = updated;
                    }
                }
                Err(error) => {
                    record_error(&p, &error);
                    eprintln!("ProcScape refresh failed: {error:#}");
                }
            }
            last = Instant::now();
        }
        thread::sleep(Duration::from_millis(25));
    }
    Ok(())
}
#[cfg(not(target_os = "linux"))]
fn daemon() -> Result<()> {
    anyhow::bail!("ProcScape daemon requires Linux /proc")
}

fn unix_nanos() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|x| x.as_nanos())
        .unwrap_or(0)
}
fn cleanup_old_wallpapers(data: &std::path::Path) {
    let Ok(entries) = fs::read_dir(data) else {
        return;
    };
    let mut files: Vec<_> = entries
        .filter_map(|item| item.ok())
        .filter(|e| {
            e.file_name().to_string_lossy().starts_with("wallpaper-")
                && e.path().extension().and_then(|x| x.to_str()) == Some("png")
        })
        .collect();
    files.sort_by_key(|e| std::cmp::Reverse(e.file_name().to_string_lossy().to_string()));
    for old in files.into_iter().skip(3) {
        let _ = fs::remove_file(old.path());
    }
}

// Internal command is kept out of the public help output.
#[cfg(test)]
mod tests {
    #[test]
    fn cli_has_expected_modes() {
        use clap::Parser;
        let parsed = super::Cli::try_parse_from(["procscape", "snapshot"]);
        assert!(parsed.is_ok());
    }
}
