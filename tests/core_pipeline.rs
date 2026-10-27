#![cfg(target_os = "linux")]

use procscape::{
    config::Config,
    monitor::{Sampler, collect_snapshot},
    render,
};

#[test]
fn live_proc_snapshot_renders_a_valid_png() {
    let temp = tempfile::tempdir().unwrap();
    let mut cfg = Config::default();
    cfg.width = 720;
    cfg.height = 405;
    cfg.max_words = 80;
    cfg.sanitize();

    let mut sampler = Sampler::default();
    let _first = collect_snapshot(&cfg, Some(&mut sampler)).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(180));
    let snapshot = collect_snapshot(&cfg, Some(&mut sampler)).unwrap();
    assert!(
        !snapshot.processes.is_empty(),
        "the live /proc scan should find this test process"
    );

    let image_path = temp.path().join("wallpaper.png");
    let state_path = temp.path().join("state/layout.json");
    render::generate(&snapshot, &cfg, &image_path, &state_path).unwrap();
    let image = image::open(&image_path).unwrap();
    assert_eq!((image.width(), image.height()), (720, 405));
    let info: serde_json::Value =
        serde_json::from_slice(&std::fs::read(temp.path().join("state/render-info.json")).unwrap())
            .unwrap();
    assert!(info["placed"].as_u64().unwrap() > 0);
}
