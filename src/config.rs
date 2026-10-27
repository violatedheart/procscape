use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum RunMode {
    Cpu,
    Memory,
    Hybrid,
}
impl Default for RunMode {
    fn default() -> Self {
        Self::Hybrid
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub mode: RunMode,
    pub theme: String,
    pub refresh_seconds: u64,
    pub width: u32,
    pub height: u32,
    pub span_monitors: bool,
    pub max_words: usize,
    pub min_font_size: f32,
    pub max_font_size: f32,
    pub spacing: u32,
    pub rotation_probability: f32,
    pub cpu_weight: f32,
    pub memory_weight: f32,
    pub smoothing: f32,
    pub density: f32,
    pub group_by_name: bool,
    pub include_system: bool,
    pub show_user_only: bool,
    pub exclude_kernel_threads: bool,
    pub exclude_zombies: bool,
    pub pinned_processes: Vec<String>,
    pub search_filter: String,
    pub layout_stability: f32,
    pub min_cpu_percent: f32,
    pub min_memory_kib: u64,
    pub exclude_patterns: Vec<String>,
    pub font_path: String,
    pub margins: u32,
    pub custom_background: String,
    pub custom_palette: Vec<String>,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            mode: RunMode::Hybrid,
            theme: "catppuccin-mocha".into(),
            refresh_seconds: 15,
            width: 0,
            height: 0,
            span_monitors: true,
            max_words: 180,
            min_font_size: 11.0,
            max_font_size: 116.0,
            spacing: 2,
            rotation_probability: 0.12,
            cpu_weight: 0.65,
            memory_weight: 0.35,
            smoothing: 0.35,
            density: 1.0,
            group_by_name: true,
            include_system: true,
            show_user_only: false,
            exclude_kernel_threads: true,
            exclude_zombies: true,
            pinned_processes: Vec::new(),
            search_filter: String::new(),
            layout_stability: 0.72,
            min_cpu_percent: 0.0,
            min_memory_kib: 0,
            exclude_patterns: Vec::new(),
            font_path: "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf".into(),
            margins: 24,
            custom_background: "#11111b".into(),
            custom_palette: vec!["#f38ba8".into(), "#a6e3a1".into(), "#89b4fa".into()],
        }
    }
}
impl Config {
    pub fn sanitize(&mut self) {
        self.refresh_seconds = self.refresh_seconds.clamp(5, 3600);
        self.width = self.width.min(7680);
        self.height = self.height.min(4320);
        self.max_words = self.max_words.clamp(1, 1000);
        if !self.min_font_size.is_finite() {
            self.min_font_size = 11.0;
        }
        if !self.max_font_size.is_finite() {
            self.max_font_size = 116.0;
        }
        self.min_font_size = self.min_font_size.clamp(6.0, 80.0);
        self.max_font_size = self.max_font_size.clamp(self.min_font_size, 240.0);
        self.spacing = self.spacing.min(48);
        self.margins = self.margins.min(500);
        self.rotation_probability = finite_clamp(self.rotation_probability, 0.0, 0.5, 0.12);
        self.cpu_weight = finite_clamp(self.cpu_weight, 0.0, 1.0, 0.65);
        self.memory_weight = finite_clamp(self.memory_weight, 0.0, 1.0, 0.35);
        self.smoothing = finite_clamp(self.smoothing, 0.01, 1.0, 0.35);
        self.density = finite_clamp(self.density, 0.1, 1.0, 1.0);
        self.layout_stability = finite_clamp(self.layout_stability, 0.0, 0.95, 0.72);
        self.min_cpu_percent = finite_clamp(self.min_cpu_percent, 0.0, 10000.0, 0.0);
        if self.font_path.trim().is_empty() {
            self.font_path = Self::default().font_path;
        }
        self.search_filter.truncate(128);
        self.exclude_patterns.truncate(128);
        self.pinned_processes.truncate(128);
        self.custom_palette.truncate(32);
        if self.custom_palette.is_empty() {
            self.custom_palette = Self::default().custom_palette;
        }
    }
}
fn finite_clamp(v: f32, min: f32, max: f32, fallback: f32) -> f32 {
    if v.is_finite() {
        v.clamp(min, max)
    } else {
        fallback
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn defaults_round_trip_toml() {
        let c = Config::default();
        let encoded = toml::to_string(&c).unwrap();
        let decoded: Config = toml::from_str(&encoded).unwrap();
        assert_eq!(decoded.theme, "catppuccin-mocha");
        assert_eq!(decoded.refresh_seconds, 15);
    }
}
