use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct BoxRect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}
#[derive(Clone, Debug)]
pub struct Item {
    pub key: String,
    pub width: u32,
    pub height: u32,
    pub rotated: bool,
}
#[derive(Clone, Debug)]
pub struct Placement {
    pub rect: BoxRect,
    pub rotated: bool,
}
#[derive(Default, Serialize, Deserialize)]
pub struct LayoutMemory {
    pub width: u32,
    pub height: u32,
    pub positions: HashMap<String, (f32, f32, bool)>,
    #[serde(default)]
    pub font_sizes: HashMap<String, f32>,
}

pub fn place(
    items: &[Item],
    width: u32,
    height: u32,
    margin: u32,
    padding: u32,
    memory: &LayoutMemory,
    stability: f32,
) -> HashMap<String, Placement> {
    let cell = 4u32;
    let gw = (width + cell - 1) / cell;
    let gh = (height + cell - 1) / cell;
    let mut mask = vec![0u8; (gw * gh) as usize];
    let mut out = HashMap::new();
    let stability = stability.clamp(0.0, 0.95);
    let mut order: Vec<&Item> = items.iter().collect();
    order.sort_by(|a, b| {
        b.width
            .max(b.height)
            .cmp(&a.width.max(a.height))
            .then_with(|| a.key.cmp(&b.key))
    });
    // Reserve legal anchors first. A newly enlarged word must not push every
    // unchanged word away merely because it is sorted first this time.
    if stability > 0.0 {
        for item in &order {
            let Some(&(cx, cy, _)) = memory.positions.get(&item.key) else {
                continue;
            };
            let (w, h) = dimensions(item);
            let x = (cx * width as f32 - w as f32 / 2.0).round() as i32;
            let y = (cy * height as f32 - h as f32 / 2.0).round() as i32;
            if let Some(rect) = bounded_rect(x, y, w, h, width, height, margin) {
                if clear(&mask, gw, gh, rect, padding, cell) {
                    mark(&mut mask, gw, gh, rect, padding, cell);
                    out.insert(
                        item.key.clone(),
                        Placement {
                            rect,
                            rotated: item.rotated,
                        },
                    );
                }
            }
        }
    }
    for item in order {
        if out.contains_key(&item.key) {
            continue;
        }
        let (w, h) = dimensions(item);
        if w.saturating_add(2 * margin) > width || h.saturating_add(2 * margin) > height {
            continue;
        }
        let old = if stability > 0.0 {
            memory.positions.get(&item.key).copied()
        } else {
            None
        };
        let center = old
            .map(|(x, y, _)| (x * width as f32, y * height as f32))
            .unwrap_or((width as f32 / 2.0, height as f32 / 2.0));
        // Search around the old center, subtracting half the dimensions only
        // once. Higher stability searches nearby positions more finely.
        let step = cell as f32
            * 1.4
            * if old.is_some() {
                1.0 - stability * 0.5
            } else {
                1.0
            };
        for n in 0..24000usize {
            let radius = step * (n as f32).sqrt();
            let angle = n as f32 * 2.399963f32;
            let x = (center.0 + radius * angle.cos() - w as f32 / 2.0).round() as i32;
            let y = (center.1 + radius * angle.sin() - h as f32 / 2.0).round() as i32;
            let Some(rect) = bounded_rect(x, y, w, h, width, height, margin) else {
                continue;
            };
            if clear(&mask, gw, gh, rect, padding, cell) {
                mark(&mut mask, gw, gh, rect, padding, cell);
                out.insert(
                    item.key.clone(),
                    Placement {
                        rect,
                        rotated: item.rotated,
                    },
                );
                break;
            }
        }
    }
    out
}
fn dimensions(item: &Item) -> (u32, u32) {
    if item.rotated {
        (item.height, item.width)
    } else {
        (item.width, item.height)
    }
}

fn bounded_rect(
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    width: u32,
    height: u32,
    margin: u32,
) -> Option<BoxRect> {
    if x < margin as i32
        || y < margin as i32
        || x as i64 + w as i64 + margin as i64 > width as i64
        || y as i64 + h as i64 + margin as i64 > height as i64
    {
        return None;
    }
    Some(BoxRect {
        x: x as u32,
        y: y as u32,
        w,
        h,
    })
}

fn cells(r: BoxRect, p: u32, c: u32, gw: u32, gh: u32) -> (u32, u32, u32, u32) {
    let x0 = r.x.saturating_sub(p) / c;
    let y0 = r.y.saturating_sub(p) / c;
    let x1 = ((r.x + r.w + p + c - 1) / c).min(gw);
    let y1 = ((r.y + r.h + p + c - 1) / c).min(gh);
    (x0, y0, x1, y1)
}
fn clear(m: &[u8], gw: u32, gh: u32, r: BoxRect, p: u32, c: u32) -> bool {
    let (x0, y0, x1, y1) = cells(r, p, c, gw, gh);
    (y0..y1).all(|y| (x0..x1).all(|x| m[(y * gw + x) as usize] == 0))
}
fn mark(m: &mut [u8], gw: u32, gh: u32, r: BoxRect, p: u32, c: u32) {
    let (x0, y0, x1, y1) = cells(r, p, c, gw, gh);
    for y in y0..y1 {
        for x in x0..x1 {
            m[(y * gw + x) as usize] = 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unchanged_odd_sized_words_do_not_drift_across_refreshes() {
        let items = (0..20)
            .map(|i| Item {
                key: format!("worker-{i}"),
                width: 47,
                height: 23,
                rotated: i % 5 == 0,
            })
            .collect::<Vec<_>>();
        let mut memory = LayoutMemory::default();
        let initial = place(&items, 800, 450, 12, 3, &memory, 0.72);
        assert_eq!(initial.len(), items.len());
        for _ in 0..20 {
            memory.positions = initial
                .iter()
                .map(|(key, p)| {
                    (
                        key.clone(),
                        (
                            (p.rect.x as f32 + p.rect.w as f32 / 2.0) / 800.0,
                            (p.rect.y as f32 + p.rect.h as f32 / 2.0) / 450.0,
                            p.rotated,
                        ),
                    )
                })
                .collect();
            let next = place(&items, 800, 450, 12, 3, &memory, 0.72);
            for (key, old) in &initial {
                let new = &next[key];
                assert_eq!(
                    (new.rect.x, new.rect.y, new.rotated),
                    (old.rect.x, old.rect.y, old.rotated)
                );
            }
        }
    }

    #[test]
    fn new_large_word_does_not_displace_legal_existing_anchor() {
        let memory = LayoutMemory {
            positions: HashMap::from([("existing".into(), (0.5, 0.5, false))]),
            ..Default::default()
        };
        let items = [
            Item {
                key: "new".into(),
                width: 250,
                height: 70,
                rotated: false,
            },
            Item {
                key: "existing".into(),
                width: 50,
                height: 20,
                rotated: false,
            },
        ];
        let placed = place(&items, 800, 450, 12, 3, &memory, 0.72);
        assert_eq!(
            (placed["existing"].rect.x, placed["existing"].rect.y),
            (375, 215)
        );
        assert!(placed.contains_key("new"));
    }

    #[test]
    fn placement_is_in_bounds_and_collision_free() {
        let items = (0..30)
            .map(|n| Item {
                key: n.to_string(),
                width: 45,
                height: 18,
                rotated: false,
            })
            .collect::<Vec<_>>();
        let p = place(&items, 480, 270, 8, 3, &LayoutMemory::default(), 0.4);
        let vals = p.values().collect::<Vec<_>>();
        assert!(
            vals.iter()
                .all(|x| x.rect.x + x.rect.w <= 480 && x.rect.y + x.rect.h <= 270)
        );
        for i in 0..vals.len() {
            for j in i + 1..vals.len() {
                let a = vals[i].rect;
                let b = vals[j].rect;
                assert!(
                    a.x + a.w + 3 <= b.x
                        || b.x + b.w + 3 <= a.x
                        || a.y + a.h + 3 <= b.y
                        || b.y + b.h + 3 <= a.y
                );
            }
        }
    }

    #[test]
    fn hundreds_of_equal_weight_items_remain_bounded_and_separate() {
        let items = (0..300)
            .map(|n| Item {
                key: format!("worker-{n}"),
                width: 44,
                height: 18,
                rotated: n % 11 == 0,
            })
            .collect::<Vec<_>>();
        let placed = place(&items, 1920, 1080, 12, 3, &LayoutMemory::default(), 0.5);
        assert!(placed.len() >= 250, "placed {} of 300", placed.len());
        let vals = placed.values().collect::<Vec<_>>();
        for i in 0..vals.len() {
            let a = vals[i].rect;
            assert!(a.x + a.w <= 1920 && a.y + a.h <= 1080);
            for b in vals.iter().skip(i + 1).map(|x| x.rect) {
                assert!(
                    a.x + a.w + 3 <= b.x
                        || b.x + b.w + 3 <= a.x
                        || a.y + a.h + 3 <= b.y
                        || b.y + b.h + 3 <= a.y
                );
            }
        }
    }

    #[test]
    fn overlong_item_is_skipped_and_removed_items_do_not_block_reflow() {
        let memory = LayoutMemory {
            width: 800,
            height: 450,
            positions: HashMap::from([("gone".into(), (0.5, 0.5, false))]),
            ..LayoutMemory::default()
        };
        let items = [
            Item {
                key: "extremely-long-real-process-name".into(),
                width: 900,
                height: 18,
                rotated: false,
            },
            Item {
                key: "survivor".into(),
                width: 100,
                height: 24,
                rotated: false,
            },
        ];
        let placed = place(&items, 800, 450, 12, 3, &memory, 0.72);
        assert!(!placed.contains_key("extremely-long-real-process-name"));
        assert!(placed.contains_key("survivor"));
    }
}
