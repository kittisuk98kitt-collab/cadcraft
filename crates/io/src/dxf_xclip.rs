//! XCLIP boundaries of block references as AutoCAD stores them (DXF Reference): the INSERT's
//! extension dictionary holds an `ACAD_FILTER` dictionary whose `SPATIAL` entry is a
//! `SPATIAL_FILTER` object: the boundary points (2D, in the clip boundary's coordinate system),
//! display flag, front/back clip planes and two 4×3 matrices written column by column, the
//! inverse of the block reference's transform (world → block) and the clip boundary's transform.
//!
//! CADCraft keeps the boundary in block coordinates ([`cadcraft_doc::XClip`]) and also writes it
//! as its own xdata (exact, with Invert); the SPATIAL_FILTER is for other programs and for files
//! clipped elsewhere.

use std::collections::HashMap;
use std::sync::Arc;

use cadcraft_doc::{Drawing, EntityKind, Handle, Insert, XClip};
use cadcraft_dxf::{Section, Tag};
use cadcraft_geom::{Mat3, Vec2};

/// Most boundary points read from one filter.
const MAX_POINTS: usize = 100_000;

/// The block-to-world transform of `i` (2D).
fn block_xf(d: &Drawing, i: &Insert) -> Mat3 {
    let base = d.block(&i.block).map_or(Vec2::ZERO, |b| b.base.xy());
    i.transform(base)
}

/// A 2D affine transform as DXF's 4×3 matrix, column by column (x axis, y axis, z axis, origin).
fn matrix(m: &Mat3) -> [f64; 12] {
    [m.a, m.b, 0.0, m.c, m.d, 0.0, 0.0, 0.0, 1.0, m.e, m.f, 0.0]
}

/// The SPATIAL_FILTER object of `c` (on block reference `i`), handle `h`, owned by `owner`.
pub(crate) fn spatial_filter(d: &Drawing, i: &Insert, c: &XClip, h: &str, owner: &str) -> Vec<Tag> {
    let m = block_xf(d, i);
    let inv = m.inverse().unwrap_or(Mat3::IDENTITY);
    let pts: Vec<Vec2> = c.boundary.iter().map(|p| m.apply(*p)).collect();
    let mut t = vec![
        Tag::s(0, "SPATIAL_FILTER"),
        Tag::s(5, h),
        Tag::s(102, "{ACAD_REACTORS"),
        Tag::s(330, owner),
        Tag::s(102, "}"),
        Tag::s(330, owner),
        Tag::s(100, "AcDbFilter"),
        Tag::s(100, "AcDbSpatialFilter"),
        Tag::i(70, pts.len().min(32_000) as i64),
    ];
    for p in pts.iter().take(32_000) {
        t.push(Tag::f(10, p.x));
        t.push(Tag::f(20, p.y));
    }
    t.extend([Tag::f(210, 0.0), Tag::f(220, 0.0), Tag::f(230, 1.0), Tag::f(11, 0.0), Tag::f(21, 0.0), Tag::f(31, 0.0)]);
    t.push(Tag::i(71, i64::from(c.enabled)));
    t.push(Tag::i(72, 0));
    t.push(Tag::i(73, 0));
    for v in matrix(&inv).into_iter().chain(matrix(&Mat3::IDENTITY)) {
        t.push(Tag::f(40, v));
    }
    t
}

/// A record's handle and owner (before any `{` group's members).
fn head(tags: &[Tag]) -> (Option<String>, Option<String>) {
    let (mut h, mut o) = (None, None);
    let mut in_group = false;
    for t in tags.iter().take_while(|t| t.code < 1000) {
        match t.code {
            102 => in_group = t.str().trim().starts_with('{'),
            5 if h.is_none() && !in_group => h = Some(t.str().trim().to_ascii_uppercase()),
            330 if o.is_none() && !in_group => o = Some(t.str().trim().to_ascii_uppercase()),
            _ => {}
        }
    }
    (h, o)
}

/// A SPATIAL_FILTER's boundary in world coordinates (block coordinates after `inv`, the inverse
/// block transform it carries, when it has one) and display flag.
fn parse(tags: &[Tag]) -> Option<(Vec<Vec2>, bool, Option<Mat3>, Mat3)> {
    let start = tags.iter().position(|t| t.code == 100 && t.str().trim() == "AcDbSpatialFilter")?;
    let body = tags.get(start + 1..)?;
    let mut pts: Vec<Vec2> = Vec::new();
    let mut x: Option<f64> = None;
    let mut enabled = true;
    let mut front = false;
    let mut front_read = false;
    let mut m: Vec<f64> = Vec::new();
    for t in body.iter().take_while(|t| t.code < 1000) {
        match t.code {
            10 => x = t.str().trim().parse().ok(),
            20 => {
                if let (Some(px), Ok(py)) = (x.take(), t.str().trim().parse::<f64>())
                    && pts.len() < MAX_POINTS
                {
                    pts.push(Vec2::new(px, py));
                }
            }
            71 => enabled = t.str().trim() != "0",
            72 => front = t.str().trim() != "0",
            40 => {
                if front && !front_read && m.is_empty() {
                    front_read = true;
                } else if m.len() < 24
                    && let Ok(v) = t.str().trim().parse::<f64>()
                {
                    m.push(v);
                }
            }
            _ => {}
        }
    }
    let mat = |k: usize| -> Option<Mat3> {
        let v = m.get(k..k + 12)?;
        let mm = Mat3 { a: *v.first()?, b: *v.get(1)?, c: *v.get(3)?, d: *v.get(4)?, e: *v.get(9)?, f: *v.get(10)? };
        [mm.a, mm.b, mm.c, mm.d, mm.e, mm.f].iter().all(|x| x.is_finite()).then_some(mm)
    };
    let inv = mat(0);
    let bound = mat(12).unwrap_or(Mat3::IDENTITY);
    if !pts.iter().all(|p| p.is_finite()) {
        return None;
    }
    // Two points: the rectangle's corners.
    if let [a, b] = pts.as_slice() {
        pts = vec![*a, Vec2::new(b.x, a.y), *b, Vec2::new(a.x, b.y)];
    }
    (pts.len() >= 3).then_some((pts, enabled, inv, bound))
}

/// Give the block references of `secs` their AutoCAD clip boundaries (those without CADCraft's
/// own, read from xdata).
pub(crate) fn read(secs: &[Section], d: &mut Drawing) {
    let Some(objects) = secs.iter().find(|s| s.name == "OBJECTS") else { return };
    // Objects: handle → (type, owner, tags).
    let mut recs: Vec<(String, usize, usize)> = Vec::new();
    let mut open: Option<(String, usize)> = None;
    for (i, t) in objects.tags.iter().enumerate() {
        if t.code == 0 {
            if let Some((k, s)) = open.take() {
                recs.push((k, s + 1, i));
            }
            open = Some((t.str(), i));
        }
    }
    if let Some((k, s)) = open {
        recs.push((k, s + 1, objects.tags.len()));
    }
    let mut owner_of: HashMap<String, String> = HashMap::new();
    let mut filters: Vec<&[Tag]> = Vec::new();
    for (k, s, e) in &recs {
        let Some(tags) = objects.tags.get(*s..*e) else { continue };
        let (h, o) = head(tags);
        if let (Some(h), Some(o)) = (h, o) {
            owner_of.insert(h, o);
        }
        if k == "SPATIAL_FILTER" && filters.len() < 1_000_000 {
            filters.push(tags);
        }
    }
    let mut clips: HashMap<Handle, (Vec<Vec2>, bool, Option<Mat3>, Mat3)> = HashMap::new();
    for tags in filters {
        let (_, Some(o)) = head(tags) else { continue };
        // SPATIAL_FILTER → ACAD_FILTER dictionary → extension dictionary → block reference.
        let Some(ins) = owner_of.get(&o).and_then(|x| owner_of.get(x)) else { continue };
        let Some(h) = Handle::parse_hex(ins) else { continue };
        if let Some(c) = parse(tags) {
            clips.insert(h, c);
        }
    }
    if clips.is_empty() {
        return;
    }
    let bases: HashMap<String, Vec2> = d.blocks.iter().map(|(k, b)| (k.to_ascii_uppercase(), b.base.xy())).collect();
    let mut give = |store: &mut cadcraft_doc::EntityStore| {
        let hs: Vec<Handle> = store
            .iter()
            .filter(|e| matches!(&e.kind, EntityKind::Insert(i) if i.clip.is_none()) && clips.contains_key(&e.handle))
            .map(|e| e.handle)
            .collect();
        for h in hs {
            let Some((pts, enabled, inv, bound)) = clips.remove(&h) else { continue };
            let Some(EntityKind::Insert(i)) = store.get(h).map(|e| e.kind.clone()) else { continue };
            // World → block: the filter's own inverse transform, else the reference's.
            let base = bases.get(&i.block.to_ascii_uppercase()).copied().unwrap_or(Vec2::ZERO);
            let to_block = inv.or_else(|| i.transform(base).inverse()).unwrap_or(Mat3::IDENTITY);
            let boundary: Vec<Vec2> = pts.iter().map(|p| to_block.apply(bound.apply(*p))).collect();
            if !boundary.iter().all(|p| p.is_finite()) {
                continue;
            }
            store.modify(h, |e| {
                if let EntityKind::Insert(i) = &mut e.kind {
                    i.clip = Some(XClip { boundary: boundary.clone(), enabled, inverted: false });
                }
            });
        }
    };
    give(&mut d.model);
    for l in &mut d.layouts {
        give(&mut l.entities);
    }
    let names: Vec<String> = d.blocks.keys().cloned().collect();
    for n in names {
        if let Some(b) = d.blocks.get_mut(&n) {
            give(&mut Arc::make_mut(b).entities);
        }
    }
}
