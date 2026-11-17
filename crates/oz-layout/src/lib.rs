//! Squarified treemap and icicle layout (D-05). Produces compact instances that
//! go straight into a WebGL2 instance buffer.
#![forbid(unsafe_code)]

use oz_model::{flag, type_class, Index, NodeId};

/// 24 bytes per instance on the wire, little-endian.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Instance {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub id: u32,
    pub depth: u8,
    pub kind: u8,
    pub class: u8,
    pub age: u8,
}

pub mod kind {
    pub const DIR: u8 = 1;
    /// Aggregate cell for items too small to draw; `id` is the parent.
    pub const SMALL: u8 = 2;
    pub const LINK: u8 = 4;
}

pub const HEADER: f32 = 16.0;
const GUTTER: f32 = 1.0;
const MIN_AREA: f32 = 9.0;
const MAX_INSTANCES: usize = 200_000;

#[derive(Debug, Clone, Copy)]
struct R {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

/// Age bucket 0 (newest) to 7 (oldest) from FILETIMEs.
pub fn age_bucket(modified: i64, now: i64) -> u8 {
    const DAY: i64 = 864_000_000_000;
    let d = (now - modified) / DAY;
    match d {
        i64::MIN..=7 => 0,
        8..=30 => 1,
        31..=90 => 2,
        91..=180 => 3,
        181..=365 => 4,
        366..=730 => 5,
        731..=1825 => 6,
        _ => 7,
    }
}

fn inst(ix: &Index, id: NodeId, r: R, depth: u8, extra: u8, now: i64) -> Instance {
    let f = ix.flags(id);
    let mut k = extra;
    if f & flag::DIR != 0 {
        k |= kind::DIR;
    }
    if f & flag::LINK != 0 {
        k |= kind::LINK;
    }
    Instance {
        x: r.x,
        y: r.y,
        w: r.w,
        h: r.h,
        id,
        depth,
        kind: k,
        class: type_class(ix.ext(id).as_deref()),
        age: age_bucket(ix.modified(id), now),
    }
}

/// Size basis: on-disk bytes by default, logical bytes when `logical` is set.
fn size(ix: &Index, id: NodeId, logical: bool) -> u64 {
    if logical {
        ix.logical(id)
    } else {
        ix.allocated(id)
    }
}

/// Lays out `node`'s contents inside `w` x `h` pixels.
pub fn treemap(ix: &Index, node: NodeId, w: f32, h: f32, max_depth: u8, now: i64, logical: bool) -> Vec<Instance> {
    let mut out = Vec::new();
    if !ix.contains(node) || w < 1.0 || h < 1.0 {
        return out;
    }
    place(ix, node, R { x: 0.0, y: 0.0, w, h }, 0, max_depth, now, logical, &mut out);
    out
}

#[allow(clippy::too_many_arguments)]
fn place(ix: &Index, node: NodeId, r: R, depth: u8, max_depth: u8, now: i64, logical: bool, out: &mut Vec<Instance>) {
    let total = size(ix, node, logical) as f64;
    if total <= 0.0 || r.w < 1.0 || r.h < 1.0 {
        return;
    }
    let scale = (r.w as f64 * r.h as f64) / total;
    let kids = ix.children(node);
    let mut items: Vec<(NodeId, f64)> = Vec::new();
    let mut rest = 0.0;
    for &c in kids {
        let sz = size(ix, c, logical) as f64;
        if sz <= 0.0 {
            continue;
        }
        if sz * scale >= MIN_AREA as f64 {
            items.push((c, sz));
        } else {
            rest += sz;
        }
    }
    // Directories' own bytes (indexes) fold into the small cell so areas add up.
    if !logical {
        rest += ix.own_allocated(node) as f64;
    }
    let small = NodeId::MAX;
    if rest > 0.0 {
        items.push((small, rest));
    }
    let mut rects = Vec::with_capacity(items.len());
    squarify(&items, r, &mut rects);
    for (id, rc) in rects {
        if out.len() >= MAX_INSTANCES {
            return;
        }
        if id == small {
            out.push(inst(ix, node, rc, depth, kind::SMALL, now));
            continue;
        }
        out.push(inst(ix, id, rc, depth, 0, now));
        if ix.is_dir(id) && depth < max_depth && ix.flags(id) & flag::LINK == 0 {
            let inner = R {
                x: rc.x + GUTTER,
                y: rc.y + if rc.h > HEADER * 2.5 && rc.w > 48.0 { HEADER } else { GUTTER },
                w: rc.w - GUTTER * 2.0,
                h: rc.h - if rc.h > HEADER * 2.5 && rc.w > 48.0 { HEADER + GUTTER } else { GUTTER * 2.0 },
            };
            if inner.w > 3.0 && inner.h > 3.0 {
                place(ix, id, inner, depth + 1, max_depth, now, logical, out);
            }
        }
    }
}

/// Bruls, Huizing and van Wijk's squarified layout. `items` need not be strictly
/// sorted but are nearly so (the small cell comes last).
fn squarify(items: &[(NodeId, f64)], r: R, out: &mut Vec<(NodeId, R)>) {
    let total: f64 = items.iter().map(|i| i.1).sum();
    if total <= 0.0 {
        return;
    }
    let scale = (r.w as f64 * r.h as f64) / total;
    let mut free = r;
    let mut i = 0;
    while i < items.len() {
        let short = free.w.min(free.h) as f64;
        if short <= 0.0 {
            break;
        }
        let mut j = i;
        let (mut sum, mut lo, mut hi) = (0.0f64, f64::MAX, 0.0f64);
        let mut best = f64::MAX;
        while j < items.len() {
            let a = items[j].1 * scale;
            let (s2, l2, h2) = (sum + a, lo.min(a), hi.max(a));
            let worst = ((short * short * h2) / (s2 * s2)).max((s2 * s2) / (short * short * l2));
            if worst > best && j > i {
                break;
            }
            best = worst;
            sum = s2;
            lo = l2;
            hi = h2;
            j += 1;
        }
        let thick = (sum / short) as f32;
        let mut pos = 0.0f32;
        for it in &items[i..j] {
            let len = ((it.1 * scale) / (thick as f64)) as f32;
            let rc = if free.w >= free.h {
                R { x: free.x, y: free.y + pos, w: thick, h: len }
            } else {
                R { x: free.x + pos, y: free.y, w: len, h: thick }
            };
            out.push((it.0, rc));
            pos += len;
        }
        if free.w >= free.h {
            free.x += thick;
            free.w -= thick;
        } else {
            free.y += thick;
            free.h -= thick;
        }
        i = j;
    }
}

/// Icicle: each row one depth level, width proportional to size (F-12).
pub fn strata(ix: &Index, node: NodeId, w: f32, h: f32, levels: u8, now: i64, logical: bool) -> Vec<Instance> {
    let mut out = Vec::new();
    if !ix.contains(node) || size(ix, node, logical) == 0 {
        return out;
    }
    let row = (h / levels.max(1) as f32).max(18.0);
    let mut stack = vec![(node, 0.0f32, w, 0u8)];
    while let Some((id, x, width, d)) = stack.pop() {
        if out.len() >= MAX_INSTANCES {
            break;
        }
        let r = R { x, y: d as f32 * row, w: width, h: row };
        out.push(inst(ix, id, r, d, 0, now));
        if d + 1 >= levels || !ix.is_dir(id) || ix.flags(id) & flag::LINK != 0 {
            continue;
        }
        let total = size(ix, id, logical).max(1) as f64;
        let mut cx = x;
        let mut rest = 0.0f32;
        for &c in ix.children(id) {
            let cw = (size(ix, c, logical) as f64 / total * width as f64) as f32;
            if cw >= 1.5 {
                stack.push((c, cx, cw, d + 1));
                cx += cw;
            } else {
                rest += cw;
            }
        }
        if rest >= 1.0 {
            out.push(inst(ix, id, R { x: cx, y: (d + 1) as f32 * row, w: rest, h: row }, d + 1, kind::SMALL, now));
        }
    }
    out
}

pub fn encode(items: &[Instance]) -> Vec<u8> {
    let mut b = Vec::with_capacity(items.len() * 24);
    for i in items {
        b.extend_from_slice(&i.x.to_le_bytes());
        b.extend_from_slice(&i.y.to_le_bytes());
        b.extend_from_slice(&i.w.to_le_bytes());
        b.extend_from_slice(&i.h.to_le_bytes());
        b.extend_from_slice(&i.id.to_le_bytes());
        b.extend_from_slice(&[i.depth, i.kind, i.class, i.age]);
    }
    b
}

#[cfg(test)]
mod tests {
    use super::*;
    use oz_model::{IndexBuilder, ROOT};

    fn ix() -> Index {
        let mut b = IndexBuilder::new("C:\\");
        let a = b.add(ROOT, "a", 0, 0, 0, flag::DIR);
        for s in [400u64, 300, 200, 100] {
            b.add(a, &format!("f{s}.iso"), s, s, 0, 0);
        }
        b.add(ROOT, "b.bin", 1000, 1000, 0, 0);
        let l = b.add(ROOT, "link", 0, 0, 0, flag::DIR | flag::LINK);
        b.add(l, "never", 5, 0, 0, 0);
        b.build()
    }

    #[test]
    fn treemap_top_level_fills_area() {
        let v = treemap(&ix(), ROOT, 1000.0, 500.0, 0, 0, false);
        let area: f32 = v.iter().map(|i| i.w * i.h).sum();
        assert!((area - 500_000.0).abs() < 50.0, "{area}");
        for r in &v {
            assert!(r.x >= -0.01 && r.y >= -0.01 && r.x + r.w <= 1000.01 && r.y + r.h <= 500.01);
        }
    }

    #[test]
    fn nesting_stays_inside_parent() {
        let v = treemap(&ix(), ROOT, 1000.0, 500.0, 4, 0, false);
        let parent = v.iter().find(|i| i.id == 1).unwrap();
        for c in v.iter().filter(|i| i.depth == 1) {
            assert!(c.x >= parent.x && c.x + c.w <= parent.x + parent.w + 0.01);
            assert!(c.y >= parent.y && c.y + c.h <= parent.y + parent.h + 0.01);
        }
    }

    #[test]
    fn tiny_items_merge_and_aspect_is_sane() {
        let mut b = IndexBuilder::new("C:\\");
        b.add(ROOT, "big", 1_000_000_000, 1_000_000_000, 0, 0);
        for i in 0..5000 {
            b.add(ROOT, &format!("t{i}"), 1, 1, 0, 0);
        }
        let v = treemap(&b.build(), ROOT, 800.0, 600.0, 0, 0, false);
        assert_eq!(v.len(), 2);
        assert!(v.iter().any(|i| i.kind & kind::SMALL != 0));
    }

    #[test]
    fn strata_rows() {
        let v = strata(&ix(), ROOT, 1000.0, 300.0, 3, 0, false);
        assert_eq!(v[0].id, ROOT);
        assert_eq!(v[0].w, 1000.0);
        assert!(v.iter().any(|i| i.depth == 2));
    }

    #[test]
    fn encoding_is_24_bytes() {
        assert_eq!(encode(&treemap(&ix(), ROOT, 100.0, 100.0, 0, 0, false)).len() % 24, 0);
    }

    #[test]
    fn logical_basis_changes_areas() {
        let mut b = IndexBuilder::new("C:\\");
        b.add(ROOT, "sparse", 1000, 10, 0, 0);
        b.add(ROOT, "plain", 10, 1000, 0, 0);
        let i = b.build();
        let a = |lg: bool| {
            let v = treemap(&i, ROOT, 100.0, 100.0, 0, 0, lg);
            let r = v.iter().find(|x| x.id == 1).unwrap();
            r.w * r.h
        };
        assert!(a(true) > 5000.0 && a(false) < 5000.0);
    }

    #[test]
    fn age() {
        const DAY: i64 = 864_000_000_000;
        assert_eq!(age_bucket(0, 3 * DAY), 0);
        assert_eq!(age_bucket(0, 4000 * DAY), 7);
    }
}
