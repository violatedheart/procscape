use crate::config::{Config, RunMode};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fs, path::Path};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Process {
    pub key: String,
    pub name: String,
    pub pid: u32,
    pub ppid: u32,
    pub uid: u32,
    pub exe: String,
    pub lifetime_seconds: f64,
    pub cpu_percent: f32,
    pub rss_kib: u64,
    pub weight: f32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Snapshot {
    pub processes: Vec<Process>,
    pub cpu_percent: f32,
    pub memory_percent: f32,
    pub collected_at: u64,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ServiceStatus {
    pub processes: usize,
    pub cpu_percent: f32,
    pub memory_percent: f32,
    pub collected_at: u64,
    pub render_ms: u128,
}
#[derive(Default)]
pub struct Sampler {
    previous_total: u64,
    previous: HashMap<u32, (u64, u64)>,
    smooth: HashMap<String, f32>,
}

fn cpu_total() -> Result<(u64, u64)> {
    let line = fs::read_to_string("/proc/stat")?;
    let mut it = line
        .lines()
        .next()
        .context("missing aggregate CPU counters")?
        .split_whitespace()
        .skip(1)
        .map(|v| v.parse::<u64>().unwrap_or(0));
    let vals: Vec<u64> = it.by_ref().collect();
    let total = vals.iter().sum();
    let idle = vals.get(3).copied().unwrap_or(0) + vals.get(4).copied().unwrap_or(0);
    Ok((total, idle))
}

fn parse_stat(s: &str) -> Option<(String, u32, u64, u64, u64, i64)> {
    let close = s.rfind(')')?;
    let name = s.get(s.find('(')? + 1..close)?.to_string();
    let f: Vec<&str> = s[close + 1..].split_whitespace().collect();
    // Fields after comm begin at field 3 (state).
    Some((
        name,
        f.get(1)?.parse().ok()?,
        f.get(11)?.parse().ok()?,
        f.get(12)?.parse().ok()?,
        f.get(19)?.parse().ok()?,
        f.get(21)?.parse().ok()?,
    ))
}

pub fn collect_snapshot(cfg: &Config, sampler: Option<&mut Sampler>) -> Result<Snapshot> {
    let (total, _idle) = cpu_total()?;
    let (mem_total, mem_avail) = memory()?;
    let num_cpus = std::thread::available_parallelism()
        .map(|x| x.get() as f64)
        .unwrap_or(1.0);
    let ticks_per_second = unsafe { libc::sysconf(libc::_SC_CLK_TCK) }.max(1) as f64;
    let mut processes = Vec::new();
    let mut grouped: HashMap<String, Process> = HashMap::new();
    let mut sampler = sampler;
    for ent in fs::read_dir("/proc")? {
        let ent = match ent {
            Ok(v) => v,
            Err(_) => continue,
        };
        let filename = ent.file_name();
        let Some(pid) = filename.to_string_lossy().parse::<u32>().ok() else {
            continue;
        };
        let dir = ent.path();
        let stat = match fs::read_to_string(dir.join("stat")) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let Some((comm, ppid, utime, stime, start_ticks, rss_pages)) = parse_stat(&stat) else {
            continue;
        };
        let status = fs::read_to_string(dir.join("status")).unwrap_or_default();
        let state = stat
            .rsplit_once(')')
            .and_then(|x| x.1.split_whitespace().next())
            .unwrap_or("?");
        if cfg.exclude_zombies && state == "Z" {
            continue;
        }
        let uid = status
            .lines()
            .find(|x| x.starts_with("Uid:"))
            .and_then(|x| x.split_whitespace().nth(1))
            .and_then(|x| x.parse().ok())
            .unwrap_or(u32::MAX);
        if (!cfg.include_system || cfg.show_user_only) && uid != unsafe { libc::getuid() } {
            continue;
        }
        let exe = fs::read_link(dir.join("exe"))
            .map(|x| x.to_string_lossy().into_owned())
            .unwrap_or_default();
        let cmd = fs::read(dir.join("cmdline")).unwrap_or_default();
        if cfg.exclude_kernel_threads && cmd.is_empty() {
            continue;
        }
        let name = process_name(comm, &exe);
        if !cfg.search_filter.trim().is_empty()
            && !name
                .to_lowercase()
                .contains(&cfg.search_filter.trim().to_lowercase())
        {
            continue;
        }
        if cfg
            .exclude_patterns
            .iter()
            .any(|p| pattern_matches(p, &name))
        {
            continue;
        }
        let rss_kib = if rss_pages > 0 {
            rss_pages as u64 * page_size_kib()
        } else {
            status
                .lines()
                .find(|x| x.starts_with("VmRSS:"))
                .and_then(|x| x.split_whitespace().nth(1))
                .and_then(|x| x.parse().ok())
                .unwrap_or(0)
        };
        let ticks = utime + stime;
        let (cpu_raw, lifetime) = if let Some(sm) = sampler.as_deref_mut() {
            let prior = sm.previous.insert(pid, (ticks, start_ticks));
            let dt = total.saturating_sub(sm.previous_total).max(1);
            let delta = prior
                .filter(|x| x.1 == start_ticks)
                .map(|x| ticks.saturating_sub(x.0))
                .unwrap_or(0);
            (
                cpu_percent_from_delta(delta, dt, num_cpus),
                start_ticks as f64 / ticks_per_second,
            )
        } else {
            (0.0, start_ticks as f64 / ticks_per_second)
        };
        let key = if cfg.group_by_name {
            name.to_lowercase()
        } else {
            format!("{}:{pid}", name.to_lowercase())
        };
        let pinned = cfg
            .pinned_processes
            .iter()
            .any(|x| x.eq_ignore_ascii_case(&name));
        if !pinned
            && ((cfg.min_memory_kib > 0 && rss_kib < cfg.min_memory_kib)
                || (cfg.min_cpu_percent > 0.0 && cpu_raw < cfg.min_cpu_percent))
        {
            continue;
        }
        let p = Process {
            key: key.clone(),
            name,
            pid,
            ppid,
            uid,
            exe,
            lifetime_seconds: lifetime,
            cpu_percent: cpu_raw,
            rss_kib,
            weight: 0.0,
        };
        if cfg.group_by_name {
            merge_group(&mut grouped, key, p);
        } else {
            processes.push(p);
        }
    }
    if let Some(sm) = sampler.as_deref_mut() {
        sm.previous
            .retain(|pid, _| Path::new(&format!("/proc/{pid}")).exists());
        sm.previous_total = total;
    }
    if cfg.group_by_name {
        processes.extend(grouped.into_values());
    }
    let cpu_percent = read_cpu_percent().unwrap_or(0.0);
    let memory_percent = if mem_total > 0 {
        mem_total.saturating_sub(mem_avail) as f32 / mem_total as f32 * 100.0
    } else {
        0.0
    };
    assign_weights(&mut processes, cfg, sampler.as_deref_mut());
    if let Some(sm) = sampler.as_deref_mut() {
        let live: std::collections::HashSet<&str> =
            processes.iter().map(|p| p.key.as_str()).collect();
        sm.smooth.retain(|k, _| live.contains(k.as_str()));
    }
    Ok(Snapshot {
        processes,
        cpu_percent,
        memory_percent,
        collected_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|x| x.as_secs())
            .unwrap_or(0),
    })
}

fn assign_weights(ps: &mut [Process], cfg: &Config, mut sampler: Option<&mut Sampler>) {
    let max_cpu = ps
        .iter()
        .map(|x| x.cpu_percent)
        .fold(0.0_f32, f32::max)
        .max(0.01);
    let max_mem = ps
        .iter()
        .map(|x| x.rss_kib as f32)
        .fold(0.0_f32, f32::max)
        .max(1.0);
    let sum = (cfg.cpu_weight.max(0.0) + cfg.memory_weight.max(0.0)).max(0.001);
    let cpu_weight = cfg.cpu_weight.max(0.0) / sum;
    let memory_weight = cfg.memory_weight.max(0.0) / sum;
    for p in ps {
        let c = (1.0 + (p.cpu_percent.max(0.0) / max_cpu * 100.0)).ln() / 101.0_f32.ln();
        let m = (1.0 + (p.rss_kib as f32 / max_mem * 100.0)).ln() / 101.0_f32.ln();
        let mut raw = match cfg.mode {
            RunMode::Cpu => c,
            RunMode::Memory => m,
            RunMode::Hybrid => c * cpu_weight + m * memory_weight,
        };
        if cfg
            .pinned_processes
            .iter()
            .any(|x| x.eq_ignore_ascii_case(&p.name))
        {
            raw = raw.max(0.55);
        }
        let s = sampler
            .as_deref_mut()
            .map(|sm| {
                let prev = sm.smooth.get(&p.key).copied().unwrap_or(raw);
                let next = prev * (1.0 - cfg.smoothing.clamp(0.0, 0.99))
                    + raw * cfg.smoothing.clamp(0.01, 1.0);
                sm.smooth.insert(p.key.clone(), next);
                next
            })
            .unwrap_or(raw);
        p.weight = s.clamp(0.003, 1.0);
    }
}
fn memory() -> Result<(u64, u64)> {
    let t = fs::read_to_string("/proc/meminfo")?;
    let val = |key: &str| {
        t.lines()
            .find(|l| l.starts_with(key))
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|x| x.parse().ok())
            .unwrap_or(0)
    };
    Ok((val("MemTotal:"), val("MemAvailable:")))
}
fn read_cpu_percent() -> Result<f32> {
    let (t, i) = cpu_total()?;
    thread_sleep_20ms();
    let (t2, i2) = cpu_total()?;
    let d = t2.saturating_sub(t).max(1);
    Ok((d.saturating_sub(i2.saturating_sub(i)) as f32 / d as f32 * 100.0).clamp(0.0, 100.0))
}
fn thread_sleep_20ms() {
    std::thread::sleep(std::time::Duration::from_millis(20));
}
fn page_size_kib() -> u64 {
    unsafe { libc::sysconf(libc::_SC_PAGESIZE) as u64 / 1024 }.max(1)
}
fn process_name(comm: String, exe: &str) -> String {
    Path::new(exe)
        .file_name()
        .map(|x| x.to_string_lossy().into_owned())
        .filter(|x| !x.is_empty())
        .unwrap_or(comm)
}
fn cpu_percent_from_delta(process_ticks: u64, total_ticks: u64, logical_cpus: f64) -> f32 {
    if total_ticks == 0 || !logical_cpus.is_finite() || logical_cpus <= 0.0 {
        return 0.0;
    }
    (process_ticks as f64 / total_ticks as f64 * 100.0 * logical_cpus) as f32
}
fn merge_group(groups: &mut HashMap<String, Process>, key: String, p: Process) {
    groups
        .entry(key)
        .and_modify(|a| {
            let is_leader = p.cpu_percent > a.cpu_percent;
            a.cpu_percent += p.cpu_percent;
            a.rss_kib = a.rss_kib.saturating_add(p.rss_kib);
            if is_leader {
                a.pid = p.pid;
                a.exe = p.exe.clone();
                a.ppid = p.ppid;
                a.uid = p.uid;
                a.lifetime_seconds = p.lifetime_seconds;
            }
        })
        .or_insert(p);
}
fn pattern_matches(pattern: &str, value: &str) -> bool {
    let p = pattern.to_lowercase().chars().collect::<Vec<_>>();
    let v = value.to_lowercase().chars().collect::<Vec<_>>();
    let mut dp = vec![vec![false; v.len() + 1]; p.len() + 1];
    dp[p.len()][v.len()] = true;
    for i in (0..p.len()).rev() {
        for j in (0..=v.len()).rev() {
            dp[i][j] = if p[i] == '*' {
                dp[i + 1][j] || (j < v.len() && dp[i][j + 1])
            } else {
                j < v.len() && (p[i] == '?' || p[i] == v[j]) && dp[i + 1][j + 1]
            };
        }
    }
    dp[0][0]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stat_handles_parentheses_in_name() {
        let v =
            "42 (python (worker)) S 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20 21 22 23";
        assert_eq!(parse_stat(v).unwrap().0, "python (worker)");
    }
    #[test]
    fn filter_patterns() {
        assert!(pattern_matches("py*", "python3"));
        assert!(pattern_matches("*thon*", "python3"));
        assert!(pattern_matches("py?hon3", "python3"));
        assert!(!pattern_matches("bash", "python3"));
    }
    #[test]
    fn duplicate_names_aggregate_metrics() {
        let p = |pid, cpu, rss| Process {
            key: "python3".into(),
            name: "python3".into(),
            pid,
            ppid: 1,
            uid: 1000,
            exe: format!("/python/{pid}"),
            lifetime_seconds: 1.0,
            cpu_percent: cpu,
            rss_kib: rss,
            weight: 0.0,
        };
        let mut m = HashMap::new();
        merge_group(&mut m, "python3".into(), p(1, 2.0, 100));
        merge_group(&mut m, "python3".into(), p(2, 5.0, 250));
        let a = &m["python3"];
        assert_eq!(a.cpu_percent, 7.0);
        assert_eq!(a.rss_kib, 350);
        assert_eq!(a.pid, 2);
    }
    #[test]
    fn weights_are_ranked_and_bounded() {
        let mut ps = vec![
            Process {
                key: "small".into(),
                name: "small".into(),
                pid: 1,
                ppid: 0,
                uid: 1,
                exe: String::new(),
                lifetime_seconds: 1.0,
                cpu_percent: 0.1,
                rss_kib: 100,
                weight: 0.0,
            },
            Process {
                key: "large".into(),
                name: "large".into(),
                pid: 2,
                ppid: 0,
                uid: 1,
                exe: String::new(),
                lifetime_seconds: 1.0,
                cpu_percent: 25.0,
                rss_kib: 10000,
                weight: 0.0,
            },
        ];
        assign_weights(&mut ps, &Config::default(), None);
        assert!(ps[1].weight > ps[0].weight);
        assert!(ps.iter().all(|x| (0.0..=1.0).contains(&x.weight)));
    }

    #[test]
    fn cpu_delta_reports_single_core_equivalent_percent() {
        assert_eq!(cpu_percent_from_delta(10, 100, 8.0), 80.0);
        assert_eq!(cpu_percent_from_delta(100, 100, 8.0), 800.0);
        assert_eq!(cpu_percent_from_delta(10, 0, 8.0), 0.0);
    }

    #[test]
    fn process_name_prefers_executable_and_falls_back_to_comm() {
        assert_eq!(
            process_name("mutable title".into(), "/usr/bin/bash"),
            "bash"
        );
        assert_eq!(process_name("init (worker)".into(), ""), "init (worker)");
    }
}
