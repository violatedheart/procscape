//! Short, bounded image crossfades for wallpaper backends that accept PNGs.
use crate::{config::Config, wallpaper};
use anyhow::Result;
use image::{
    ImageEncoder, RgbaImage,
    codecs::png::{CompressionType, FilterType, PngEncoder},
};
use std::{
    fs,
    path::Path,
    thread,
    time::{Duration, Instant},
};

pub fn set_active(data: &Path, image: &Path) -> Result<()> {
    let path = data.join("active-wallpaper.txt");
    let temporary = path.with_extension("txt.tmp");
    fs::write(&temporary, image.to_string_lossy().as_bytes())?;
    fs::rename(temporary, path)?;
    Ok(())
}

pub fn apply(data: &Path, image: &Path, cfg: &Config) -> Result<()> {
    if cfg.transition_ms > 0 {
        if let Some((old, new)) = transition_images(data, image) {
            let duration = Duration::from_millis(cfg.transition_ms.min(2000));
            let steps = if new.width() as u64 * new.height() as u64 > 2_100_000 {
                6
            } else {
                12
            };
            let started = Instant::now();
            for step in 1..steps {
                if started.elapsed() >= duration {
                    break;
                }
                let fraction = step as f32 / steps as f32;
                let alpha = fraction * fraction * (3.0 - 2.0 * fraction);
                let frame = blend(&old, &new, alpha);
                let path = image.with_file_name(format!(
                    "{}-fade-{step:02}.png",
                    image.file_stem().unwrap().to_string_lossy()
                ));
                let apply_frame = || -> Result<()> {
                    let file = fs::File::create(&path)?;
                    PngEncoder::new_with_quality(file, CompressionType::Fast, FilterType::Sub)
                        .write_image(
                            frame.as_raw(),
                            frame.width(),
                            frame.height(),
                            image::ExtendedColorType::Rgba8,
                        )?;
                    wallpaper::apply(data, &path, cfg)?;
                    set_active(data, &path)
                };
                // A transition is optional. If a frame cannot be displayed,
                // still try the final wallpaper and report that result.
                if apply_frame().is_err() {
                    break;
                }
                let deadline = duration.mul_f32(fraction);
                thread::sleep(deadline.saturating_sub(started.elapsed()));
            }
        }
    }
    wallpaper::apply(data, image, cfg)?;
    set_active(data, image)
}

fn transition_images(data: &Path, target: &Path) -> Option<(RgbaImage, RgbaImage)> {
    let previous = fs::read_to_string(data.join("active-wallpaper.txt")).ok()?;
    if Path::new(&previous) == target {
        return None;
    }
    // Inspect dimensions before allocating full image buffers.
    let dimensions = image::image_dimensions(target).ok()?;
    if image::image_dimensions(&previous).ok()? != dimensions
        || dimensions.0 as u64 * dimensions.1 as u64 > 8_500_000
    {
        return None;
    }
    let old = image::open(previous).ok()?.to_rgba8();
    let new = image::open(target).ok()?.to_rgba8();
    if old == new {
        return None;
    }
    Some((old, new))
}

fn blend(old: &RgbaImage, new: &RgbaImage, alpha: f32) -> RgbaImage {
    let alpha = alpha.clamp(0.0, 1.0);
    let mut frame = old.clone();
    for (output, target) in frame.as_mut().iter_mut().zip(new.as_raw()) {
        *output = (*output as f32 * (1.0 - alpha) + *target as f32 * alpha).round() as u8;
    }
    frame
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn blend_keeps_unchanged_pixels_and_interpolates_changed_names() {
        let old = RgbaImage::from_pixel(2, 1, image::Rgba([20, 40, 60, 255]));
        let mut new = old.clone();
        new.put_pixel(1, 0, image::Rgba([220, 140, 100, 255]));
        let halfway = blend(&old, &new, 0.5);
        assert_eq!(halfway.get_pixel(0, 0), old.get_pixel(0, 0));
        assert_eq!(halfway.get_pixel(1, 0).0, [120, 90, 80, 255]);
        assert_eq!(blend(&old, &new, 0.0), old);
        assert_eq!(blend(&old, &new, 1.0), new);
    }
}
