use crate::config::Config;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{fs, path::Path, process::Command};

#[derive(Serialize, Deserialize, Default)]
struct Original {
    backend: String,
    uri: Option<String>,
    dark_uri: Option<String>,
    picture_options: Option<String>,
    xfce: Option<String>,
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
        let original = if path.exists() {
            serde_json::from_slice(&fs::read(&path)?)?
        } else {
            let o = Original {
                backend: "gnome".into(),
                uri: Some(get_gsetting("picture-uri")?),
                dark_uri: get_gsetting("picture-uri-dark").ok(),
                picture_options: get_gsetting("picture-options").ok(),
                xfce: None,
            };
            fs::write(&path, serde_json::to_vec_pretty(&o)?)?;
            o
        };
        let uri = format!(
            "file://{}",
            image.canonicalize()?.to_string_lossy().replace(' ', "%20")
        );
        if _cfg.span_monitors {
            let _ = set_gsetting("picture-options", "spanned");
        }
        set_gsetting("picture-uri", &uri)?;
        let _ = set_gsetting("picture-uri-dark", &uri);
        let _ = original;
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
fn xfce_apply(data: &Path, image: &Path) -> Result<()> {
    let key = "/backdrop/screen0/monitor0/workspace0/last-image";
    let path = data.join("original-wallpaper.json");
    if !path.exists() {
        let old = Command::new("xfconf-query")
            .args(["-c", "xfce4-desktop", "-p", key])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());
        fs::write(
            &path,
            serde_json::to_vec(&Original {
                backend: "xfce".into(),
                uri: None,
                dark_uri: None,
                picture_options: None,
                xfce: old,
            })?,
        )?;
    }
    let out = Command::new("xfconf-query")
        .args(["-c", "xfce4-desktop", "-p", key, "-s"])
        .arg(image.canonicalize()?)
        .output()?;
    if !out.status.success() {
        anyhow::bail!(
            "xfconf-query failed: {}",
            String::from_utf8_lossy(&out.stderr)
        )
    }
    Ok(())
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
            if let Some(v) = o.xfce {
                let _ = Command::new("xfconf-query")
                    .args([
                        "-c",
                        "xfce4-desktop",
                        "-p",
                        "/backdrop/screen0/monitor0/workspace0/last-image",
                        "-s",
                        &v,
                    ])
                    .status();
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
