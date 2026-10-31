use gtk4::{
    Application, ApplicationWindow, Box as GtkBox, Button, CheckButton, DropDown, Entry, Grid,
    Label, Orientation, Picture, ScrolledWindow, Separator, SpinButton, prelude::*,
};
use std::{
    cell::{Cell, RefCell},
    fs,
    path::PathBuf,
    process::Command,
    rc::Rc,
};

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
fn binary() -> PathBuf {
    std::env::current_exe()
        .unwrap_or_else(|_| PathBuf::from("procscape-gui"))
        .with_file_name("procscape")
}
fn invoke(args: &[&str]) -> Result<String, String> {
    let output = Command::new(binary())
        .args(args)
        .output()
        .map_err(|e| format!("Could not run ProcScape: {e}"))?;
    command_result(output)
}

fn command_result(output: std::process::Output) -> Result<String, String> {
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        let detail = String::from_utf8_lossy(&output.stderr);
        Err(format!(
            "ProcScape failed ({}): {}",
            output.status,
            detail.trim()
        ))
    }
}

async fn background<T: Send + 'static>(
    job: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    gtk4::gio::spawn_blocking(job)
        .await
        .unwrap_or_else(|_| Err("Background task failed.".into()))
}

fn run_action(
    status: Label,
    controls: Vec<Button>,
    job: impl FnOnce() -> Result<String, String> + Send + 'static,
) {
    status.set_text("Working…");
    for button in &controls {
        button.set_sensitive(false);
    }
    gtk4::glib::MainContext::default().spawn_local(async move {
        let result = background(job).await;
        status.set_text(&result.unwrap_or_else(|error| error));
        for button in controls {
            button.set_sensitive(true);
        }
    });
}

fn local_time(timestamp: u64) -> String {
    i64::try_from(timestamp)
        .ok()
        .and_then(|t| gtk4::glib::DateTime::from_unix_local(t).ok())
        .and_then(|date| date.format("%d %b %Y, %H:%M:%S").ok())
        .map(|text| text.to_string())
        .unwrap_or_else(|| "Unknown time".into())
}

fn service_summary(report: &procscape::monitor::ServiceReport) -> (String, String, String) {
    let last = report
        .last_success
        .as_ref()
        .map(|info| local_time(info.collected_at))
        .unwrap_or_else(|| "No successful update yet".into());
    let error = report
        .last_error
        .as_ref()
        .map(|error| {
            format!(
                "Last error ({}): {}",
                local_time(error.occurred_at),
                error.message
            )
        })
        .unwrap_or_default();
    (
        format!("Service: {}", report.state),
        format!("Last successful update: {last}"),
        error,
    )
}

fn update_service_labels(
    state: Label,
    updated: Label,
    error: Label,
    picture: Picture,
    caption: Label,
    current_image: Rc<RefCell<Option<PathBuf>>>,
    pending: Rc<Cell<bool>>,
) {
    if pending.replace(true) {
        return;
    }
    gtk4::glib::MainContext::default().spawn_local(async move {
        let result = background(|| {
            invoke(&["status", "--json"]).and_then(|raw| {
                serde_json::from_str::<procscape::monitor::ServiceReport>(&raw)
                    .map_err(|e| e.to_string())
            })
        })
        .await;
        match result {
            Ok(report) => {
                let (state_text, update_text, error_text) = service_summary(&report);
                state.set_text(&state_text);
                updated.set_text(&update_text);
                error.set_text(&error_text);
                error.set_visible(!error_text.is_empty());
                // Only show the exact image acknowledged as applied by the daemon.
                // Restore clears this field; opening the dashboard never renders.
                if *current_image.borrow() != report.active_wallpaper {
                    picture.set_filename(report.active_wallpaper.as_deref());
                    *current_image.borrow_mut() = report.active_wallpaper.clone();
                }
                caption.set_text(if report.active_wallpaper.is_some() {
                    "Last applied ProcScape wallpaper"
                } else {
                    "No ProcScape wallpaper is active."
                });
            }
            Err(message) => {
                state.set_text("Service: Unable to read status");
                updated.set_text("Last successful update: Unavailable");
                error.set_text(&message);
                error.set_visible(true);
            }
        }
        pending.set(false);
    });
}

fn update_metrics(
    count: Label,
    cpu: Label,
    memory: Label,
    mode: Label,
    list: Label,
    pending: Rc<Cell<bool>>,
) {
    if pending.replace(true) {
        return;
    }
    gtk4::glib::MainContext::default().spawn_local(async move {
        let result = background(|| {
            let raw = invoke(&["metrics"])?;
            let snapshot = serde_json::from_str::<procscape::monitor::Snapshot>(&raw)
                .map_err(|e| e.to_string())?;
            Ok((snapshot, mode_name()))
        })
        .await;
        match result {
            Ok((snapshot, mode_name)) => {
                count.set_text(&format!("Processes: {}", snapshot.processes.len()));
                cpu.set_text(&format!("CPU: {:.1}%", snapshot.cpu_percent));
                memory.set_text(&format!("Memory: {:.1}%", snapshot.memory_percent));
                mode.set_text(&format!("Mode: {mode_name}"));
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
                list.set_text(if text.is_empty() {
                    "No matching processes"
                } else {
                    &text
                });
            }
            Err(message) => list.set_text(&format!("Metrics unavailable: {message}")),
        }
        pending.set(false);
    });
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
    fade: SpinButton,
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
    status.set_wrap(true);
    root.append(&status);

    let service_state = Label::new(None);
    let last_update = Label::new(None);
    let service_error = Label::new(None);
    service_error.add_css_class("error");
    for label in [&service_state, &last_update, &service_error] {
        label.set_xalign(0.0);
        label.set_wrap(true);
        label.set_selectable(true);
        root.append(label);
    }
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
    root.append(&preview);
    let preview_caption = Label::new(Some("Loading applied wallpaper…"));
    preview_caption.add_css_class("dim-label");
    root.append(&preview_caption);

    let process_list = Label::new(Some("Waiting for process metrics…"));
    process_list.set_xalign(0.0);
    process_list.set_wrap(true);
    process_list.set_selectable(true);
    root.append(&process_list);

    let actions = GtkBox::new(Orientation::Horizontal, 8);
    let start = Button::with_label("Start / resume");
    let pause = Button::with_label("Pause");
    let refresh = Button::with_label("Refresh now");
    let restore = Button::with_label("Restore my wallpaper");
    for button in [&start, &pause, &refresh, &restore] {
        actions.append(button);
    }
    root.append(&actions);

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
    add_spin(
        &grid,
        0,
        8,
        "Refresh fade (seconds; 0 = off)",
        &settings.fade,
        0.0,
        2.0,
        0.1,
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
    let controls = vec![
        start.clone(),
        pause.clone(),
        refresh.clone(),
        restore.clone(),
        apply.clone(),
    ];
    for (button, command) in [
        (&start, "start"),
        (&pause, "pause"),
        (&refresh, "refresh"),
        (&restore, "restore"),
    ] {
        let status = status.clone();
        let controls = controls.clone();
        button.connect_clicked(move |_| {
            run_action(status.clone(), controls.clone(), move || invoke(&[command]));
        });
    }
    let result = status.clone();
    apply.connect_clicked(move |_| {
        if !settings.start_login.is_sensitive() {
            result.set_text("Loading service settings. Try again shortly.");
            return;
        }
        let cfg = settings_config(&settings);
        let start_login = settings.start_login.is_active();
        run_action(result.clone(), controls.clone(), move || {
            save_settings(cfg, start_login).map_err(|e| e.to_string())?;
            match invoke(&["refresh"]) {
                Ok(message) => Ok(format!("Settings saved. {message}")),
                Err(error) => Err(format!("Settings saved. {error}")),
            }
        });
    });
    root.append(&apply);
    let note = Label::new(Some(
        "Restore my wallpaper pauses updates until Start / resume, even after restarting. Change your personal wallpaper while paused; ProcScape remembers it the next time updates run. Closing this window keeps the current service state.",
    ));
    note.set_xalign(0.0);
    note.set_wrap(true);
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

    let status_pending = Rc::new(Cell::new(false));
    let current_image = Rc::new(RefCell::new(None));
    let poll_status = move || {
        update_service_labels(
            service_state.clone(),
            last_update.clone(),
            service_error.clone(),
            preview.clone(),
            preview_caption.clone(),
            current_image.clone(),
            status_pending.clone(),
        )
    };
    poll_status();
    let weak_window = win.downgrade();
    gtk4::glib::timeout_add_seconds_local(1, move || {
        if weak_window.upgrade().is_none() {
            return gtk4::glib::ControlFlow::Break;
        }
        poll_status();
        gtk4::glib::ControlFlow::Continue
    });
    let metrics_pending = Rc::new(Cell::new(false));
    let poll_metrics = move || {
        update_metrics(
            processes_metric.clone(),
            cpu_metric.clone(),
            memory_metric.clone(),
            mode_metric.clone(),
            process_list.clone(),
            metrics_pending.clone(),
        )
    };
    poll_metrics();
    let weak_window = win.downgrade();
    gtk4::glib::timeout_add_seconds_local(5, move || {
        if weak_window.upgrade().is_none() {
            return gtk4::glib::ControlFlow::Break;
        }
        poll_metrics();
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
    let fade = spin(0.6, 0.0, 2.0, 0.1);
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
        fade,
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
        (&w.fade, cfg.transition_ms as f64 / 1000.0),
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
    let start_login = w.start_login.clone();
    start_login.set_sensitive(false);
    gtk4::glib::MainContext::default().spawn_local(async move {
        let enabled = background(|| {
            Ok(Command::new("systemctl")
                .args(["--user", "is-enabled", "procscape.service"])
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false))
        })
        .await
        .unwrap_or(false);
        start_login.set_active(enabled);
        start_login.set_sensitive(true);
    });
}

fn settings_config(w: &SettingsWidgets) -> Config {
    let path = config_path();
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
    cfg.transition_ms = (w.fade.value() * 1000.0).round() as u64;
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
    cfg
}

fn save_settings(cfg: Config, start_login: bool) -> Result<(), Box<dyn std::error::Error>> {
    let path = config_path();
    fs::create_dir_all(path.parent().unwrap())?;
    let temporary = path.with_extension("toml.tmp");
    fs::write(&temporary, toml::to_string_pretty(&cfg)?)?;
    fs::rename(temporary, path)?;
    let currently_enabled = Command::new("systemctl")
        .args(["--user", "is-enabled", "procscape.service"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if currently_enabled != start_login {
        let action = if start_login { "enable" } else { "disable" };
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::process::ExitStatusExt;

    #[test]
    fn service_summary_shows_state_update_and_error_independently() {
        let mut report = procscape::monitor::ServiceReport {
            state: "Paused".into(),
            pid: Some(42),
            last_success: None,
            last_error: None,
            active_wallpaper: None,
        };
        let (state, update, error) = service_summary(&report);
        assert_eq!(state, "Service: Paused");
        assert!(update.contains("No successful update yet"));
        assert!(error.is_empty());
        report.last_success = Some(procscape::monitor::ServiceStatus {
            collected_at: 1_793_375_909,
            ..Default::default()
        });
        report.last_error = Some(procscape::monitor::RefreshError {
            occurred_at: 1_793_375_910,
            message: "Desktop unavailable".into(),
        });
        let (_, update, error) = service_summary(&report);
        assert!(!update.contains("No successful update"));
        assert!(error.contains("Desktop unavailable"));
    }

    #[test]
    fn background_work_keeps_main_context_available() {
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        };
        let context = gtk4::glib::MainContext::new();
        context
            .with_thread_default(|| {
                context.block_on(async {
                    let finished = Arc::new(AtomicBool::new(false));
                    let flag = finished.clone();
                    let (release, wait) = std::sync::mpsc::channel();
                    let job = background(move || {
                        wait.recv_timeout(std::time::Duration::from_secs(5))
                            .map_err(|e| e.to_string())?;
                        flag.store(true, Ordering::SeqCst);
                        Ok("done")
                    });
                    // The main-loop timer must be able to release a blocked worker.
                    let task = context.spawn_local(job);
                    gtk4::glib::timeout_future(std::time::Duration::from_millis(15)).await;
                    assert!(!finished.load(Ordering::SeqCst));
                    release.send(()).unwrap();
                    assert_eq!(task.await.unwrap().unwrap(), "done");
                })
            })
            .unwrap();
    }

    #[test]
    fn failed_commands_show_stderr() {
        let output = std::process::Output {
            status: std::process::ExitStatus::from_raw(1 << 8),
            stdout: Vec::new(),
            stderr: b"Error: reading font /missing/font.ttf\n".to_vec(),
        };
        let error = command_result(output).unwrap_err();
        assert!(error.contains("reading font /missing/font.ttf"));
        assert!(error.contains("exit status: 1"));
    }
}
