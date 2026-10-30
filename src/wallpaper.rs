use crate::config::Config;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, path::Path, process::Command};

#[derive(Serialize, Deserialize, Default)]
struct Original {
    backend: String,
    uri: Option<String>,
    dark_uri: Option<String>,
    picture_options: Option<String>,
    xfce: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    xfce_properties: BTreeMap<String, String>,
}

pub fn apply(data: &Path, image: &Path, _cfg: &Config) -> Result<()> {
    let desktop = std::env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_default()
        .to_lowercase();
    if desktop.contains("xfce") || (desktop.is_empty() && which("xfconf-query")) {
        xfce_apply(data, image)?;
        return Ok(());
    }
    if desktop.contains("gnome")
        || desktop.contains("ubuntu")
        || (desktop.is_empty() && which("gsettings"))
    {
        let path = data.join("original-wallpaper.json");
        let current_uri = get_gsetting("picture-uri")?;
        let current_dark = get_gsetting("picture-uri-dark").ok();
        let mut original = read_original(&path, "gnome")?;
        let personal_light = !managed_image(data, &current_uri);
        let personal_dark = current_dark
            .as_ref()
            .is_some_and(|v| !managed_image(data, v));
        if personal_light {
            original.uri = Some(current_uri);
        }
        if personal_dark {
            original.dark_uri = current_dark;
        }
        if personal_light || personal_dark {
            original.picture_options = get_gsetting("picture-options").ok();
        }
        save_original(&path, &original)?;
        let uri = format!(
            "file://{}",
            image.canonicalize()?.to_string_lossy().replace(' ', "%20")
        );
        if _cfg.span_monitors {
            let _ = set_gsetting("picture-options", "spanned");
        }
        set_gsetting("picture-uri", &uri)?;
        let _ = set_gsetting("picture-uri-dark", &uri);
        return Ok(());
    }
    anyhow::bail!(
        "unsupported desktop session: {}; snapshot PNG was saved",
        if desktop.is_empty() {
            "unknown"
        } else {
            &desktop
        }
    )
}
fn get_gsetting(key: &str) -> Result<String> {
    let out = Command::new("gsettings")
        .args(["get", "org.gnome.desktop.background", key])
        .output()?;
    if !out.status.success() {
        anyhow::bail!("gsettings read failed for {key}")
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}
fn set_gsetting(key: &str, uri: &str) -> Result<()> {
    let escaped = uri.replace('\\', "\\\\").replace('\'', "\\'");
    let value = format!("'{escaped}'");
    let out = Command::new("gsettings")
        .args(["set", "org.gnome.desktop.background", key, &value])
        .output()?;
    if !out.status.success() {
        anyhow::bail!(
            "gsettings could not update {key}: {}",
            String::from_utf8_lossy(&out.stderr)
        )
    }
    Ok(())
}
fn xfce_query(args: &[&str]) -> Result<String> {
    let out = Command::new("xfconf-query")
        .args(["-c", "xfce4-desktop"])
        .args(args)
        .output()
        .context("running xfconf-query")?;
    if !out.status.success() {
        anyhow::bail!(
            "xfconf-query failed ({}): {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8(out.stdout)?
        .trim_end_matches(['\r', '\n'])
        .to_string())
}

fn xfce_apply(data: &Path, image: &Path) -> Result<()> {
    let image = image.canonicalize()?;
    let listing = xfce_query(&["-l"])?;
    let mut keys: Vec<_> = listing
        .lines()
        .filter(|key| key.starts_with("/backdrop/") && key.ends_with("/last-image"))
        .collect();
    // Older XFCE versions use image-path without a workspace component.
    if keys.is_empty() {
        keys = listing
            .lines()
            .filter(|key| key.starts_with("/backdrop/") && key.ends_with("/image-path"))
            .collect();
    }
    if keys.is_empty() {
        anyhow::bail!(
            "no XFCE wallpaper properties found; open Desktop Settings and select a wallpaper first"
        );
    }
    let path = data.join("original-wallpaper.json");
    let mut original = read_original(&path, "xfce")?;
    // Inspect all properties before changing any. Never replace a saved personal
    // wallpaper with one of our own images, including after an upgrade/restart.
    for key in &keys {
        let current = xfce_query(&["-p", key])?;
        if !managed_image(data, &current) {
            original.xfce_properties.insert((*key).to_string(), current);
        }
    }
    save_original(&path, &original)?;
    for key in keys {
        xfce_query(&["-p", key, "-s", &image.to_string_lossy()])?;
    }
    Ok(())
}

fn read_original(path: &Path, backend: &str) -> Result<Original> {
    if path.exists() {
        let original: Original = serde_json::from_slice(&fs::read(path)?)?;
        if original.backend == backend {
            return Ok(original);
        }
    }
    Ok(Original {
        backend: backend.into(),
        ..Original::default()
    })
}

fn save_original(path: &Path, original: &Original) -> Result<()> {
    let temporary = path.with_extension("json.tmp");
    fs::write(&temporary, serde_json::to_vec_pretty(original)?)?;
    fs::rename(temporary, path)?;
    Ok(())
}

fn managed_image(data: &Path, value: &str) -> bool {
    let value = value.trim_matches('\'').trim_matches('"');
    let decoded = value
        .strip_prefix("file://")
        .unwrap_or(value)
        .replace("%20", " ");
    let path = Path::new(&decoded);
    let parent = path
        .parent()
        .map(|p| p.canonicalize().unwrap_or_else(|_| p.to_path_buf()));
    let data = data.canonicalize().unwrap_or_else(|_| data.to_path_buf());
    parent.as_deref() == Some(data.as_path())
        && path.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
            n == "wallpaper.png" || (n.starts_with("wallpaper-") && n.ends_with(".png"))
        })
}

/// Serialize desktop mutations across the daemon and foreground controls.
/// Closing the file releases the advisory lock even if a process exits early.
pub fn lock(data: &Path) -> Result<fs::File> {
    use std::os::fd::AsRawFd;
    let file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(data.join("wallpaper.lock"))?;
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(file)
}

pub fn restore(data: &Path) -> Result<()> {
    let path = data.join("original-wallpaper.json");
    if !path.exists() {
        println!("No saved wallpaper settings are available.");
        return Ok(());
    }
    let o: Original = serde_json::from_slice(&fs::read(path)?)?;
    match o.backend.as_str() {
        "gnome" => {
            if let Some(v) = o.uri {
                set_raw_gsetting("picture-uri", &v)?;
            }
            if let Some(v) = o.dark_uri {
                set_raw_gsetting("picture-uri-dark", &v)?;
            }
            if let Some(v) = o.picture_options {
                set_raw_gsetting("picture-options", &v)?;
            }
        }
        "xfce" => {
            if o.xfce_properties.is_empty() {
                // Keep restoration compatible with snapshots from older releases.
                if let Some(value) = o.xfce {
                    xfce_query(&[
                        "-p",
                        "/backdrop/screen0/monitor0/workspace0/last-image",
                        "-s",
                        &value,
                    ])?;
                }
            } else {
                for (key, value) in o.xfce_properties {
                    xfce_query(&["-p", &key, "-s", &value])?;
                }
            }
        }
        _ => anyhow::bail!("unknown saved wallpaper backend"),
    };
    Ok(())
}
fn set_raw_gsetting(key: &str, value: &str) -> Result<()> {
    let o = Command::new("gsettings")
        .args(["set", "org.gnome.desktop.background", key, value])
        .output()
        .context("running gsettings")?;
    if !o.status.success() {
        anyhow::bail!(
            "gsettings restore failed: {}",
            String::from_utf8_lossy(&o.stderr)
        )
    }
    Ok(())
}
fn which(bin: &str) -> bool {
    Command::new("sh")
        .args(["-c", "command -v \"$1\" >/dev/null 2>&1", "procscape", bin])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}
