mod config;
mod layout;
mod monitor;
mod render;
mod wallpaper;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use config::Config;
use monitor::{Sampler, ServiceStatus, collect_snapshot};
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
    Status,
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
        CommandKind::Daemon => daemon(),
        CommandKind::Start => start_daemon(&paths),
        CommandKind::Stop => signal_daemon(&paths, "stop"),
        CommandKind::Pause => signal_daemon(&paths, "pause"),
        CommandKind::Resume => signal_daemon(&paths, "resume"),
        CommandKind::Status => status(&paths),
        CommandKind::Refresh => refresh_now(&paths),
        CommandKind::Snapshot { output } => snapshot(&paths, output),
        CommandKind::Metrics => metrics(&paths),
        CommandKind::Restore => wallpaper::restore(&paths.data),
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
    if daemon_pid(p).is_some() {
        write_signal(p, "resume")?;
        let _ = fs::remove_file(p.runtime.join("paused"));
        println!("ProcScape is already running; updates resumed.");
        return Ok(());
    }
    let exe = std::env::current_exe()?;
    Command::new(exe)
        .arg("__daemon")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("could not start background service")?;
    thread::sleep(Duration::from_millis(180));
    println!("ProcScape background service started.");
    Ok(())
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
fn write_signal(p: &Paths, s: &str) -> Result<()> {
    p.ensure()?;
    fs::write(p.runtime.join("command"), s)?;
    Ok(())
}
fn signal_daemon(p: &Paths, s: &str) -> Result<()> {
    if daemon_pid(p).is_none() {
        println!("ProcScape is not running.");
        return Ok(());
    }
    write_signal(p, s)?;
    println!("{} requested.", s);
    Ok(())
}
fn status(p: &Paths) -> Result<()> {
    match daemon_pid(p) {
        Some(pid) => println!(
            "ProcScape is running (PID {pid}){}.",
            if p.runtime.join("paused").exists() {
                " and paused"
            } else {
                ""
            }
        ),
        None => println!("ProcScape is stopped."),
    }
    if p.data.join("last-render.json").exists() {
        println!("{}", fs::read_to_string(p.data.join("last-render.json"))?);
    }
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
        write_signal(p, "refresh")?;
        println!("Refresh requested from the running service.");
        return Ok(());
    }
    p.ensure()?;
    let cfg = p.cfg()?;
    let mut sampler = Sampler::default();
    let _ = collect_snapshot(&cfg, Some(&mut sampler))?;
    thread::sleep(Duration::from_millis(180));
    let snapshot = collect_snapshot(&cfg, Some(&mut sampler))?;
    let target = p.data.join(format!("wallpaper-{}.png", unix_nanos()));
    render::generate(&snapshot, &cfg, &target, &p.data.join("state/layout.json"))?;
    wallpaper::apply(&p.data, &target, &cfg)?;
    fs::write(
        p.data.join("active-wallpaper.txt"),
        target.to_string_lossy().as_bytes(),
    )?;
    cleanup_old_wallpapers(&p.data);
    println!(
        "Rendered and applied a wallpaper from {} live processes.",
        snapshot.processes.len()
    );
    Ok(())
}

#[cfg(target_os = "linux")]
fn daemon() -> Result<()> {
    let p = Paths::new()?;
    p.ensure()?;
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(p.pid())
    {
        Ok(mut f) => {
            use std::io::Write;
            write!(f, "{}", std::process::id())?;
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            if daemon_pid(&p).is_some() {
                return Ok(());
            }
            let _ = fs::remove_file(p.pid());
            let mut f = fs::File::create(p.pid())?;
            use std::io::Write;
            write!(f, "{}", std::process::id())?;
        }
        Err(e) => return Err(e.into()),
    }
    let _ = fs::remove_file(p.runtime.join("command"));
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    {
        let s = stop.clone();
        ctrlc::set_handler(move || s.store(true, std::sync::atomic::Ordering::Relaxed))
            .context("signal handler")?;
    }
    let mut cfg = p.cfg()?;
    let mut sampler = Sampler::default();
    let mut last = Instant::now() - Duration::from_secs(120);
    let _ = collect_snapshot(&cfg, Some(&mut sampler))?;
    thread::sleep(Duration::from_millis(180));
    loop {
        if stop.load(std::sync::atomic::Ordering::Relaxed) {
            break;
        }
        if let Ok(command) = fs::read_to_string(p.runtime.join("command")) {
            let _ = fs::remove_file(p.runtime.join("command"));
            match command.trim() {
                "stop" => break,
                "pause" => {
                    fs::write(p.runtime.join("paused"), "")?;
                }
                "resume" => {
                    let _ = fs::remove_file(p.runtime.join("paused"));
                }
                "refresh" => {
                    cfg = p.cfg()?;
                    last = Instant::now() - Duration::from_secs(cfg.refresh_seconds.max(1));
                }
                _ => {}
            }
        }
        let paused = p.runtime.join("paused").exists();
        if !paused && last.elapsed() >= Duration::from_secs(cfg.refresh_seconds.max(1)) {
            cfg = p.cfg()?;
            let target = p.data.join(format!("wallpaper-{}.png", unix_nanos()));
            let render_started = Instant::now();
            match collect_snapshot(&cfg, Some(&mut sampler)).and_then(|s| {
                render::generate(&s, &cfg, &target, &p.data.join("state/layout.json"))?;
                wallpaper::apply(&p.data, &target, &cfg)?;
                fs::write(
                    p.data.join("active-wallpaper.txt"),
                    target.to_string_lossy().as_bytes(),
                )?;
                cleanup_old_wallpapers(&p.data);
                let info = ServiceStatus {
                    processes: s.processes.len(),
                    cpu_percent: s.cpu_percent,
                    memory_percent: s.memory_percent,
                    collected_at: s.collected_at,
                    render_ms: render_started.elapsed().as_millis(),
                };
                fs::write(
                    p.data.join("last-render.json"),
                    serde_json::to_vec_pretty(&info)?,
                )?;
                Ok(s)
            }) {
                Ok(_) => {}
                Err(e) => eprintln!("ProcScape refresh failed: {e:#}"),
            }
            last = Instant::now();
        }
        thread::sleep(Duration::from_millis(250));
    }
    let _ = fs::remove_file(p.pid());
    let _ = fs::remove_file(p.runtime.join("paused"));
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
