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
    let mut order: Vec<&Item> = items.iter().collect();
    order.sort_by_key(|i| std::cmp::Reverse(i.width.max(i.height)));
    for item in order {
        let (w, h) = if item.rotated {
            (item.height, item.width)
        } else {
            (item.width, item.height)
        };
        if w + 2 * margin > width || h + 2 * margin > height {
            continue;
        }
        let old = memory.positions.get(&item.key).copied();
        let center = old
            .map(|(x, y, _)| ((x * width as f32) as i32, (y * height as f32) as i32))
            .unwrap_or((width as i32 / 2, height as i32 / 2));
        let mut found = None;
        // Fermat spiral with the last location as its origin keeps retained words visually calm.
        let golden = 2.399963f32;
        let step = cell as f32 * 1.4;
        let tries = 24000usize;
        for n in 0..tries {
            let (cx, cy) = if n == 0 {
                center
            } else {
                let r = step * (n as f32).sqrt();
                let a = n as f32 * golden;
                (
                    (center.0 as f32 + r * a.cos()) as i32,
                    (center.1 as f32 + r * a.sin()) as i32,
                )
            };
            let mut x = cx - w as i32 / 2;
            let mut y = cy - h as i32 / 2;
            if let Some((ox, oy, _)) = old {
                let stability = stability.clamp(0.0, 0.95);
                let acx = x as f32;
                let acy = y as f32;
                x = (acx * (1.0 - stability) + ox * width as f32 * stability - w as f32 / 2.0)
                    as i32;
                y = (acy * (1.0 - stability) + oy * height as f32 * stability - h as f32 / 2.0)
                    as i32;
            }
            if x < margin as i32
                || y < margin as i32
                || x + w as i32 + margin as i32 > width as i32
                || y + h as i32 + margin as i32 > height as i32
            {
                continue;
            }
            let rect = BoxRect {
                x: x as u32,
                y: y as u32,
                w,
                h,
            };
            if clear(&mask, gw, gh, rect, padding, cell) {
                mark(&mut mask, gw, gh, rect, padding, cell);
                found = Some(rect);
                break;
            }
        }
        if let Some(rect) = found {
            out.insert(
                item.key.clone(),
                Placement {
                    rect,
                    rotated: item.rotated,
                },
            );
        }
    }
    out
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
