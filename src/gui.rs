use gtk4::{
    Application, ApplicationWindow, Box as GtkBox, Button, CheckButton, DropDown, Entry, Grid,
    Label, Orientation, Picture, ScrolledWindow, Separator, SpinButton, prelude::*,
};
use std::{fs, path::PathBuf, process::Command};

type Config = procscape::config::Config;

fn main() {
    let app = Application::builder()
        .application_id("io.github.procscape.ProcScape")
        .build();
    app.connect_activate(build);
    app.run();
}

fn config_path() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(dirs::config_dir)
        .unwrap_or_else(|| PathBuf::from(".config"))
        .join("procscape/config.toml")
}
fn data_path() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(dirs::data_dir)
        .unwrap_or_else(|| PathBuf::from(".local/share"))
        .join("procscape")
}
fn binary() -> PathBuf {
    std::env::current_exe()
        .unwrap_or_else(|_| PathBuf::from("procscape-gui"))
        .with_file_name("procscape")
}
fn invoke(args: &[&str]) -> String {
    Command::new(binary())
        .args(args)
        .output()
        .map(|x| String::from_utf8_lossy(&x.stdout).to_string())
        .unwrap_or_else(|e| format!("Could not run ProcScape: {e}"))
}

struct SettingsWidgets {
    theme: DropDown,
    mode: DropDown,
    interval: DropDown,
    interval_custom: SpinButton,
    min_font: SpinButton,
    max_font: SpinButton,
    max_words: SpinButton,
    spacing: SpinButton,
    margins: SpinButton,
    density: SpinButton,
    rotation: SpinButton,
    cpu_weight: SpinButton,
    memory_weight: SpinButton,
    smoothing: SpinButton,
    stability: SpinButton,
    min_cpu: SpinButton,
    min_memory: SpinButton,
    width: SpinButton,
    height: SpinButton,
    search: Entry,
    excludes: Entry,
    pins: Entry,
    font: Entry,
    background: Entry,
    palette: Entry,
    only_user: CheckButton,
    aggregate: CheckButton,
    exclude_kernel: CheckButton,
    exclude_zombies: CheckButton,
    span_monitors: CheckButton,
    start_login: CheckButton,
}

fn build(app: &Application) {
    let root = GtkBox::new(Orientation::Vertical, 14);
    root.set_margin_top(20);
    root.set_margin_bottom(20);
    root.set_margin_start(22);
    root.set_margin_end(22);

    let title = Label::new(None);
    title.set_markup("<span size='xx-large' weight='bold'>ProcScape</span>  <span alpha='70%'>Living process wallpaper</span>");
    title.set_xalign(0.0);
    root.append(&title);

    let status = Label::new(Some(
        "Ready · live metrics update while this window is open",
    ));
    status.set_xalign(0.0);
    root.append(&status);

    let stats = GtkBox::new(Orientation::Horizontal, 18);
    let processes_metric = Label::new(Some("Processes: —"));
    let cpu_metric = Label::new(Some("CPU: —"));
    let memory_metric = Label::new(Some("Memory: —"));
    let mode_metric = Label::new(Some("Mode: —"));
    for item in [&processes_metric, &cpu_metric, &memory_metric, &mode_metric] {
        item.add_css_class("card");
        stats.append(item);
    }
    root.append(&stats);

    let preview = Picture::new();
    preview.set_size_request(740, 390);
    preview.set_can_shrink(true);
    preview.set_content_fit(gtk4::ContentFit::Contain);
    let _ = fs::create_dir_all(data_path());
    let image = data_path().join("wallpaper.png");
    let active_image = fs::read_to_string(data_path().join("active-wallpaper.txt"))
        .ok()
        .map(PathBuf::from)
        .filter(|path| path.exists())
        .unwrap_or_else(|| image.clone());
    if active_image.exists() {
        preview.set_filename(Some(&active_image.to_string_lossy()));
    }
    root.append(&preview);

    let process_list = Label::new(Some("Waiting for process metrics…"));
    process_list.set_xalign(0.0);
    process_list.set_wrap(true);
    process_list.set_selectable(true);
    root.append(&process_list);

    let actions = GtkBox::new(Orientation::Horizontal, 8);
    let start = Button::with_label("Start / resume");
    let pause = Button::with_label("Pause");
    let refresh = Button::with_label("Refresh now");
    let restore = Button::with_label("Restore previous wallpaper");
    for button in [&start, &pause, &refresh, &restore] {
        actions.append(button);
    }
    root.append(&actions);

    let result = status.clone();
    start.connect_clicked(move |_| result.set_text(&invoke(&["start"])));
    let result = status.clone();
    pause.connect_clicked(move |_| result.set_text(&invoke(&["pause"])));
    let result = status.clone();
    let picture = preview.clone();
    let refresh_image = image.clone();
    refresh.connect_clicked(move |_| {
        let _ = invoke(&["refresh"]);
        let _ = invoke(&["snapshot"]);
        if refresh_image.exists() {
            picture.set_filename(Some(&refresh_image.to_string_lossy()));
        }
        result.set_text("Wallpaper refreshed. Process metrics continue to update below.");
    });
    let result = status.clone();
    restore.connect_clicked(move |_| result.set_text(&invoke(&["restore"])));

    root.append(&Separator::new(Orientation::Horizontal));
    let heading = Label::new(Some("Appearance and process filtering"));
    heading.set_xalign(0.0);
    heading.add_css_class("heading");
    root.append(&heading);

    let settings = settings_widgets();
    let theme_row = GtkBox::new(Orientation::Horizontal, 12);
    theme_row.append(&Label::new(Some("Theme")));
    theme_row.append(&settings.theme);
    theme_row.append(&Label::new(Some("Sizing mode")));
    theme_row.append(&settings.mode);
    theme_row.append(&Label::new(Some("Refresh")));
    theme_row.append(&settings.interval);
    theme_row.append(&Label::new(Some("Custom seconds")));
    theme_row.append(&settings.interval_custom);
    root.append(&theme_row);

    let grid = Grid::new();
    grid.set_row_spacing(8);
    grid.set_column_spacing(14);
    add_spin(
        &grid,
        0,
        0,
        "Minimum font",
        &settings.min_font,
        6.0,
        80.0,
        1.0,
    );
    add_spin(
        &grid,
        2,
        0,
        "Maximum font",
        &settings.max_font,
        6.0,
        240.0,
        1.0,
    );
    add_spin(
        &grid,
        0,
        1,
        "Word spacing",
        &settings.spacing,
        0.0,
        48.0,
        1.0,
    );
    add_spin(&grid, 2, 1, "Density", &settings.density, 0.1, 1.0, 0.05);
    add_spin(
        &grid,
        0,
        2,
        "Vertical rotation chance",
        &settings.rotation,
        0.0,
        0.5,
        0.01,
    );
    add_spin(
        &grid,
        2,
        2,
        "CPU weight",
        &settings.cpu_weight,
        0.0,
        1.0,
        0.05,
    );
    add_spin(
        &grid,
        0,
        3,
        "Memory weight",
        &settings.memory_weight,
        0.0,
        1.0,
        0.05,
    );
    add_spin(
        &grid,
        2,
        3,
        "Smoothing",
        &settings.smoothing,
        0.01,
        1.0,
        0.05,
    );
    add_spin(
        &grid,
        0,
        4,
        "Layout stability",
        &settings.stability,
        0.0,
        0.95,
        0.05,
    );
    add_spin(
        &grid,
        2,
        4,
        "Minimum CPU %",
        &settings.min_cpu,
        0.0,
        10000.0,
        0.5,
    );
    add_spin(
        &grid,
        0,
        5,
        "Minimum RSS KiB",
        &settings.min_memory,
        0.0,
        1_000_000_000.0,
        1024.0,
    );
    add_spin(
        &grid,
        2,
        5,
        "Output width (0 = detect)",
        &settings.width,
        0.0,
        7680.0,
        100.0,
    );
    add_spin(
        &grid,
        0,
        6,
        "Output height (0 = detect)",
        &settings.height,
        0.0,
        4320.0,
        100.0,
    );
    add_spin(
        &grid,
        0,
        7,
        "Maximum visible words",
        &settings.max_words,
        1.0,
        1000.0,
        10.0,
    );
    add_spin(
        &grid,
        2,
        7,
        "Wallpaper margin",
        &settings.margins,
        0.0,
        500.0,
        4.0,
    );
    root.append(&grid);

    add_entry(&root, "Search process names", &settings.search);
    add_entry(
        &root,
        "Exclude names (comma separated; * is a wildcard)",
        &settings.excludes,
    );
    add_entry(
        &root,
        "Pinned process names (comma separated)",
        &settings.pins,
    );
    add_entry(&root, "Font file", &settings.font);
    add_entry(&root, "Custom background color", &settings.background);
    add_entry(
        &root,
        "Custom palette (comma separated hex colors)",
        &settings.palette,
    );

    let toggles = GtkBox::new(Orientation::Horizontal, 12);
    for item in [
        &settings.only_user,
        &settings.aggregate,
        &settings.exclude_kernel,
        &settings.exclude_zombies,
        &settings.span_monitors,
        &settings.start_login,
    ] {
        toggles.append(item);
    }
    root.append(&toggles);

    let apply = Button::with_label("Save settings and refresh");
    let result = status.clone();
    let picture = preview.clone();
    let preview_image = image.clone();
    apply.connect_clicked(move |_| match save_settings(&settings) {
        Ok(()) => {
            let _ = invoke(&["refresh"]);
            let _ = invoke(&["snapshot"]);
            if preview_image.exists() {
                picture.set_filename(Some(&preview_image.to_string_lossy()));
            }
            result.set_text("Settings saved. Wallpaper refresh requested.");
        }
        Err(error) => result.set_text(&format!("Settings could not be saved: {error}")),
    });
    root.append(&apply);
    let note = Label::new(Some(
        "Wallpaper updates continue after closing this window. Desktop wallpaper application needs a supported GNOME or XFCE session.",
    ));
    note.set_xalign(0.0);
    note.add_css_class("dim-label");
    root.append(&note);

    let scrolled = ScrolledWindow::builder().child(&root).build();
    let win = ApplicationWindow::builder()
        .application(app)
        .title("ProcScape")
        .default_width(1040)
        .default_height(900)
        .child(&scrolled)
        .build();
    win.present();

    let process_label = process_list.clone();
    let process_count = processes_metric.clone();
    let cpu_value = cpu_metric.clone();
    let memory_value = memory_metric.clone();
    let mode_value = mode_metric.clone();
    gtk4::glib::timeout_add_seconds_local(5, move || {
        let raw = invoke(&["metrics"]);
        if let Ok(snapshot) = serde_json::from_str::<procscape::monitor::Snapshot>(&raw) {
            process_count.set_text(&format!("Processes: {}", snapshot.processes.len()));
            cpu_value.set_text(&format!("CPU: {:.1}%", snapshot.cpu_percent));
            memory_value.set_text(&format!("Memory: {:.1}%", snapshot.memory_percent));
            let mut processes = snapshot.processes;
            processes.sort_by(|a, b| {
                b.weight
                    .total_cmp(&a.weight)
                    .then_with(|| a.name.cmp(&b.name))
            });
            let text = processes
                .iter()
                .take(24)
                .map(|p| {
                    format!(
                        "{} · {:.0}% CPU · {:.1} MiB",
                        p.name,
                        p.cpu_percent,
                        p.rss_kib as f32 / 1024.0
                    )
                })
                .collect::<Vec<_>>()
                .join("    •    ");
            process_label.set_text(if text.is_empty() {
                "No matching processes"
            } else {
                &text
            });
            mode_value.set_text(&format!("Mode: {}", mode_name()));
        }
        gtk4::glib::ControlFlow::Continue
    });
}

fn settings_widgets() -> SettingsWidgets {
    let theme = DropDown::from_strings(&[
        "Catppuccin Mocha",
        "Catppuccin Latte",
        "Terminal Green",
        "Cyberpunk",
        "Monochrome",
        "Custom",
    ]);
    let mode = DropDown::from_strings(&["Hybrid", "CPU", "Memory"]);
    let interval = DropDown::from_strings(&[
        "5 seconds",
        "10 seconds",
        "15 seconds",
        "30 seconds",
        "60 seconds",
        "Custom…",
    ]);
    let interval_custom = spin(15.0, 5.0, 3600.0, 1.0);
    let min_font = spin(11.0, 6.0, 80.0, 1.0);
    let max_font = spin(116.0, 6.0, 240.0, 1.0);
    let max_words = spin(180.0, 1.0, 1000.0, 10.0);
    let spacing = spin(2.0, 0.0, 48.0, 1.0);
    let margins = spin(24.0, 0.0, 500.0, 4.0);
    let density = spin(1.0, 0.1, 1.0, 0.05);
    let rotation = spin(0.12, 0.0, 0.5, 0.01);
    let cpu_weight = spin(0.65, 0.0, 1.0, 0.05);
    let memory_weight = spin(0.35, 0.0, 1.0, 0.05);
    let smoothing = spin(0.35, 0.01, 1.0, 0.05);
    let stability = spin(0.72, 0.0, 0.95, 0.05);
    let min_cpu = spin(0.0, 0.0, 10000.0, 0.5);
    let min_memory = spin(0.0, 0.0, 1_000_000_000.0, 1024.0);
    let width = spin(0.0, 0.0, 7680.0, 100.0);
    let height = spin(0.0, 0.0, 4320.0, 100.0);
    let search = Entry::new();
    let excludes = Entry::new();
    let pins = Entry::new();
    let font = Entry::new();
    let background = Entry::new();
    let palette = Entry::new();
    let only_user = CheckButton::with_label("Only my processes");
    let aggregate = CheckButton::with_label("Group duplicate names");
    let exclude_kernel = CheckButton::with_label("Exclude kernel threads");
    let exclude_zombies = CheckButton::with_label("Exclude zombies");
    let span_monitors = CheckButton::with_label("Span wallpaper across monitors");
    let start_login = CheckButton::with_label("Start on login (systemd user service)");
    let widgets = SettingsWidgets {
        theme,
        mode,
        interval,
        interval_custom,
        min_font,
        max_font,
        max_words,
        spacing,
        margins,
        density,
        rotation,
        cpu_weight,
        memory_weight,
        smoothing,
        stability,
        min_cpu,
        min_memory,
        width,
        height,
        search,
        excludes,
        pins,
        font,
        background,
        palette,
        only_user,
        aggregate,
        exclude_kernel,
        exclude_zombies,
        span_monitors,
        start_login,
    };
    load_settings(&widgets);
    widgets
}

fn load_settings(w: &SettingsWidgets) {
    let cfg = fs::read_to_string(config_path())
        .ok()
        .and_then(|s| toml::from_str::<Config>(&s).ok())
        .unwrap_or_default();
    let theme = match cfg.theme.as_str() {
        "catppuccin-latte" => 1,
        "terminal-green" => 2,
        "cyberpunk" => 3,
        "monochrome" => 4,
        "custom" => 5,
        _ => 0,
    };
    w.theme.set_selected(theme);
    w.mode.set_selected(match cfg.mode {
        procscape::config::RunMode::Hybrid => 0,
        procscape::config::RunMode::Cpu => 1,
        procscape::config::RunMode::Memory => 2,
    });
    w.interval.set_selected(match cfg.refresh_seconds {
        5 => 0,
        10 => 1,
        15 => 2,
        30 => 3,
        60 => 4,
        _ => 5,
    });
    w.interval_custom.set_value(cfg.refresh_seconds as f64);
    for (widget, value) in [
        (&w.min_font, cfg.min_font_size as f64),
        (&w.max_font, cfg.max_font_size as f64),
        (&w.max_words, cfg.max_words as f64),
        (&w.spacing, cfg.spacing as f64),
        (&w.margins, cfg.margins as f64),
        (&w.density, cfg.density as f64),
        (&w.rotation, cfg.rotation_probability as f64),
        (&w.cpu_weight, cfg.cpu_weight as f64),
        (&w.memory_weight, cfg.memory_weight as f64),
        (&w.smoothing, cfg.smoothing as f64),
        (&w.stability, cfg.layout_stability as f64),
        (&w.min_cpu, cfg.min_cpu_percent as f64),
        (&w.min_memory, cfg.min_memory_kib as f64),
        (&w.width, cfg.width as f64),
        (&w.height, cfg.height as f64),
    ] {
        widget.set_value(value);
    }
    w.search.set_text(&cfg.search_filter);
    w.excludes.set_text(&cfg.exclude_patterns.join(", "));
    w.pins.set_text(&cfg.pinned_processes.join(", "));
    w.font.set_text(&cfg.font_path);
    w.background.set_text(&cfg.custom_background);
    w.palette.set_text(&cfg.custom_palette.join(", "));
    w.only_user.set_active(cfg.show_user_only);
    w.aggregate.set_active(cfg.group_by_name);
    w.exclude_kernel.set_active(cfg.exclude_kernel_threads);
    w.exclude_zombies.set_active(cfg.exclude_zombies);
    w.span_monitors.set_active(cfg.span_monitors);
    let enabled = Command::new("systemctl")
        .args(["--user", "is-enabled", "procscape.service"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    w.start_login.set_active(enabled);
}

fn save_settings(w: &SettingsWidgets) -> Result<(), Box<dyn std::error::Error>> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut cfg = fs::read_to_string(&path)
        .ok()
        .and_then(|s| toml::from_str::<Config>(&s).ok())
        .unwrap_or_default();
    cfg.theme = [
        "catppuccin-mocha",
        "catppuccin-latte",
        "terminal-green",
        "cyberpunk",
        "monochrome",
        "custom",
    ]
    .get(w.theme.selected() as usize)
    .unwrap_or(&"catppuccin-mocha")
    .to_string();
    cfg.mode = [
        procscape::config::RunMode::Hybrid,
        procscape::config::RunMode::Cpu,
        procscape::config::RunMode::Memory,
    ]
    .get(w.mode.selected() as usize)
    .copied()
    .unwrap_or(procscape::config::RunMode::Hybrid);
    cfg.refresh_seconds = [5, 10, 15, 30, 60]
        .get(w.interval.selected() as usize)
        .copied()
        .unwrap_or(w.interval_custom.value_as_int().max(5) as u64);
    cfg.min_font_size = w.min_font.value() as f32;
    cfg.max_font_size = w.max_font.value() as f32;
    cfg.max_words = w.max_words.value_as_int().max(1) as usize;
    cfg.spacing = w.spacing.value_as_int().max(0) as u32;
    cfg.margins = w.margins.value_as_int().max(0) as u32;
    cfg.density = w.density.value() as f32;
    cfg.rotation_probability = w.rotation.value() as f32;
    cfg.cpu_weight = w.cpu_weight.value() as f32;
    cfg.memory_weight = w.memory_weight.value() as f32;
    cfg.smoothing = w.smoothing.value() as f32;
    cfg.layout_stability = w.stability.value() as f32;
    cfg.min_cpu_percent = w.min_cpu.value() as f32;
    cfg.min_memory_kib = w.min_memory.value_as_int().max(0) as u64;
    cfg.width = w.width.value_as_int().max(0) as u32;
    cfg.height = w.height.value_as_int().max(0) as u32;
    cfg.search_filter = w.search.text().trim().to_string();
    cfg.exclude_patterns = split_csv(&w.excludes.text());
    cfg.pinned_processes = split_csv(&w.pins.text());
    cfg.font_path = w.font.text().trim().to_string();
    cfg.custom_background = w.background.text().trim().to_string();
    cfg.custom_palette = split_csv(&w.palette.text());
    cfg.show_user_only = w.only_user.is_active();
    cfg.include_system = !cfg.show_user_only;
    cfg.group_by_name = w.aggregate.is_active();
    cfg.exclude_kernel_threads = w.exclude_kernel.is_active();
    cfg.exclude_zombies = w.exclude_zombies.is_active();
    cfg.span_monitors = w.span_monitors.is_active();
    cfg.sanitize();
    fs::write(path, toml::to_string_pretty(&cfg)?)?;
    let currently_enabled = Command::new("systemctl")
        .args(["--user", "is-enabled", "procscape.service"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if currently_enabled != w.start_login.is_active() {
        let action = if w.start_login.is_active() {
            "enable"
        } else {
            "disable"
        };
        let result = Command::new("systemctl")
            .args(["--user", action, "procscape.service"])
            .output()?;
        if !result.status.success() {
            return Err(format!("systemctl --user {action} procscape.service failed; install the ProcScape service first").into());
        }
    }
    Ok(())
}

fn split_csv(input: &str) -> Vec<String> {
    input
        .split(',')
        .map(str::trim)
        .filter(|x| !x.is_empty())
        .map(str::to_string)
        .collect()
}
fn spin(value: f64, min: f64, max: f64, step: f64) -> SpinButton {
    let widget = SpinButton::with_range(min, max, step);
    widget.set_value(value);
    widget
}
fn add_spin(
    grid: &Grid,
    col: i32,
    row: i32,
    name: &str,
    widget: &SpinButton,
    min: f64,
    max: f64,
    step: f64,
) {
    let _ = (min, max, step);
    let label = Label::new(Some(name));
    label.set_xalign(0.0);
    grid.attach(&label, col, row, 1, 1);
    grid.attach(widget, col + 1, row, 1, 1);
}
fn add_entry(root: &GtkBox, label: &str, entry: &Entry) {
    let row = GtkBox::new(Orientation::Horizontal, 10);
    let caption = Label::new(Some(label));
    caption.set_xalign(0.0);
    caption.set_width_chars(34);
    entry.set_hexpand(true);
    row.append(&caption);
    row.append(entry);
    root.append(&row);
}
fn mode_name() -> String {
    fs::read_to_string(config_path())
        .ok()
        .and_then(|s| toml::from_str::<Config>(&s).ok())
        .map(|c| format!("{:?}", c.mode))
        .unwrap_or_else(|| "Hybrid".into())
}
