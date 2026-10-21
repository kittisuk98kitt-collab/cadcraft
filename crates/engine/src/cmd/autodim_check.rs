//! DIMCHECK: the positions of a view that no dimension ties down, holes without a size, and
//! dimension text that disagrees with the geometry.
//!
//! Every outline end point, hole centre and unsized arc centre must be reachable along X (and
//! along Y) from the rest of its view through dimensions. What a shop drawing gives otherwise
//! counts too: pitch dimensions, symmetry about a centre line, a plate thickness note (`t=12`)
//! and steel section sizes, sized holes (their edges follow from the centre), fillets and 45°
//! chamfers with an R/C note, slots with a `WxL SLOT` note, points on sized curves, weld beads,
//! the same length dimensioned in another view, and views in projection sharing coordinates.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use cadcraft_doc::{DimKind, Drawing, Entity, EntityKind};
use cadcraft_geom::{Bounds2, Vec2};
use serde_json::{Value, json};

use super::autodim::{Feat, Kind, collect, lead_number, numbers_after, numbers_before};
use super::*;
use crate::{Result, Session};

/// Drafting inaccuracy accepted between coordinates that should be equal.
const SNAP: f64 = 0.2;
const TOL: f64 = 0.06;

/// A float as a grid index, kept far from the i64 limits so neighbours (±1) and differences
/// can't overflow.
fn qi(v: f64) -> i64 {
    let k = v.round();
    if k.is_finite() { k.clamp(-1.0e15, 1.0e15) as i64 } else { 0 }
}

fn q(v: f64) -> i64 {
    qi(v / SNAP)
}

/// Union-find over quantised coordinates.
#[derive(Default)]
struct Uf {
    p: HashMap<i64, i64>,
}
impl Uf {
    fn find(&mut self, a: i64) -> i64 {
        let mut r = a;
        let mut n = 0;
        while let Some(&up) = self.p.get(&r) {
            if up == r || n > 1_000_000 {
                break;
            }
            r = up;
            n += 1;
        }
        // Path compression.
        let mut x = a;
        while let Some(&up) = self.p.get(&x) {
            if up == x {
                break;
            }
            self.p.insert(x, r);
            x = up;
        }
        self.p.entry(r).or_insert(r);
        r
    }
    fn union(&mut self, a: i64, b: i64) {
        let (a, b) = (self.find(a), self.find(b));
        if a != b {
            self.p.insert(a, b);
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct Issue {
    pub kind: &'static str,
    pub msg: String,
    pub at: Vec2,
}

/// Plain text of TEXT/MTEXT contents, with %%c and ∅ as Ø.
fn plain(s: &str) -> String {
    cadcraft_fonts::plain_mtext(s).replace("%%c", "Ø").replace("%%C", "Ø").replace('∅', "Ø").replace('\n', " ")
}

fn rkey(r: f64) -> i64 {
    let k = (r * 100.0).round();
    if k.is_finite() && k.abs() < 9.0e15 { k as i64 } else { 0 }
}
fn rkey1(r: f64) -> i64 {
    let k = (r * 10.0).round();
    if k.is_finite() && k.abs() < 9.0e15 { k as i64 } else { 0 }
}

struct Dim {
    kind: DimKind,
    p13: Vec2,
    p14: Vec2,
    defpt: Vec2,
    p15: Vec2,
    text_at: Vec2,
    text: String,
    meas: f64,
}

struct View {
    feats: Vec<usize>,
    bbox: Bounds2,
    dims: Vec<usize>,
    xl: Vec<(i64, i64)>,
    yl: Vec<(i64, i64)>,
    rx: BTreeMap<i64, (Vec2, String)>,
    ry: BTreeMap<i64, (Vec2, String)>,
    polar: bool,
}

fn seg_dist(p: Vec2, a: Vec2, b: Vec2) -> f64 {
    let ab = b - a;
    let l2 = ab.dot(ab);
    if l2 < 1e-24 {
        return p.dist(a);
    }
    let t = ((p - a).dot(ab) / l2).clamp(0.0, 1.0);
    p.dist(a + ab * t)
}

fn title_word(t: &str) -> bool {
    let u = t.to_uppercase();
    ["DRAWING", "図番", "SCALE", "尺度", "TOLERANCE", "APPROVED"].iter().any(|w| u.contains(w))
}

/// A block whose definition holds title-block words: a sheet frame / title block.
fn titled_block(d: &Drawing, name: &str) -> bool {
    d.block(name).is_some_and(|b| {
        b.entities.iter().any(|e| match &e.kind {
            EntityKind::Text(t) => title_word(&t.value),
            EntityKind::MText(m) => title_word(&m.contents),
            EntityKind::AttDef(a) => title_word(&a.text.value) || title_word(&a.tag),
            _ => false,
        })
    })
}

/// The sheet frames and title blocks drawn as line work: connected lines, mostly on the axes,
/// that hold title-block words and other geometry. Returns (feature indices, frame boxes).
fn frames_from_lines(feats: &[Feat], texts: &[(Vec2, String)]) -> (HashSet<usize>, Vec<Bounds2>) {
    let segs: Vec<(usize, Vec2, Vec2)> =
        feats.iter().enumerate().filter(|(_, f)| f.kind == Kind::Line && !f.hidden).map(|(i, f)| (i, f.a, f.b)).collect();
    if segs.is_empty() {
        return (HashSet::new(), Vec::new());
    }
    let mut all = Bounds2::EMPTY;
    for (_, a, b) in &segs {
        all.add(*a);
        all.add(*b);
    }
    let size = all.width().max(all.height()).max(1.0);
    let cell = size / 150.0;
    let key = |p: Vec2| (qi((p.x / cell).floor()), qi((p.y / cell).floor()));
    let mut grid: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
    for (k, (_, a, b)) in segs.iter().enumerate() {
        let (x0, y0) = key(Vec2::new(a.x.min(b.x), a.y.min(b.y)));
        let (x1, y1) = key(Vec2::new(a.x.max(b.x), a.y.max(b.y)));
        if (x1 - x0) * (y1 - y0) > 40_000 {
            continue;
        }
        for gx in x0..=x1 {
            for gy in y0..=y1 {
                let v = grid.entry((gx, gy)).or_default();
                if v.len() < 400 {
                    v.push(k);
                }
            }
        }
    }
    let tol = (size * 1e-5).max(0.01);
    let mut uf = Uf::default();
    for (k, (_, a, b)) in segs.iter().enumerate() {
        uf.find(k as i64);
        for p in [*a, *b] {
            if let Some(cands) = grid.get(&key(p)) {
                for m in cands {
                    if *m != k
                        && let Some((_, c, e)) = segs.get(*m)
                        && seg_dist(p, *c, *e) < tol
                    {
                        uf.union(k as i64, *m as i64);
                    }
                }
            }
        }
    }
    let mut comp: HashMap<i64, Vec<usize>> = HashMap::new();
    for k in 0..segs.len() {
        comp.entry(uf.find(k as i64)).or_default().push(k);
    }
    let title: Vec<Vec2> = texts.iter().filter(|(_, t)| title_word(t)).map(|(p, _)| *p).collect();
    let boxes: Vec<(Vec<usize>, Bounds2)> = comp
        .into_values()
        .map(|ks| {
            let mut b = Bounds2::EMPTY;
            for k in &ks {
                if let Some((_, a, e)) = segs.get(*k) {
                    b.add(*a);
                    b.add(*e);
                }
            }
            (ks, b)
        })
        .collect();
    let total = (all.width() * all.height()).max(1.0);
    let mut out = HashSet::new();
    let mut frames = Vec::new();
    for (i, (ks, b0)) in boxes.iter().enumerate() {
        if ks.len() < 4 {
            continue;
        }
        let area = b0.width() * b0.height();
        let inside = boxes.iter().enumerate().filter(|(j, (k2, b1))| *j != i && k2.len() >= 3 && b0.contains_box(b1)).count();
        let axis = ks.iter().filter(|k| segs.get(**k).is_some_and(|(_, a, b)| (a.x - b.x).abs() < 1e-6 || (a.y - b.y).abs() < 1e-6)).count() as f64
            / ks.len() as f64;
        let titled = title.iter().any(|p| b0.contains(*p));
        if axis > 0.7 && ((titled && inside >= 1) || (inside >= 3 && area > 0.3 * total)) {
            out.extend(ks.iter().filter_map(|k| segs.get(*k).map(|s| s.0)));
            frames.push(*b0);
        }
    }
    (out, frames)
}

pub(super) fn run_dimcheck(s: &mut Session, p: &Value) -> Result<Value> {
    let hs = targets(s, p)?;
    let mark = bool_or(p, "mark", true);
    let quiet = bool_or(p, "quiet", false);
    let th = super::autodim::style_height(s)?;
    let (issues, nviews) = {
        let d = s.doc()?;
        let whole = hs.is_empty();
        let ents: Vec<Entity> = if whole {
            d.model.iter().map(|e| (**e).clone()).collect()
        } else {
            hs.iter().filter_map(|h| d.entity(*h)).map(|e| (**e).clone()).collect()
        };
        check(d, &ents, whole)
    };
    if !quiet {
        if issues.is_empty() {
            s.echo(format!("DIMCHECK: {nviews} view(s) checked, every position is dimensioned."));
        } else {
            s.echo(format!("DIMCHECK: {nviews} view(s), {} finding(s):", issues.len()));
            for i in issues.iter().take(200) {
                s.echo(format!("  [{}] {}", i.kind, i.msg));
            }
        }
    }
    let mut marks = Vec::new();
    if mark && !issues.is_empty() {
        {
            let d = s.doc_mut()?;
            if d.layer("DIMCHECK").is_none() {
                let mut l = cadcraft_doc::Layer::new("DIMCHECK");
                l.color = cadcraft_color::Color::Index(1);
                l.plot = false;
                d.layers.push(l);
            }
        }
        for i in &issues {
            let k = EntityKind::Circle(cadcraft_doc::Circle { center: cadcraft_geom::Vec3::new(i.at.x, i.at.y, 0.0), radius: th * 1.6 });
            let h = s.add_entity(k)?;
            s.doc_mut()?.modify_entity(h, |e| {
                e.common.layer = "DIMCHECK".into();
                e.common.color = cadcraft_color::Color::ByLayer;
            })?;
            marks.push(h.hex());
        }
        if !quiet {
            s.echo("Findings are circled on layer DIMCHECK (not plotted); erase or freeze it when done.");
        }
    }
    Ok(json!({
        "views": nviews,
        "issues": issues.iter().map(|i| json!({ "kind": i.kind, "message": i.msg, "at": [i.at.x, i.at.y] })).collect::<Vec<_>>(),
        "marks": marks,
    }))
}

/// The findings for `ents` (one view, or with `whole` the drawing split into views).
pub(super) fn check(d: &Drawing, ents: &[Entity], whole: bool) -> (Vec<Issue>, usize) {
    let dimscale = d.header.f64("DIMSCALE", 1.0);
    // ---- dimensions
    let mut dims: Vec<Dim> = Vec::new();
    for e in ents {
        if let EntityKind::Dimension(dm) = &e.kind {
            let st = d.dim_style(&dm.style).cloned().unwrap_or_default().with_overrides(&dm.overrides);
            let g = cadcraft_render::dimension_geometry(dm, &st, dimscale);
            dims.push(Dim {
                kind: dm.kind,
                p13: dm.p13.xy(),
                p14: dm.p14.xy(),
                defpt: dm.defpt.xy(),
                p15: dm.p15.xy(),
                text_at: g.text_pos,
                text: dm.text.clone(),
                meas: g.measurement,
            });
        }
    }
    let lin = |k: &DimKind| matches!(k, DimKind::Linear { .. } | DimKind::Aligned);
    let dim_pts: Vec<Vec2> = dims.iter().flat_map(|m| if lin(&m.kind) { vec![m.p13, m.p14] } else { vec![m.p15, m.defpt] }).collect();
    // ---- geometry: drawn directly, and inside the block references dimensions attach to
    let mut feats: Vec<Feat> = Vec::new();
    let mut frames: Vec<Bounds2> = Vec::new();
    let mut plain_ents: Vec<Entity> = Vec::new();
    for e in ents {
        match &e.kind {
            EntityKind::Insert(ins) => {
                let b = cadcraft_doc::entity_bounds(d, e, 0);
                if b.is_empty() {
                    continue;
                }
                if titled_block(d, &ins.block) {
                    frames.push(b);
                    continue;
                }
                if whole {
                    let bx = b.expand(TOL);
                    let inside = dim_pts.iter().filter(|p| bx.contains(**p)).count();
                    if inside == 0 {
                        continue;
                    }
                    // A frame block round most of the dimensions, far larger than what they span,
                    // is the sheet, not a part.
                    let held = Bounds2::from_points(dim_pts.iter().copied().filter(|p| bx.contains(*p)));
                    if inside > 6 && b.width() * b.height() > 4.0 * (held.width() * held.height()).max(1e-9) {
                        frames.push(b);
                        continue;
                    }
                }
                collect(d, std::slice::from_ref(e), &mut feats, 0, None);
            }
            _ => plain_ents.push(e.clone()),
        }
    }
    collect(d, &plain_ents, &mut feats, 0, None);
    feats.retain(|f| !(whole && f.layer_annot));
    // Centre lines: axes of symmetry.
    let centre_x: Vec<(f64, f64, f64)> = feats
        .iter()
        .filter(|f| f.centre && f.kind == Kind::Line && (f.a.x - f.b.x).abs() < TOL)
        .map(|f| (f.a.x, f.a.y.min(f.b.y), f.a.y.max(f.b.y)))
        .collect();
    let centre_y: Vec<(f64, f64, f64)> = feats
        .iter()
        .filter(|f| f.centre && f.kind == Kind::Line && (f.a.y - f.b.y).abs() < TOL)
        .map(|f| (f.a.y, f.a.x.min(f.b.x), f.a.x.max(f.b.x)))
        .collect();
    // Weld beads (solids, solid hatches): their corners need no dimension.
    let mut weld: Vec<Vec2> = Vec::new();
    // ---- texts, leaders, callouts
    let mut texts: Vec<(Vec2, String)> = Vec::new();
    let mut leaders: Vec<Vec<Vec2>> = Vec::new();
    for e in ents {
        match &e.kind {
            EntityKind::Text(t) => texts.push((t.insert.xy(), plain(&t.value))),
            EntityKind::MText(m) => texts.push((m.insert.xy(), plain(&m.contents))),
            EntityKind::Leader(l) if l.vertices.len() >= 2 => leaders.push(l.vertices.iter().map(|v| v.xy()).collect()),
            EntityKind::MLeader(ml) => {
                for l in &ml.leaders {
                    let mut v: Vec<Vec2> = l.iter().map(|p| p.xy()).collect();
                    if !v.is_empty() {
                        v.push(ml.landing.xy());
                        leaders.push(v);
                    }
                }
                if let Some(t) = &ml.text {
                    texts.push((t.insert.xy(), plain(&t.contents)));
                }
            }
            EntityKind::Line(l) if super::autodim::annotation_layer(&e.common.layer) && e.common.layer.to_uppercase().contains("CUT") => {
                leaders.push(vec![l.a.xy(), l.b.xy()]);
                leaders.push(vec![l.b.xy(), l.a.xy()]);
            }
            EntityKind::Solid(so) => weld.extend(so.corners.iter().map(|c| c.xy())),
            EntityKind::Hatch(h) if h.solid => {
                for lp in &h.loops {
                    weld.extend(lp.vertices.iter().map(|v| v.p));
                }
            }
            EntityKind::Insert(ins) => {
                for a in &ins.attribs {
                    texts.push((a.text.insert.xy(), plain(&a.text.value)));
                }
            }
            _ => {}
        }
    }
    let near_text = |p: Vec2, maxd: f64| -> Option<String> {
        texts.iter().map(|(tp, t)| (tp.dist(p), t)).filter(|(dd, _)| *dd < maxd).min_by(|a, b| a.0.total_cmp(&b.0)).map(|(_, t)| t.clone())
    };
    let mut callouts: Vec<(Vec2, String)> = Vec::new();
    for vs in &leaders {
        let (Some(tip), Some(tail)) = (vs.first(), vs.last()) else { continue };
        let reach = 40.0_f64.max(tip.dist(*tail) * 0.5);
        if let Some(t) = near_text(*tail, reach) {
            callouts.push((*tip, t));
        }
    }
    // Leaders drawn into the dimension texts themselves (QLEADER text is MText, read above).

    // ---- frames and stray construction lines (whole drawing only)
    let mut drop: HashSet<usize> = HashSet::new();
    let mut issues: Vec<Issue> = Vec::new();
    if whole {
        let (f_lines, f_boxes) = frames_from_lines(&feats, &texts);
        drop = f_lines;
        frames.extend(f_boxes);
        for (i, f) in feats.iter().enumerate() {
            if f.kind != Kind::Line || drop.contains(&i) {
                continue;
            }
            if (f.a.x - f.b.x).abs() > TOL && (f.a.y - f.b.y).abs() > TOL {
                let l = f.a.dist(f.b);
                for fr in &frames {
                    let diag = (fr.max - fr.min).len();
                    if l > 0.4 * diag && fr.min.x - 1.0 <= f.a.x.min(f.b.x) && f.a.x.max(f.b.x) <= fr.max.x + 1.0 {
                        drop.insert(i);
                        issues.push(Issue {
                            kind: "stray",
                            msg: "a long slanted line crosses the sheet (a construction line left over?)".into(),
                            at: (f.a + f.b) * 0.5,
                        });
                        break;
                    }
                }
            }
        }
    }
    // ---- views: overlapping boxes of outline geometry
    let vis: Vec<usize> = (0..feats.len()).filter(|i| !drop.contains(i)).collect();
    let boxes: Vec<Bounds2> = feats.iter().map(Feat::bounds).collect();
    let mut views: Vec<View> = Vec::new();
    if whole {
        let mut order = vis.clone();
        order.sort_by(|a, b| boxes.get(*a).map(|x| x.min.x).unwrap_or(0.0).total_cmp(&boxes.get(*b).map(|x| x.min.x).unwrap_or(0.0)));
        let mut uf = Uf::default();
        let mut active: Vec<usize> = Vec::new();
        let e = 0.5;
        for i in order {
            let Some(b) = boxes.get(i).copied() else { continue };
            active.retain(|j| boxes.get(*j).is_some_and(|c| c.max.x + e >= b.min.x));
            if active.len() > 20_000 {
                active.drain(..10_000);
            }
            uf.find(i as i64);
            for j in &active {
                if let Some(c) = boxes.get(*j)
                    && c.min.x - e <= b.max.x
                    && b.min.x <= c.max.x + e
                    && c.min.y - e <= b.max.y
                    && b.min.y <= c.max.y + e
                {
                    uf.union(i as i64, *j as i64);
                }
            }
            active.push(i);
        }
        let mut groups: BTreeMap<i64, Vec<usize>> = BTreeMap::new();
        for i in &vis {
            groups.entry(uf.find(*i as i64)).or_default().push(*i);
        }
        for g in groups.into_values() {
            let mut bb = Bounds2::EMPTY;
            for i in &g {
                if let Some(b) = boxes.get(*i) {
                    bb = bb.union(b);
                }
            }
            views.push(View {
                feats: g,
                bbox: bb,
                dims: Vec::new(),
                xl: Vec::new(),
                yl: Vec::new(),
                rx: BTreeMap::new(),
                ry: BTreeMap::new(),
                polar: false,
            });
        }
    } else {
        let mut bb = Bounds2::EMPTY;
        for i in &vis {
            if let Some(b) = boxes.get(*i) {
                bb = bb.union(b);
            }
        }
        views.push(View {
            feats: vis.clone(),
            bbox: bb,
            dims: Vec::new(),
            xl: Vec::new(),
            yl: Vec::new(),
            rx: BTreeMap::new(),
            ry: BTreeMap::new(),
            polar: false,
        });
    }
    // A view inside another one's box (symbols, small parts drawn on it) belongs to it.
    if whole {
        views.sort_by(|a, b| (b.bbox.width() * b.bbox.height()).total_cmp(&(a.bbox.width() * a.bbox.height())));
        let mut merged: Vec<View> = Vec::new();
        for v in views {
            let symbol = v.feats.iter().all(|i| feats.get(*i).is_some_and(|f| f.block_area.is_some()));
            if let Some(host) = merged.iter_mut().find(|h| symbol && h.bbox.contains_box(&v.bbox)) {
                host.feats.extend(v.feats);
            } else {
                merged.push(v);
            }
        }
        views = merged;
    }
    let view_of = |views: &[View], p: Vec2| -> Option<usize> { views.iter().position(|v| v.bbox.expand(1.0).contains(p)) };
    for (k, m) in dims.iter().enumerate() {
        let v = if lin(&m.kind) {
            view_of(&views, m.p13).or_else(|| view_of(&views, m.p14))
        } else {
            view_of(&views, m.p15).or_else(|| view_of(&views, m.defpt))
        };
        if let Some(v) = v.or(if whole { None } else { Some(0) })
            && let Some(view) = views.get_mut(v)
        {
            view.dims.push(k);
        }
    }
    let in_frame = |b: &Bounds2| frames.iter().any(|f| f.contains_box(b));
    views.retain(|v| !v.dims.is_empty() || !whole);
    if whole && views.iter().any(|v| in_frame(&v.bbox)) {
        views.retain(|v| in_frame(&v.bbox));
    }
    // ---- notes that give sizes
    let mut thick: Vec<f64> = Vec::new();
    let mut sections: Vec<Vec2> = Vec::new();
    for (tp, t) in &texts {
        for v in numbers_after(t, "T") {
            thick.push(v);
        }
        // Steel sections: C75x40xt5, L65x65xt6, □150x100xt6, FB32x6, H200x100x5.5x8.
        let u = t.to_uppercase();
        if !["SLOT", "DRILL", "THRU", "BOLT", "TAP"].iter().any(|w| u.contains(w)) {
            for word in u.split_whitespace() {
                let w = word.trim_start_matches(['C', 'L', 'H', 'I', 'U', 'F', 'B', '□', '[', ']', 'P', 'O', '(']);
                if w.len() < word.len() || word.starts_with(|c: char| c.is_ascii_digit()) {
                    let parts: Vec<f64> = w.split(['X', '×']).filter_map(|x| lead_number(x.trim_start_matches('T')).map(|(v, _)| v)).collect();
                    if parts.len() >= 2 && w.contains(['X', '×']) && w.len() < word.len() {
                        thick.extend(parts.iter().copied());
                        sections.push(*tp);
                    }
                }
            }
        }
    }
    thick.retain(|t| t.is_finite() && *t > 0.0);
    let thick_max = thick.iter().copied().fold(0.0_f64, f64::max);
    let mut all_text: Vec<String> = callouts.iter().map(|(_, t)| t.clone()).collect();
    all_text.extend(texts.iter().map(|(_, t)| t.clone()));
    all_text.extend(dims.iter().map(|m| plain(&m.text)));
    let mut sized: HashSet<i64> = HashSet::new();
    let mut sized1: HashSet<i64> = HashSet::new();
    let mut rmin = false;
    let mut chamfer_c: Vec<f64> = Vec::new();
    let add_r = |sized: &mut HashSet<i64>, sized1: &mut HashSet<i64>, r: f64| {
        sized.insert(rkey(r));
        sized1.insert(rkey1(r));
    };
    for t in &all_text {
        for w in ["DRILL", "THRU", "HOLE", "DP"] {
            for v in numbers_before(t, w) {
                add_r(&mut sized, &mut sized1, v / 2.0);
            }
        }
        for v in numbers_after(t, "Ø") {
            add_r(&mut sized, &mut sized1, v / 2.0);
        }
        for v in numbers_after(t, "M") {
            for k in [1.0, 0.85, 0.8, 0.84, 0.86, 0.9] {
                sized1.insert(rkey1(v * k / 2.0));
            }
        }
        for v in numbers_after(t, "R") {
            add_r(&mut sized, &mut sized1, v);
        }
        let u = t.to_uppercase().replace([' ', '-'], "");
        if u.contains("RMIN") {
            rmin = true;
        }
        chamfer_c.extend(numbers_after(t, "C"));
    }
    for m in &dims {
        match m.kind {
            DimKind::Diameter => add_r(&mut sized, &mut sized1, m.meas / 2.0),
            DimKind::Radius => add_r(&mut sized, &mut sized1, m.meas),
            DimKind::Linear { .. } | DimKind::Aligned
                if m.text.contains("%%c") || m.text.contains("%%C") || m.text.contains('Ø') || m.text.contains('∅') =>
            {
                add_r(&mut sized, &mut sized1, m.meas / 2.0)
            }
            _ => {}
        }
    }
    let dim_values: Vec<f64> = dims.iter().filter(|m| lin(&m.kind)).map(|m| m.meas).filter(|v| v.is_finite() && *v > 0.0).collect();
    let is_sized = |sized: &HashSet<i64>, sized1: &HashSet<i64>, r: f64| sized.contains(&rkey(r)) || sized1.contains(&rkey1(r));

    // ---- per view
    let nviews = views.len();
    for vi in 0..views.len() {
        let Some(v) = views.get(vi) else { continue };
        let vb = v.bbox;
        let varea = (vb.width() * vb.height()).max(1e-9);
        let f: Vec<&Feat> = v.feats.iter().filter_map(|i| feats.get(*i)).filter(|f| f.block_area.is_none_or(|a| a >= 0.5 * varea)).collect();
        let circles: Vec<&Feat> = f.iter().copied().filter(|f| f.kind == Kind::Circle && !f.hidden).collect();
        let arcs: Vec<&Feat> = f.iter().copied().filter(|f| f.kind == Kind::Arc && !f.hidden).collect();
        let lines: Vec<&Feat> = f.iter().copied().filter(|f| f.kind == Kind::Line && !f.hidden).collect();
        let mut sized = sized.clone();
        let mut sized1 = sized1.clone();
        // Callouts pointing at a circle or arc size it.
        for (tip, t) in &callouts {
            let u = t.to_uppercase();
            let says = ["DRILL", "THRU", "HOLE", "TAP", "SLOT", "Ø", "R", "C"].iter().any(|w| u.contains(w)) || u.chars().any(|c| c.is_ascii_digit());
            if !says {
                continue;
            }
            for g in circles.iter().chain(arcs.iter()) {
                let dd = tip.dist(g.c);
                if (dd - g.r).abs() < (g.r * 0.6).max(1.5) || dd < g.r {
                    sized.insert(rkey(g.r));
                    sized1.insert(rkey1(g.r));
                }
            }
        }
        let mut ux: Vec<(i64, i64)> = Vec::new();
        let mut uy: Vec<(i64, i64)> = Vec::new();
        // --- dimension links, pitch, values
        for k in &v.dims {
            let Some(m) = dims.get(*k) else { continue };
            if !lin(&m.kind) {
                continue;
            }
            let (a, b) = (m.p13, m.p14);
            let (mut horiz, mut vert) = match m.kind {
                DimKind::Linear { rotation } => {
                    let ang = rotation.to_degrees().rem_euclid(180.0);
                    (!(1.0..=179.0).contains(&ang), (ang - 90.0).abs() < 1.0)
                }
                _ => (false, false),
            };
            if matches!(m.kind, DimKind::Aligned) {
                if (a.y - b.y).abs() < TOL {
                    horiz = true;
                } else if (a.x - b.x).abs() < TOL {
                    vert = true;
                }
            }
            if horiz {
                ux.push((q(a.x), q(b.x)));
            } else if vert {
                uy.push((q(a.y), q(b.y)));
            }
            // Pitch "P75x22=<>": the equally spaced points between are known.
            let ov = plain(&m.text).to_uppercase();
            if let Some(i) = ov.find('P')
                && let Some((step, used)) = lead_number(ov.get(i + 1..).unwrap_or("").trim_start())
            {
                let rest = ov.get(i + 1..).unwrap_or("").trim_start();
                let after = rest.get(used..).unwrap_or("").trim_start().trim_start_matches(['X', '×']).trim_start();
                if let Some((n, _)) = lead_number(after)
                    && (horiz || vert)
                    && step > 0.0
                    && (1.0..10_000.0).contains(&n)
                {
                    let (s0, s1) = if horiz { (a.x, b.x) } else { (a.y, b.y) };
                    let sg = if s1 >= s0 { 1.0 } else { -1.0 };
                    for kk in 1..(n as i64) {
                        let pair = (q(s0), q(s0 + sg * step * kk as f64));
                        if horiz { ux.push(pair) } else { uy.push(pair) }
                    }
                }
            }
            // Text typed over the measurement that doesn't agree with it.
            if !m.text.is_empty() && !m.text.contains("<>") && !m.text.contains("\\L") && !ov.contains('P') {
                let first = ov.split(|c: char| !(c.is_ascii_digit() || c == '.')).find(|w| !w.is_empty() && w.chars().any(|c| c.is_ascii_digit()));
                if let Some(val) = first.and_then(|w| w.trim_matches('.').parse::<f64>().ok())
                    && (val - m.meas).abs() > 0.05
                    && (val - m.meas * 2.0).abs() > 0.05
                {
                    issues.push(Issue {
                        kind: "value",
                        msg: format!("dimension text \"{}\" but the geometry measures {:.2}", plain(&m.text), m.meas),
                        at: m.text_at,
                    });
                }
            }
            // Symmetry: centred on a centre line, the dimension ties the axis to both ends.
            for (axis_x, cl) in [(true, &centre_x), (false, &centre_y)] {
                let (a1, a2) = if axis_x { (a.x, b.x) } else { (a.y, b.y) };
                if (a1 - a2).abs() < TOL {
                    continue;
                }
                let mid = (a1 + a2) / 2.0;
                let (lo, hi) = if axis_x { (vb.min.y, vb.max.y) } else { (vb.min.x, vb.max.x) };
                if cl.iter().any(|(c, s0, s1)| (c - mid).abs() < SNAP && *s0 <= hi && lo <= *s1) {
                    let u = if axis_x { &mut ux } else { &mut uy };
                    u.push((q(mid), q(a1)));
                    u.push((q(mid), q(a2)));
                }
            }
        }
        // A plate's two faces one thickness apart.
        if !thick.is_empty() {
            let mut vl: Vec<(f64, f64, f64)> =
                lines.iter().filter(|l| (l.a.x - l.b.x).abs() < TOL).map(|l| (l.a.x, l.a.y.min(l.b.y), l.a.y.max(l.b.y))).collect();
            let mut hl: Vec<(f64, f64, f64)> =
                lines.iter().filter(|l| (l.a.y - l.b.y).abs() < TOL).map(|l| (l.a.y, l.a.x.min(l.b.x), l.a.x.max(l.b.x))).collect();
            for (ls, is_x) in [(&mut vl, true), (&mut hl, false)] {
                ls.sort_by(|a, b| a.0.total_cmp(&b.0));
                for i in 0..ls.len() {
                    let Some(li) = ls.get(i).copied() else { continue };
                    for lj in ls.iter().skip(i + 1) {
                        if lj.0 - li.0 > thick_max + 0.1 {
                            break;
                        }
                        let gap = lj.0 - li.0;
                        if thick.iter().any(|t| (gap - t).abs() < 0.05) && li.2.min(lj.2) - li.1.max(lj.1) > gap {
                            let pair = (q(li.0), q(lj.0));
                            if is_x { ux.push(pair) } else { uy.push(pair) }
                        }
                    }
                }
            }
        }
        // --- implied points
        let mut implied: Vec<Vec2> = Vec::new();
        for l in &lines {
            let (dx, dy) = ((l.b.x - l.a.x).abs(), (l.b.y - l.a.y).abs());
            if dx > TOL && dy > TOL {
                let len = l.a.dist(l.b);
                let mid = (l.a + l.b) * 0.5;
                let called = callouts.iter().any(|(tip, t)| tip.dist(mid) < len.max(3.0) && !numbers_after(t, "C").is_empty());
                let leg45 = (dx - dy).abs() < 0.05 * dx.max(dy);
                if (leg45 && (called || chamfer_c.iter().any(|c| (dx - c).abs() < 0.15))) || len < 3.0 {
                    implied.extend([l.a, l.b]);
                    ux.push((q(l.a.x), q(l.b.x)));
                    uy.push((q(l.a.y), q(l.b.y)));
                }
            }
        }
        let marg = vb.width().max(vb.height()) * 0.6;
        let near_section = sections.iter().any(|p| vb.expand(marg).contains(*p));
        for a in &arcs {
            if is_sized(&sized, &sized1, a.r) || rmin || (near_section && a.r < 20.0) {
                implied.extend([a.a, a.b, a.c]);
                ux.push((q(a.a.x), q(a.c.x)));
                uy.push((q(a.a.y), q(a.c.y)));
                ux.push((q(a.b.x), q(a.c.x)));
                uy.push((q(a.b.y), q(a.c.y)));
            }
        }
        // Slots: two equal arcs and a "WxL SLOT" note.
        for (_, t) in &callouts {
            let u = t.to_uppercase();
            if let Some(i) = u.find("SLOT") {
                let head = u.get(..i).unwrap_or("").trim_end();
                let nums: Vec<f64> = head.split(|c: char| !(c.is_ascii_digit() || c == '.')).filter_map(|w| w.parse::<f64>().ok()).collect();
                if let [.., w, l] = nums.as_slice() {
                    for (ia, fa) in arcs.iter().enumerate() {
                        for fb in arcs.iter().skip(ia + 1) {
                            if (fa.r - w / 2.0).abs() < 0.1 && (fb.r - w / 2.0).abs() < 0.1 && (fa.c.dist(fb.c) - (l - w)).abs() < 0.1 {
                                ux.push((q(fa.c.x), q(fb.c.x)));
                                uy.push((q(fa.c.y), q(fb.c.y)));
                                sized.insert(rkey(fa.r));
                            }
                        }
                    }
                }
            }
        }
        implied.extend(weld.iter().copied());
        // A sized circle's extremes follow from its centre.
        for c in &circles {
            if is_sized(&sized, &sized1, c.r) {
                ux.push((q(c.c.x), q(c.c.x - c.r)));
                ux.push((q(c.c.x), q(c.c.x + c.r)));
                uy.push((q(c.c.y), q(c.c.y - c.r)));
                uy.push((q(c.c.y), q(c.c.y + c.r)));
            }
        }
        for l in &lines {
            for pt in [l.a, l.b] {
                if circles
                    .iter()
                    .chain(arcs.iter())
                    .any(|g| (pt.dist(g.c) - g.r).abs() < SNAP && (g.kind == Kind::Circle || is_sized(&sized, &sized1, g.r)))
                {
                    implied.push(pt);
                }
            }
        }
        // Ends where a line just continues (split lines, gaps for a crossing part).
        {
            let cell = SNAP * 0.5;
            let key = |p: Vec2| (qi(p.x / cell), qi(p.y / cell));
            let mut ends: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
            for (k, l) in lines.iter().enumerate() {
                for pt in [l.a, l.b] {
                    let v = ends.entry(key(pt)).or_default();
                    if v.len() < 64 {
                        v.push(k);
                    }
                }
            }
            for (k, l) in lines.iter().enumerate() {
                let d1 = (l.b - l.a).normalized();
                for pt in [l.a, l.b] {
                    let (kx, ky) = key(pt);
                    let mut hit = false;
                    for gx in kx - 1..=kx + 1 {
                        for gy in ky - 1..=ky + 1 {
                            for m in ends.get(&(gx, gy)).map(Vec::as_slice).unwrap_or(&[]) {
                                if *m == k {
                                    continue;
                                }
                                if let Some(g) = lines.get(*m) {
                                    let d2 = (g.b - g.a).normalized();
                                    if d1.cross(d2).abs() < 1e-3 && (g.a.dist(pt) < 1e-6 || g.b.dist(pt) < 1e-6) {
                                        hit = true;
                                    }
                                }
                            }
                        }
                    }
                    if hit {
                        implied.push(pt);
                    }
                }
            }
        }
        // --- required coordinates
        let mut rx: BTreeMap<i64, (Vec2, String)> = BTreeMap::new();
        let mut ry: BTreeMap<i64, (Vec2, String)> = BTreeMap::new();
        let mut need = |pt: Vec2, what: String| {
            rx.entry(q(pt.x)).or_insert((pt, what.clone()));
            ry.entry(q(pt.y)).or_insert((pt, what));
        };
        let implied_grid: HashSet<(i64, i64)> = implied.iter().map(|p| (q(p.x), q(p.y))).collect();
        let is_implied = |p: Vec2| {
            let (x, y) = (q(p.x), q(p.y));
            (x - 1..=x + 1).any(|a| (y - 1..=y + 1).any(|b| implied_grid.contains(&(a, b)))) && implied.iter().any(|ip| ip.dist(p) <= SNAP * 1.5)
        };
        for l in &lines {
            for pt in [l.a, l.b] {
                if !is_implied(pt) {
                    need(pt, "an edge".into());
                }
            }
        }
        for c in &circles {
            need(c.c, format!("a hole Ø{}", super::autodim::fmt_len(2.0 * c.r)));
        }
        for a in &arcs {
            if !sized.contains(&rkey(a.r)) {
                need(a.c, format!("an arc R{}", super::autodim::fmt_len(a.r)));
            }
        }
        // Coordinates one thickness apart are a wall's two faces; two coordinates as far apart
        // as a length dimensioned elsewhere are that size (shown in another view).
        for (req, u) in [(&rx, &mut ux), (&ry, &mut uy)] {
            let keys: Vec<i64> = req.keys().copied().collect();
            let set: BTreeSet<i64> = keys.iter().copied().collect();
            for a in &keys {
                for t in &thick {
                    let b = q(*a as f64 * SNAP + t);
                    for bb in [b - 1, b, b + 1] {
                        if set.contains(&bb) && (((bb - a) as f64 * SNAP) - t).abs() <= SNAP {
                            u.push((*a, bb));
                        }
                    }
                }
                for val in &dim_values {
                    let b = q(*a as f64 * SNAP + val);
                    for bb in [b - 1, b, b + 1] {
                        if set.contains(&bb) && (((bb - a) as f64 * SNAP) - val).abs() <= SNAP {
                            u.push((*a, bb));
                        }
                    }
                }
            }
        }
        // Views laid out by angles and radii, or turned off the axes, are not checked.
        let slant: f64 = lines.iter().filter(|l| (l.a.x - l.b.x).abs() > TOL && (l.a.y - l.b.y).abs() > TOL).map(|l| l.a.dist(l.b)).sum();
        let total: f64 = lines.iter().map(|l| l.a.dist(l.b)).sum::<f64>().max(1.0);
        let angular = v.dims.iter().filter(|k| dims.get(**k).is_some_and(|m| matches!(m.kind, DimKind::Angular | DimKind::Angular3P))).count();
        let rotated = v
            .dims
            .iter()
            .filter(|k| {
                dims.get(**k).is_some_and(|m| match m.kind {
                    DimKind::Linear { rotation } => {
                        let a = rotation.to_degrees().rem_euclid(90.0);
                        a.min(90.0 - a) > 1.0
                    }
                    _ => false,
                })
            })
            .count();
        let polar = angular >= 2 || slant / total > 0.3 || rotated >= 2;
        // Holes without a size.
        let mut seen: HashSet<i64> = HashSet::new();
        for c in &circles {
            let r = rkey(c.r);
            if is_sized(&sized, &sized1, c.r) || seen.contains(&r) {
                continue;
            }
            if circles.iter().any(|g| is_sized(&sized, &sized1, g.r) && g.c.dist(c.c) < TOL) {
                continue;
            }
            seen.insert(r);
            issues.push(Issue {
                kind: "size",
                msg: format!("the hole Ø{} has no size note or dimension", super::autodim::fmt_len(2.0 * c.r)),
                at: c.c,
            });
        }
        if let Some(view) = views.get_mut(vi) {
            view.xl = ux;
            view.yl = uy;
            view.rx = rx;
            view.ry = ry;
            view.polar = polar;
        }
    }
    // ---- connectivity, with views in projection (above/below for X, beside for Y)
    for vi in 0..views.len() {
        let Some(v) = views.get(vi) else { continue };
        if v.polar {
            issues.push(Issue { kind: "skipped", msg: "a view laid out by angles and radii (or turned): not checked".into(), at: v.bbox.center() });
            continue;
        }
        for axis_x in [true, false] {
            let req = if axis_x { &v.rx } else { &v.ry };
            if req.len() < 2 {
                continue;
            }
            let mut uf = Uf::default();
            let mut keys: BTreeSet<i64> = req.keys().copied().collect();
            for w in &views {
                let overlap = if axis_x {
                    w.bbox.min.x <= v.bbox.max.x && v.bbox.min.x <= w.bbox.max.x
                } else {
                    w.bbox.min.y <= v.bbox.max.y && v.bbox.min.y <= w.bbox.max.y
                };
                if overlap {
                    for (a, b) in if axis_x { &w.xl } else { &w.yl } {
                        uf.union(*a, *b);
                        keys.insert(*a);
                        keys.insert(*b);
                    }
                }
            }
            let ks: Vec<i64> = keys.iter().copied().collect();
            for k in &ks {
                if keys.contains(&(k + 1)) {
                    uf.union(*k, k + 1);
                }
            }
            let mut comp: BTreeMap<i64, Vec<(i64, Vec2, String)>> = BTreeMap::new();
            for (k, (pt, what)) in req {
                comp.entry(uf.find(*k)).or_default().push((*k, *pt, what.clone()));
            }
            if comp.len() > 1 {
                let mut comps: Vec<Vec<(i64, Vec2, String)>> = comp.into_values().collect();
                comps.sort_by_key(|c| std::cmp::Reverse(c.len()));
                let main: Vec<i64> = comps.first().map(|c| c.iter().map(|x| x.0).collect()).unwrap_or_default();
                for c in comps.iter().skip(1) {
                    let Some((k, pt, what)) = c.first() else { continue };
                    let near = main.iter().map(|m| (k - m).abs()).min().unwrap_or(0) as f64 * SNAP;
                    let kind = if near < 5.0 { "minor" } else { "missing" };
                    issues.push(Issue {
                        kind,
                        msg: format!(
                            "{} position of {what} at ({:.1}, {:.1}) is not tied to the rest ({} point(s), {:.1} from a dimensioned one)",
                            if axis_x { "X" } else { "Y" },
                            pt.x,
                            pt.y,
                            c.len(),
                            near
                        ),
                        at: *pt,
                    });
                }
            }
        }
    }
    (issues, nviews)
}
