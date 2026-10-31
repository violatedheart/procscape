use crate::{
    config::Config,
    layout::{self, Item, LayoutMemory},
    monitor::Snapshot,
};
use ab_glyph::{Font, FontArc, PxScale, ScaleFont, point};
use anyhow::{Context, Result};
use image::{Rgba, RgbaImage};
use serde::Serialize;
use std::{collections::HashMap, fs, path::Path};

#[derive(Serialize)]
struct RenderInfo {
    width: u32,
    height: u32,
    placed: usize,
    skipped: usize,
    elapsed_ms: u128,
}

pub fn generate(snapshot: &Snapshot, cfg: &Config, out: &Path, state_path: &Path) -> Result<()> {
    let started = std::time::Instant::now();
    let (width, height) = dimensions(cfg);
    let font = load_font(&cfg.font_path)?;
    let mut state = load_layout(state_path);
    let palette = palette(cfg);
    let background = if cfg.theme == "custom" {
        parse_color(&cfg.custom_background)
    } else {
        theme_background(&cfg.theme)
    };
    let max_weight = snapshot
        .processes
        .iter()
        .map(|p| p.weight)
        .fold(0.0_f32, f32::max)
        .max(0.001);
    let mut ranked = snapshot.processes.clone();
    ranked.sort_by(|a, b| {
        b.weight
            .total_cmp(&a.weight)
            .then_with(|| a.key.cmp(&b.key))
    });
    ranked.truncate(((cfg.max_words as f32 * cfg.density.clamp(0.1, 1.0)).round() as usize).max(1));
    let mut words = Vec::new();
    let mut by_key = HashMap::new();
    let mut sizes = HashMap::new();
    let mut seed = 1_u64;
    for p in ranked {
        let fraction = (p.weight / max_weight).clamp(0.0, 1.0);
        let target_size =
            cfg.min_font_size + (cfg.max_font_size - cfg.min_font_size) * fraction.sqrt();
        let font_size = smoothed_size(state.font_sizes.get(&p.key).copied(), target_size, cfg);
        let (tw, th) = text_bounds(&font, &p.name, font_size);
        let rotate = state.positions.get(&p.key).map(|x| x.2).unwrap_or_else(|| {
            seed = hash(&p.key);
            (seed as f64 / u64::MAX as f64) < cfg.rotation_probability as f64
        });
        let spacing = cfg.spacing;
        let item = Item {
            key: p.key.clone(),
            width: tw + spacing * 2,
            height: th + spacing * 2,
            rotated: rotate,
        };
        sizes.insert(p.key.clone(), font_size);
        by_key.insert(p.key.clone(), p);
        words.push(item);
    }
    let placements = layout::place(
        &words,
        width,
        height,
        cfg.margins,
        cfg.spacing,
        &state,
        cfg.layout_stability.clamp(0.0, 0.95),
    );
    let mut canvas = RgbaImage::from_pixel(
        width,
        height,
        Rgba([background.0, background.1, background.2, 255]),
    );
    let mut next = HashMap::new();
    for item in &words {
        let Some(place) = placements.get(&item.key) else {
            continue;
        };
        let p = &by_key[&item.key];
        let size = sizes[&item.key];
        let is_pinned = cfg
            .pinned_processes
            .iter()
            .any(|x| x.eq_ignore_ascii_case(&p.name));
        let color = if is_pinned {
            (249, 226, 175)
        } else if cfg.theme == "terminal-green" {
            let g = (105 + (hash(&p.key) % 145) as u8).min(240);
            (63, g, 95)
        } else {
            palette[(hash(&p.key) as usize) % palette.len()]
        };
        let x = place.rect.x + cfg.spacing;
        let y = place.rect.y + cfg.spacing;
        draw_text(
            &mut canvas,
            &font,
            &p.name,
            size,
            x,
            y,
            item.width - cfg.spacing * 2,
            item.height - cfg.spacing * 2,
            place.rotated,
            color,
        );
        next.insert(
            item.key.clone(),
            (
                (place.rect.x as f32 + place.rect.w as f32 / 2.0) / width as f32,
                (place.rect.y as f32 + place.rect.h as f32 / 2.0) / height as f32,
                place.rotated,
            ),
        );
    }
    if let Some(parent) = out.parent() {
        fs::create_dir_all(parent)?;
    }
    canvas
        .save(out)
        .with_context(|| format!("saving PNG {}", out.display()))?;
    state.width = width;
    state.height = height;
    state.font_sizes = sizes
        .into_iter()
        .filter(|(key, _)| next.contains_key(key))
        .collect();
    state.positions = next;
    if let Some(parent) = state_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(state_path, serde_json::to_vec(&state)?)?;
    let info = RenderInfo {
        width,
        height,
        placed: placements.len(),
        skipped: words.len().saturating_sub(placements.len()),
        elapsed_ms: started.elapsed().as_millis(),
    };
    if let Some(parent) = state_path.parent() {
        fs::write(
            parent.join("render-info.json"),
            serde_json::to_vec_pretty(&info)?,
        )?;
    }
    Ok(())
}
fn smoothed_size(previous: Option<f32>, target: f32, cfg: &Config) -> f32 {
    let target = target.clamp(cfg.min_font_size, cfg.max_font_size);
    let Some(previous) = previous.filter(|v| v.is_finite()) else {
        return target;
    };
    let previous = previous.clamp(cfg.min_font_size, cfg.max_font_size);
    if (target - previous).abs() < 0.5 {
        return previous;
    }
    previous + (target - previous) * cfg.smoothing.clamp(0.01, 1.0)
}

fn dimensions(c: &Config) -> (u32, u32) {
    if c.width > 0 && c.height > 0 {
        return (c.width.min(7680), c.height.min(4320));
    }
    if let Some((w, h)) = xrandr_dimensions(c.span_monitors) {
        return (w.min(7680), h.min(4320));
    }
    (1920, 1080)
}
fn xrandr_dimensions(span: bool) -> Option<(u32, u32)> {
    let out = std::process::Command::new("xrandr")
        .arg("--current")
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout);
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    let mut found = false;
    for line in s.lines().filter(|l| l.contains(" connected")) {
        for token in line.split_whitespace() {
            let Some((w, h, x, y)) = parse_geometry(token) else {
                continue;
            };
            if !span {
                return Some((w as u32, h as u32));
            }
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x + w);
            max_y = max_y.max(y + h);
            found = true;
            break;
        }
    }
    if found {
        Some(((max_x - min_x) as u32, (max_y - min_y) as u32))
    } else {
        None
    }
}
fn parse_geometry(s: &str) -> Option<(i32, i32, i32, i32)> {
    let (w, rest) = s.split_once('x')?;
    let (h, offsets) = rest
        .char_indices()
        .skip(1)
        .find(|(_, c)| *c == '+' || *c == '-')
        .map(|(i, _)| (&rest[..i], &rest[i..]))?;
    let split = offsets
        .char_indices()
        .skip(1)
        .find(|(_, c)| *c == '+' || *c == '-')
        .map(|(i, _)| i)?;
    let (w, h, x, y) = (
        w.parse().ok()?,
        h.parse().ok()?,
        offsets[..split].parse().ok()?,
        offsets[split..].parse().ok()?,
    );
    if w > 0 && h > 0 {
        Some((w, h, x, y))
    } else {
        None
    }
}
fn load_font(path: &str) -> Result<FontArc> {
    // Existing configs contain Debian's font path. Resolve that default on
    // other distributions too, while keeping explicit custom paths strict.
    let path = if path == crate::config::DEFAULT_FONT_PATH && !Path::new(path).is_file() {
        [
            "/usr/share/fonts/dejavu-sans-fonts/DejaVuSans-Bold.ttf",
            "/usr/share/fonts/TTF/DejaVuSans-Bold.ttf",
            "/usr/share/fonts/dejavu/DejaVuSans-Bold.ttf",
        ]
        .into_iter()
        .find(|candidate| Path::new(candidate).is_file())
        .unwrap_or(path)
    } else {
        path
    };
    let data = fs::read(path)
        .with_context(|| format!("reading font {path}; set font_path in config.toml"))?;
    FontArc::try_from_vec(data).map_err(|_| anyhow::anyhow!("invalid font file: {path}"))
}
fn text_bounds(font: &FontArc, text: &str, size: f32) -> (u32, u32) {
    let f = font.as_scaled(PxScale::from(size));
    let w = text
        .chars()
        .map(|c| f.h_advance(font.glyph_id(c)))
        .sum::<f32>()
        .ceil() as u32;
    let h = (f.ascent() - f.descent()).ceil() as u32;
    (w.max(1), h.max(1))
}
fn draw_text(
    im: &mut RgbaImage,
    font: &FontArc,
    text: &str,
    size: f32,
    x: u32,
    y: u32,
    _w: u32,
    h: u32,
    rotated: bool,
    color: (u8, u8, u8),
) {
    let f = font.as_scaled(PxScale::from(size));
    let baseline = y as f32 + f.ascent();
    let mut cursor = x as f32;
    for ch in text.chars() {
        let glyph = font
            .glyph_id(ch)
            .with_scale_and_position(PxScale::from(size), point(cursor, baseline));
        cursor += f.h_advance(font.glyph_id(ch));
        if let Some(outline) = font.outline_glyph(glyph) {
            let bounds = outline.px_bounds();
            outline.draw(|gx, gy, coverage| {
                if coverage <= 0.02 {
                    return;
                }
                let lx = (bounds.min.x + gx as f32).floor() as i32 - x as i32;
                let ly = (bounds.min.y + gy as f32).floor() as i32 - y as i32;
                let (tx, ty) = if rotated {
                    (x as i32 + h as i32 - 1 - ly, y as i32 + lx)
                } else {
                    (x as i32 + lx, y as i32 + ly)
                };
                if tx < 0 || ty < 0 || tx >= im.width() as i32 || ty >= im.height() as i32 {
                    return;
                }
                let a = coverage.clamp(0.0, 1.0);
                let dst = *im.get_pixel(tx as u32, ty as u32);
                im.put_pixel(
                    tx as u32,
                    ty as u32,
                    Rgba([
                        (color.0 as f32 * a + dst[0] as f32 * (1.0 - a)) as u8,
                        (color.1 as f32 * a + dst[1] as f32 * (1.0 - a)) as u8,
                        (color.2 as f32 * a + dst[2] as f32 * (1.0 - a)) as u8,
                        255,
                    ]),
                );
            });
        }
    }
}
fn hash(s: &str) -> u64 {
    let mut h = 14695981039346656037u64;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(1099511628211);
    }
    h
}
fn parse_color(s: &str) -> (u8, u8, u8) {
    let s = s.trim_start_matches('#');
    if s.len() == 6 {
        (
            u8::from_str_radix(&s[0..2], 16).unwrap_or(17),
            u8::from_str_radix(&s[2..4], 16).unwrap_or(17),
            u8::from_str_radix(&s[4..6], 16).unwrap_or(27),
        )
    } else {
        (17, 17, 27)
    }
}
fn theme_background(t: &str) -> (u8, u8, u8) {
    match t {
        "catppuccin-mocha" => (30, 30, 46),
        "catppuccin-latte" => (239, 241, 245),
        "terminal-green" => (5, 10, 7),
        "cyberpunk" => (10, 8, 24),
        "monochrome" => (15, 15, 15),
        _ => (17, 17, 27),
    }
}
fn palette(c: &Config) -> Vec<(u8, u8, u8)> {
    if c.theme == "custom" {
        return c.custom_palette.iter().map(|x| parse_color(x)).collect();
    }
    let v: &[&str] = match c.theme.as_str() {
        "catppuccin-latte" => &[
            "#d20f39", "#40a02b", "#1e66f5", "#8839ef", "#fe640b", "#179299", "#df8e1d", "#ea76cb",
        ],
        "cyberpunk" => &[
            "#00f5ff", "#ff2a9d", "#bb86fc", "#00b8ff", "#e945ff", "#79c7ff",
        ],
        "monochrome" => &["#eeeeee", "#bdbdbd", "#999999", "#d6d6d6", "#777777"],
        "terminal-green" => &["#39ff78", "#7cff6b", "#b6ff5c", "#00cc66"],
        _ => &[
            "#f38ba8", "#a6e3a1", "#89b4fa", "#f9e2af", "#cba6f7", "#94e2d5", "#fab387", "#f5c2e7",
            "#74c7ec",
        ],
    };
    v.iter().map(|x| parse_color(x)).collect()
}
fn load_layout(path: &Path) -> LayoutMemory {
    fs::read(path)
        .ok()
        .and_then(|x| serde_json::from_slice(&x).ok())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn font_size_changes_gradually_and_respects_new_bounds() {
        let cfg = Config {
            smoothing: 0.25,
            ..Config::default()
        };
        assert_eq!(smoothed_size(Some(100.0), 20.0, &cfg), 80.0);
        assert_eq!(smoothed_size(Some(20.0), 100.0, &cfg), 40.0);
        assert_eq!(smoothed_size(None, 100.0, &cfg), 100.0);
        let smaller = Config {
            max_font_size: 30.0,
            ..cfg
        };
        assert!(smoothed_size(Some(100.0), 20.0, &smaller) <= 30.0);
    }

    #[test]
    fn font_size_ignores_subpixel_jitter_and_legacy_state_loads() {
        assert_eq!(smoothed_size(Some(30.0), 30.2, &Config::default()), 30.0);
        let state: LayoutMemory = serde_json::from_str(
            r#"{"width":800,"height":450,"positions":{"worker":[0.5,0.5,false]}}"#,
        )
        .unwrap();
        assert!(state.font_sizes.is_empty());
        assert!(state.positions.contains_key("worker"));
    }

    #[test]
    fn default_font_loads_on_supported_distributions() {
        load_font(&Config::default().font_path).unwrap();
    }
    #[test]
    fn missing_custom_font_reports_configured_path() {
        let error = load_font("/procscape-missing/custom.ttf").err().unwrap();
        assert!(error.to_string().contains("/procscape-missing/custom.ttf"));
    }
    #[test]
    fn color_hash_is_stable() {
        assert_eq!(hash("python3"), hash("python3"));
        assert_ne!(hash("python3"), hash("code"));
    }
    #[test]
    fn color_parser_reads_hex() {
        assert_eq!(parse_color("#112233"), (17, 34, 51));
    }
    #[test]
    fn multi_monitor_geometry_handles_negative_origin() {
        assert_eq!(
            parse_geometry("1920x1080-1920+0"),
            Some((1920, 1080, -1920, 0))
        );
        assert_eq!(parse_geometry("2560x1440+0+0"), Some((2560, 1440, 0, 0)));
    }
}
