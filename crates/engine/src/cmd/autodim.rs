//! AUTODIM: dimension a part drawn in 2D the way a shop drawing is dimensioned, and DIMCHECK
//! (in `autodim_check`): find the coordinates of a view that no dimension ties down.
//!
//! AUTODIM lays out, for the selected outline and holes:
//! - chains above the part (X) and to its right (Y): hole and slot positions from the edges, the
//!   steps of the outline, and the overall size outermost;
//! - equally spaced holes as one pitch dimension, `P75x22=1650`;
//! - hole callouts on leaders, counted and grouped by size: `4-12DRILL`, `4-M10 THRU`,
//!   `2-12x36 SLOT`; fillets `R10` and 45° chamfers `C5`;
//! - an optional thickness note `t=12` in the middle.
//!
//! Dimensions use the current dimension style (or `style`), so their size follows its scale.

use std::collections::BTreeMap;

use cadcraft_doc::{DimKind, Drawing, Entity, EntityKind, Handle, Prim};
use cadcraft_geom::{Bounds2, Segment, Vec2};
use serde_json::{Value, json};

use super::machines::{SelOutcome, SelectPhase};
use super::*;
use crate::{Accept, Input, Interactive, Prompt, Result, Session, Step};

/// Most curves looked at (a whole sheet of an assembly drawing is far below this).
pub(super) const MAX_FEATS: usize = 400_000;
/// Block nesting followed when looking into block references.
const MAX_DEPTH: usize = 4;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("autodim", "Auto Dimension", run_autodim)
            .menu(&["Dimension", "Auto Dimension"])
            .alias(&["adim"])
            .params(
                "{handles?: [hex] (the part's outline, holes and slots; default the selection), style?: dimension style, \
                 thickness?: number (adds the note t=…), pitchMin?: holes in a row that take a pitch dimension (default 4), \
                 callouts?: bool (hole, fillet and chamfer notes, default true), check?: bool (run DIMCHECK after, default true)}",
            )
            .interactive(|_| Ok(Box::new(AutoDimM::default()))),
        CommandSpec::new("dimcheck", "Check Dimensions", super::autodim_check::run_dimcheck)
            .menu(&["Dimension", "Check Dimensions"])
            .alias(&["dchk"])
            .params(
                "{handles?: [hex] (one view: its geometry and dimensions; default the selection, or the whole drawing \
                 split into views), mark?: bool (circles on layer DIMCHECK at each finding, default true)}",
            )
            .noundo()
            .interactive(|_| Ok(Box::new(DimCheckM::default()))),
    ]
}

// ------------------------------------------------------------------ geometry

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Kind {
    Line,
    Arc,
    Circle,
}

/// A curve of the part: a line (`a`–`b`), an arc (centre `c`, radius `r`, from `a` to `b`
/// counter-clockwise over `sweep`) or a circle.
#[derive(Clone, Copy, Debug)]
pub(super) struct Feat {
    pub kind: Kind,
    /// Centre, hidden or phantom line: not outline.
    pub hidden: bool,
    pub a: Vec2,
    pub b: Vec2,
    pub c: Vec2,
    pub r: f64,
    pub sweep: f64,
    /// Drawn inside a block reference of this area (None: drawn directly).
    pub block_area: Option<f64>,
    pub layer_annot: bool,
    /// A centre line (an axis of symmetry).
    pub centre: bool,
}

impl Feat {
    pub fn bounds(&self) -> Bounds2 {
        match self.kind {
            Kind::Line => Bounds2::from_points([self.a, self.b]),
            _ => Bounds2::new(self.c - Vec2::new(self.r, self.r), self.c + Vec2::new(self.r, self.r)),
        }
    }
    pub fn is_h(&self, tol: f64) -> bool {
        self.kind == Kind::Line && (self.a.y - self.b.y).abs() <= tol && (self.a.x - self.b.x).abs() > tol
    }
    pub fn is_v(&self, tol: f64) -> bool {
        self.kind == Kind::Line && (self.a.x - self.b.x).abs() <= tol && (self.a.y - self.b.y).abs() > tol
    }
}

/// A linetype that marks a line as not part of the outline.
pub(super) fn non_outline_linetype(name: &str) -> bool {
    let u = name.to_ascii_uppercase();
    ["CENTER", "CENTRE", "HIDDEN", "HIDEN", "PHANTOM", "DASH", "DOT", "CHAIN"].iter().any(|k| u.contains(k))
}

pub(super) fn centre_linetype(name: &str) -> bool {
    let u = name.to_ascii_uppercase();
    u.contains("CENTER") || u.contains("CENTRE") || u.contains("CHAIN")
}

/// Layers that hold annotation, not the part.
pub(super) fn annotation_layer(name: &str) -> bool {
    let u = name.to_ascii_uppercase();
    ["DIM", "CUT", "TEXT", "DEFPOINT", "TITLE", "FRAME", "BALLOON", "NOTE"].iter().any(|k| u.contains(k))
}

/// An entity's linetype after ByLayer.
pub(super) fn linetype_of(d: &Drawing, e: &Entity) -> String {
    let lt = &e.common.linetype;
    if lt.eq_ignore_ascii_case("ByLayer") {
        d.layer(&e.common.layer).map(|l| l.linetype.clone()).unwrap_or_default()
    } else if lt.eq_ignore_ascii_case("ByBlock") {
        String::new()
    } else {
        lt.clone()
    }
}

/// The curves of `ents`, looking into block references (with the referenced block's area).
pub(super) fn collect(d: &Drawing, ents: &[Entity], out: &mut Vec<Feat>, depth: usize, block_area: Option<f64>) {
    for e in ents {
        if out.len() >= MAX_FEATS {
            return;
        }
        match &e.kind {
            EntityKind::Insert(_) => {
                if depth >= MAX_DEPTH {
                    continue;
                }
                let area = block_area.or_else(|| {
                    let b = cadcraft_doc::entity_bounds(d, e, 0);
                    (!b.is_empty()).then(|| b.width() * b.height())
                });
                if let Some(parts) = super::modify::explode_kind(d, e) {
                    collect(d, &parts, out, depth + 1, area);
                }
            }
            EntityKind::Line(_) | EntityKind::LwPolyline(_) | EntityKind::Arc(_) | EntityKind::Circle(_) | EntityKind::Polyline3d(_) => {
                let lt = linetype_of(d, e);
                let hidden = non_outline_linetype(&lt);
                let centre = centre_linetype(&lt);
                let layer_annot = annotation_layer(&e.common.layer);
                for pr in e.kind.prims() {
                    let f = match pr {
                        Prim::Seg(Segment::Line(l)) => {
                            if l.a.near(l.b, 1e-12) {
                                continue;
                            }
                            Feat { kind: Kind::Line, hidden, a: l.a, b: l.b, c: Vec2::ZERO, r: 0.0, sweep: 0.0, block_area, layer_annot, centre }
                        }
                        Prim::Seg(Segment::Arc { arc, .. }) => Feat {
                            kind: Kind::Arc,
                            hidden,
                            a: arc.start_point(),
                            b: arc.end_point(),
                            c: arc.center,
                            r: arc.radius,
                            sweep: arc.sweep(),
                            block_area,
                            layer_annot,
                            centre,
                        },
                        Prim::Circle(c) => Feat {
                            kind: Kind::Circle,
                            hidden,
                            a: c.center,
                            b: c.center,
                            c: c.center,
                            r: c.radius,
                            sweep: std::f64::consts::TAU,
                            block_area,
                            layer_annot,
                            centre,
                        },
                        _ => continue,
                    };
                    if f.a.is_finite() && f.b.is_finite() && f.c.is_finite() && f.r.is_finite() {
                        out.push(f);
                    }
                }
            }
            _ => {}
        }
    }
}

// ------------------------------------------------------------------ text helpers

/// A number written at the start of `s` (digits and one decimal point).
pub(super) fn lead_number(s: &str) -> Option<(f64, usize)> {
    let mut end = 0;
    let mut dot = false;
    for (i, c) in s.char_indices() {
        if c.is_ascii_digit() {
            end = i + 1;
        } else if c == '.' && !dot && end == i && i > 0 {
            dot = true;
        } else {
            break;
        }
    }
    let t = s.get(..end)?.trim_end_matches('.');
    t.parse::<f64>().ok().filter(|v| v.is_finite()).map(|v| (v, end))
}

/// The numbers that follow `prefix` (as a word start: not after a letter or digit), after
/// optional spaces, `=` or `-`.
pub(super) fn numbers_after(text: &str, prefix: &str) -> Vec<f64> {
    let up = text.to_uppercase();
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(i) = up.get(from..).and_then(|t| t.find(prefix)).map(|i| i + from) {
        from = i + prefix.len().max(1);
        let before = up.get(..i).and_then(|t| t.chars().last());
        if before.is_some_and(|c| c.is_ascii_alphanumeric()) {
            continue;
        }
        let rest = up.get(i + prefix.len()..).unwrap_or("").trim_start_matches([' ', '=', '-']);
        if let Some((v, _)) = lead_number(rest) {
            out.push(v);
        }
    }
    out
}

/// The numbers directly before `word` ("12DRILL", "Ø12 THRU", "4-12 DRILL" → 12).
pub(super) fn numbers_before(text: &str, word: &str) -> Vec<f64> {
    let up = text.to_uppercase();
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(i) = up.get(from..).and_then(|t| t.find(word)).map(|i| i + from) {
        from = i + word.len();
        let head: Vec<char> = up.get(..i).unwrap_or("").trim_end().chars().collect();
        let mut j = head.len();
        while j > 0 && head.get(j - 1).is_some_and(|c| c.is_ascii_digit() || *c == '.') {
            j -= 1;
        }
        let num: String = head.get(j..).unwrap_or(&[]).iter().collect();
        if let Ok(v) = num.trim_matches('.').parse::<f64>() {
            out.push(v);
        }
    }
    out
}

/// A length as a shop drawing writes it: up to two decimals, no trailing zeros.
pub(super) fn fmt_len(v: f64) -> String {
    let r = (v * 100.0).round() / 100.0;
    let s = format!("{r:.2}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.to_string() }
}

// ------------------------------------------------------------------ the plan

/// A hole (circles sharing a centre) or a slot.
#[derive(Clone, Debug)]
struct Hole {
    /// The points whose X (`px`) and Y (`py`) positions are dimensioned: a hole's centre; a
    /// slot's two ends along its length (as hand-drafted: 9/57/203/57/9) and its centre across.
    px: Vec<Vec2>,
    py: Vec<Vec2>,
    /// The circle the callout's leader points at.
    tip_c: Vec2,
    tip_r: f64,
    /// Callout without the count: "12DRILL", "M10 THRU", "12x36 SLOT".
    label: String,
}

impl Hole {
    fn round(c: Vec2, r: f64, label: String) -> Hole {
        Hole { px: vec![c], py: vec![c], tip_c: c, tip_r: r, label }
    }
}

/// What AUTODIM adds, before it is added.
#[derive(Debug, Default)]
pub(super) struct Plan {
    pub dims: Vec<EntityKind>,
    /// Leader points (tip first) and text.
    pub notes: Vec<(Vec<Vec2>, String)>,
    /// Text placed in the middle (thickness).
    pub labels: Vec<(Vec2, String)>,
}

fn same(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

/// Values sorted and merged when closer than `tol`.
fn uniq(mut v: Vec<f64>, tol: f64) -> Vec<f64> {
    v.retain(|x| x.is_finite());
    v.sort_by(f64::total_cmp);
    let mut out: Vec<f64> = Vec::new();
    for x in v {
        if out.last().is_none_or(|l| (x - l).abs() > tol) {
            out.push(x);
        }
    }
    out
}

/// One chain of dimensions along an axis (from edge to edge): consecutive values, with runs of
/// `pitch_min - 1` or more equal steps between them as one pitch dimension. Returns (from, to,
/// text).
fn chain(vals: &[f64], pitch_min: usize, tol: f64) -> Vec<(f64, f64, String)> {
    let mut out = Vec::new();
    let steps: Vec<f64> = vals.windows(2).filter_map(|w| Some(w.get(1)? - w.first()?)).collect();
    let mut i = 0;
    while i < steps.len() {
        let s = steps.get(i).copied().unwrap_or(0.0);
        // A pitch runs between holes: not the first and last steps, which start at the edges.
        let last = steps.len().saturating_sub(1);
        let mut j = i + 1;
        while i > 0 && j < last && steps.get(j).is_some_and(|t| same(*t, s, tol.max(s.abs() * 1e-4))) {
            j += 1;
        }
        let n = j - i;
        let (Some(a), Some(b)) = (vals.get(i).copied(), vals.get(j).copied()) else { break };
        if n + 1 >= pitch_min.max(3) && n >= 2 {
            out.push((a, b, format!("P{}x{}=<>", fmt_len(s), n)));
            i = j;
        } else {
            if let Some(b1) = vals.get(i + 1).copied() {
                out.push((a, b1, String::new()));
            }
            i += 1;
        }
    }
    out
}

pub(super) struct Opts {
    pub pitch_min: usize,
    pub callouts: bool,
    pub thickness: Option<f64>,
    /// Text height in drawing units (style height × scale).
    pub th: f64,
    /// A layout (a whole line of machines): only the sections along it and the overall size.
    /// None: decided from the view (long and busy).
    pub layout: Option<bool>,
}

/// The dimensions and notes for a part made of `feats` (outline curves, holes, slots).
pub(super) fn plan(s: &Session, feats: &[Feat], o: &Opts) -> Result<Plan> {
    // A part drawn at an angle is dimensioned along its own edges: planned square to them, then
    // turned back.
    if let Some(t) = tilt(feats) {
        let turned: Vec<Feat> = feats.iter().map(|f| turn_feat(f, -t)).collect();
        let mut p = plan_square(s, &turned, o)?;
        turn_plan(&mut p, t);
        return Ok(p);
    }
    plan_square(s, feats, o)
}

fn turn(v: Vec2, t: f64) -> Vec2 {
    let (sn, cs) = t.sin_cos();
    Vec2::new(v.x * cs - v.y * sn, v.x * sn + v.y * cs)
}

fn turn_feat(f: &Feat, t: f64) -> Feat {
    Feat { a: turn(f.a, t), b: turn(f.b, t), c: turn(f.c, t), ..*f }
}

fn turn_plan(p: &mut Plan, t: f64) {
    let t3 = |v: &mut cadcraft_geom::Vec3| {
        let q = turn(v.xy(), t);
        v.x = q.x;
        v.y = q.y;
    };
    for d in &mut p.dims {
        if let EntityKind::Dimension(dm) = d {
            for v in [&mut dm.defpt, &mut dm.text_mid, &mut dm.p13, &mut dm.p14, &mut dm.p15, &mut dm.p16] {
                t3(v);
            }
            if let DimKind::Linear { rotation } = &mut dm.kind {
                *rotation += t;
            }
        }
    }
    for (pts, _) in &mut p.notes {
        for q in pts.iter_mut() {
            *q = turn(*q, t);
        }
    }
    for (q, _) in &mut p.labels {
        *q = turn(*q, t);
    }
}

/// The angle a part is drawn at: the direction (up to quarter turns) most of its straight
/// outline runs in, when that is not square to the sheet. None for a part drawn square.
fn tilt(feats: &[Feat]) -> Option<f64> {
    let q = std::f64::consts::FRAC_PI_2;
    let mut bins: BTreeMap<i64, f64> = BTreeMap::new();
    let mut total = 0.0;
    for f in feats.iter().filter(|f| !f.hidden && f.kind == Kind::Line).take(MAX_FEATS) {
        let d = f.b - f.a;
        let len = d.len();
        if !(len.is_finite() && len > 0.0) {
            continue;
        }
        let a = d.y.atan2(d.x).rem_euclid(q);
        let a = if a >= q / 2.0 { a - q } else { a };
        // Quarter-degree bins.
        *bins.entry((a.to_degrees() * 4.0).round() as i64).or_default() += len;
        total += len;
    }
    let (bin, w) = bins.into_iter().max_by(|x, y| x.1.total_cmp(&y.1))?;
    let a = (bin as f64 / 4.0).to_radians();
    (w >= 0.5 * total && a.abs() > 0.5_f64.to_radians()).then_some(a)
}

fn plan_square(s: &Session, feats: &[Feat], o: &Opts) -> Result<Plan> {
    // A gear or sprocket: its teeth are given by the tooth form, not dimensioned one by one.
    let reduced = without_teeth(feats);
    let feats: &[Feat] = reduced.as_deref().unwrap_or(feats);
    let outline: Vec<&Feat> = feats.iter().filter(|f| !f.hidden).collect();
    let mut bounds = Bounds2::EMPTY;
    for f in &outline {
        bounds = bounds.union(&f.bounds());
    }
    if bounds.is_empty() || bounds.width() <= 0.0 && bounds.height() <= 0.0 {
        return Err(bad("autodim", "select the part: its outline lines, arcs and circles"));
    }
    let size = bounds.width().max(bounds.height()).max(1e-9);
    let tol = (size * 1e-6).max(1e-6);
    // Coordinates closer than this are one (drafting inaccuracy).
    let ctol = (size * 2e-4).clamp(1e-6, 0.1);
    let th = o.th.max(size * 1e-4);
    let long = bounds.width().max(bounds.height()) >= 5.0 * bounds.width().min(bounds.height());
    if o.layout.unwrap_or(long && outline.len() >= 150) {
        return Ok(layout_plan(s, &outline, &bounds, tol, ctol, th));
    }

    // --- circles: holes (grouped by centre) or the part itself (a disc)
    let circles: Vec<&Feat> = outline.iter().copied().filter(|f| f.kind == Kind::Circle).collect();
    let arcs: Vec<&Feat> = outline.iter().copied().filter(|f| f.kind == Kind::Arc).collect();
    let lines: Vec<&Feat> = outline.iter().copied().filter(|f| f.kind == Kind::Line).collect();
    let near = |a: Vec2, b: Vec2| a.dist(b) <= size * 1e-4;
    let mut holes: Vec<Hole> = Vec::new();
    let mut disc: Option<(Vec2, f64)> = None;
    // A disc's concentric circles given as diameters.
    let mut rings: Vec<f64> = Vec::new();
    // Bores: centre and diameters.
    let mut bores: Vec<(Vec2, Vec<f64>)> = Vec::new();
    let mut used_arcs = vec![false; arcs.len()];
    let mut used_lines = vec![false; lines.len()];
    let mut done = vec![false; circles.len()];
    for (i, c) in circles.iter().enumerate() {
        if done.get(i).copied().unwrap_or(true) {
            continue;
        }
        let group: Vec<usize> = (0..circles.len()).filter(|j| circles.get(*j).is_some_and(|d| near(d.c, c.c))).collect();
        for j in &group {
            if let Some(d) = done.get_mut(*j) {
                *d = true;
            }
        }
        let rmin = group.iter().filter_map(|j| circles.get(*j)).map(|f| f.r).fold(f64::INFINITY, f64::min);
        let rmax = group.iter().filter_map(|j| circles.get(*j)).map(|f| f.r).fold(0.0_f64, f64::max);
        // The part is a disc when its largest circle holds everything (lines only inside it:
        // a keyway).
        let inside = |q: Vec2| q.dist(c.c) < 0.9 * rmax;
        if 2.0 * rmax >= 0.98 * size && lines.iter().all(|l| inside(l.a) && inside(l.b)) && arcs.iter().all(|a| inside(a.a) && inside(a.b)) {
            disc = Some((c.c, rmax));
            if group.len() == 1 {
                continue;
            }
        }
        // A thread: a 3/4 arc round the drilled circle.
        let thread = arcs.iter().enumerate().find(|(_, a)| near(a.c, c.c) && a.r > rmin && a.sweep > 3.5 && a.sweep < 5.9);
        let label = if let Some((k, a)) = thread {
            if let Some(u) = used_arcs.get_mut(k) {
                *u = true;
            }
            format!("M{} THRU", fmt_len(2.0 * a.r))
        } else if let Some(m) = (group.len() >= 2).then(|| thread_size(2.0 * rmin, 2.0 * rmax)).flatten() {
            format!("M{} THRU", fmt_len(m))
        } else if let Some((m, big)) = (group.len() >= 3)
            .then(|| {
                // A tapped hole at the bottom of a counterbore: the two smallest circles are
                // the thread, the largest the bore.
                let mut rs: Vec<f64> = group.iter().filter_map(|j| circles.get(*j)).map(|f| f.r).collect();
                rs.sort_by(f64::total_cmp);
                let (a, b) = (rs.first().copied()?, rs.get(1).copied()?);
                thread_size(2.0 * a, 2.0 * b).map(|m| (m, rmax))
            })
            .flatten()
        {
            format!("M{} {}C'BORE", fmt_len(m), fmt_len(2.0 * big))
        } else if group.len() >= 2 && !disc.is_some_and(|(dc, _)| near(dc, c.c)) {
            format!("{}DRILL {}C'BORE", fmt_len(2.0 * rmin), fmt_len(2.0 * rmax))
        } else {
            format!("{}DRILL", fmt_len(2.0 * rmin))
        };
        if disc.is_some_and(|(dc, _)| near(dc, c.c)) && group.len() >= 2 {
            // A disc with a centre bore: the bore is the hole. A ring just inside the rim is
            // its chamfer, not a bore.
            if rmin >= 0.85 * rmax && group.len() == 2 {
                continue;
            }
            // A turned disc (a hub, steps, a keyed bore): its circles are diameters.
            let keyed = arcs.iter().any(|a| near(a.c, c.c) && a.sweep > std::f64::consts::PI && a.r < rmax);
            if group.len() > 2 || keyed {
                let mut rs: Vec<f64> = group.iter().filter_map(|j| circles.get(*j)).map(|f| f.r).filter(|r| *r < 0.97 * rmax).collect();
                rs.sort_by(f64::total_cmp);
                rs.dedup_by(|a, b| same(*a, *b, tol.max(*a * 1e-4)));
                rings.extend(rs);
                continue;
            }
            holes.push(Hole::round(c.c, rmin, format!("{}DRILL", fmt_len(2.0 * rmin))));
            continue;
        }
        // A bore (a circle a good part of the part's size, and its counterbore) is given its
        // diameters; only its centre is a position.
        if 2.0 * rmax >= 0.45 * bounds.width().min(bounds.height()) && !label.starts_with('M') {
            let mut rs: Vec<f64> = group.iter().filter_map(|j| circles.get(*j)).map(|f| f.r).collect();
            rs.sort_by(f64::total_cmp);
            rs.dedup_by(|a, b| same(*a, *b, tol.max(*a * 1e-4)));
            bores.push((c.c, rs));
            holes.push(Hole::round(c.c, rmax, String::new()));
            continue;
        }
        holes.push(Hole::round(c.c, rmin, label));
    }
    // --- slots: two half circles of one radius facing apart, joined by two lines
    for i in 0..arcs.len() {
        for j in i + 1..arcs.len() {
            let (Some(a), Some(b)) = (arcs.get(i), arcs.get(j)) else { continue };
            if used_arcs.get(i).copied().unwrap_or(true) || used_arcs.get(j).copied().unwrap_or(true) {
                continue;
            }
            let half = |f: &Feat| (f.sweep - std::f64::consts::PI).abs() < 0.02;
            if !half(a) || !half(b) || !same(a.r, b.r, tol.max(a.r * 1e-4)) || near(a.c, b.c) {
                continue;
            }
            // Joined: two lines from one arc's ends to the other's.
            let joins: Vec<usize> = (0..lines.len())
                .filter(|k| {
                    lines.get(*k).is_some_and(|l| {
                        let ends = [a.a, a.b];
                        let others = [b.a, b.b];
                        (ends.iter().any(|p| near(*p, l.a)) && others.iter().any(|p| near(*p, l.b)))
                            || (ends.iter().any(|p| near(*p, l.b)) && others.iter().any(|p| near(*p, l.a)))
                    })
                })
                .collect();
            if joins.len() < 2 {
                continue;
            }
            for k in joins {
                if let Some(u) = used_lines.get_mut(k) {
                    *u = true;
                }
            }
            for k in [i, j] {
                if let Some(u) = used_arcs.get_mut(k) {
                    *u = true;
                }
            }
            let len = a.c.dist(b.c) + 2.0 * a.r;
            let (lo, hi) = if (a.c.x, a.c.y) <= (b.c.x, b.c.y) { (a.c, b.c) } else { (b.c, a.c) };
            let along = (hi - lo).normalized();
            let (e0, e1, mid) = (lo - along * a.r, hi + along * a.r, (lo + hi) * 0.5);
            let (px, py) = if along.y.abs() <= 1e-6 {
                (vec![e0, e1], vec![mid])
            } else if along.x.abs() <= 1e-6 {
                (vec![mid], vec![e0, e1])
            } else {
                (vec![a.c, b.c], vec![a.c, b.c])
            };
            holes.push(Hole { px, py, tip_c: a.c, tip_r: a.r, label: format!("{}x{} SLOT", fmt_len(2.0 * a.r), fmt_len(len)) });
        }
    }
    // Holes are the part's outline when they sit outside every edge of it (a lone circle part).
    let holes: Vec<Hole> = holes;

    // --- coordinates to tie down
    let edge_lines: Vec<&Feat> = lines.iter().enumerate().filter(|(k, _)| !used_lines.get(*k).copied().unwrap_or(false)).map(|(_, l)| *l).collect();
    let mut xs_edge: Vec<f64> = Vec::new();
    let mut ys_edge: Vec<f64> = Vec::new();
    let mut chamfers: Vec<(Vec2, Vec2, f64)> = Vec::new();
    // Ends of sloping edges (a tapered tooth, a bevel).
    let mut diag: Vec<Vec2> = Vec::new();
    // A small rectangle carrying holes (a mounting pad, a plate welded on) is placed by its
    // holes; its own edges take no position.
    // Three or more rectangles of one size with no hole (posts, legs, cut-outs) are placed by
    // their centres, as a row of holes is: a pitch when evenly spaced.
    let rects = small_rects(&edge_lines, &bounds, tol, ctol);
    let mut pad_lines = vec![false; edge_lines.len()];
    let mut repeats: Vec<Hole> = Vec::new();
    for (k, (x0, x1, y0, y1, ls)) in rects.iter().enumerate() {
        let inside = |q: &Vec2| q.x > *x0 && q.x < *x1 && q.y > *y0 && q.y < *y1;
        let pad = holes.iter().any(|h| h.px.iter().chain(h.py.iter()).any(inside));
        let (w, h) = (x1 - x0, y1 - y0);
        let alike: Vec<&(f64, f64, f64, f64, [usize; 4])> = rects.iter().filter(|r| same(r.1 - r.0, w, ctol) && same(r.3 - r.2, h, ctol)).collect();
        if pad || alike.len() >= 3 {
            for i in ls {
                if let Some(o) = pad_lines.get_mut(*i) {
                    *o = true;
                }
            }
        }
        if !pad && alike.len() >= 3 && !rects.iter().take(k).any(|r| same(r.0, *x0, ctol) && same(r.2, *y0, ctol)) {
            let c = Vec2::new((x0 + x1) / 2.0, (y0 + y1) / 2.0);
            repeats.push(Hole {
                px: vec![Vec2::new(c.x, *y1)],
                py: vec![Vec2::new(*x1, c.y)],
                tip_c: c,
                tip_r: 0.0,
                label: format!("[]{}x{}", fmt_len(w), fmt_len(h)),
            });
        }
    }
    // V grooves: two sloping edges meeting at a point, opening symmetrically on a straight face.
    // A V takes its angle and its opening width; only its depth (the point) is a position.
    // (vertex, the two ends on the face, the face is vertical)
    let mut vees: Vec<(Vec2, Vec2, Vec2, bool)> = Vec::new();
    let mut vee_line = vec![false; edge_lines.len()];
    let sloping = |l: &Feat| l.kind == Kind::Line && !l.is_h(tol) && !l.is_v(tol);
    if edge_lines.len() <= 4000 {
        for i in 0..edge_lines.len() {
            for j in i + 1..edge_lines.len() {
                let (Some(l1), Some(l2)) = (edge_lines.get(i), edge_lines.get(j)) else { continue };
                if !sloping(l1) || !sloping(l2) || vee_line.get(i).copied().unwrap_or(true) || vee_line.get(j).copied().unwrap_or(true) {
                    continue;
                }
                let shared = [(l1.a, l1.b, l2.a, l2.b), (l1.a, l1.b, l2.b, l2.a), (l1.b, l1.a, l2.a, l2.b), (l1.b, l1.a, l2.b, l2.a)]
                    .into_iter()
                    .find(|(v1, _, v2, _)| near(*v1, *v2));
                let Some((v, a, _, b)) = shared else { continue };
                let on_face = |q: Vec2| edge_lines.iter().any(|f| (f.is_v(tol) || f.is_h(tol)) && (near(f.a, q) || near(f.b, q)));
                if !on_face(a) || !on_face(b) {
                    continue;
                }
                let vertical = same(a.x, b.x, ctol) && same(a.y - v.y, v.y - b.y, ctol) && !same(a.x, v.x, ctol);
                let level = same(a.y, b.y, ctol) && same(a.x - v.x, v.x - b.x, ctol) && !same(a.y, v.y, ctol);
                if !vertical && !level {
                    continue;
                }
                vees.push((v, a, b, vertical));
                for k in [i, j] {
                    if let Some(f) = vee_line.get_mut(k) {
                        *f = true;
                    }
                }
            }
        }
    }
    for (v, _, _, vertical) in &vees {
        if *vertical {
            xs_edge.push(v.x);
        } else {
            ys_edge.push(v.y);
        }
    }
    for (k, l) in edge_lines.iter().enumerate() {
        if pad_lines.get(k).copied().unwrap_or(false) || vee_line.get(k).copied().unwrap_or(false) {
            continue;
        }
        if l.is_v(tol) {
            xs_edge.push(l.a.x);
        } else if l.is_h(tol) {
            ys_edge.push(l.a.y);
        } else {
            let (dx, dy) = ((l.b.x - l.a.x).abs(), (l.b.y - l.a.y).abs());
            if same(dx, dy, 0.02 * dx.max(dy)) && dx.max(dy) < 0.25 * size {
                chamfers.push((l.a, l.b, dx));
            } else {
                xs_edge.extend([l.a.x, l.b.x]);
                ys_edge.extend([l.a.y, l.b.y]);
                diag.extend([l.a, l.b]);
            }
        }
    }
    // Fillets (up to a quarter turn) are given by their radius; larger arcs by their centre too.
    let mut fillets: Vec<&Feat> = Vec::new();
    for (k, a) in arcs.iter().enumerate() {
        if used_arcs.get(k).copied().unwrap_or(true) {
            continue;
        }
        // A disc's bore takes a diameter, not a radius.
        if a.sweep > std::f64::consts::PI && disc.is_some_and(|(dc, _)| near(dc, a.c)) {
            continue;
        }
        if a.sweep <= std::f64::consts::FRAC_PI_2 + 0.05 {
            fillets.push(a);
        } else {
            fillets.push(a);
            // Its centre is placed like a hole's: not across a centre line it sits on in the
            // middle of the part (the round end of a U slot).
            let mid = bounds.center();
            let on_cl = |v: bool| {
                feats.iter().any(|f| {
                    f.centre
                        && f.kind == Kind::Line
                        && if v { f.is_v(tol) && same(f.a.x, a.c.x, ctol) } else { f.is_h(tol) && same(f.a.y, a.c.y, ctol) }
                })
            };
            if !(same(a.c.x, mid.x, ctol) && on_cl(true)) {
                xs_edge.push(a.c.x);
            }
            if !(same(a.c.y, mid.y, ctol) && on_cl(false)) {
                ys_edge.push(a.c.y);
            }
        }
    }
    // A chamfer given by its note (C3) takes no position: the line where it starts (on a turned
    // part, the edge of its cone) is not dimensioned.
    if o.callouts {
        let mid = bounds.center();
        let chamfer_end = |q: Vec2| chamfers.iter().any(|(a, b, _)| near(*a, q) || near(*b, q));
        // A face there (a line with an end that is no chamfer's) keeps its position; only a
        // cone edge running from chamfer to chamfer is left out.
        let face_x = |x: f64| edge_lines.iter().any(|l| l.is_v(tol) && same(l.a.x, x, ctol) && !(chamfer_end(l.a) && chamfer_end(l.b)));
        let face_y = |y: f64| edge_lines.iter().any(|l| l.is_h(tol) && same(l.a.y, y, ctol) && !(chamfer_end(l.a) && chamfer_end(l.b)));
        for (a, b, _) in &chamfers {
            let ix = if (a.x - mid.x).abs() < (b.x - mid.x).abs() { a.x } else { b.x };
            let iy = if (a.y - mid.y).abs() < (b.y - mid.y).abs() { a.y } else { b.y };
            if !face_x(ix) {
                xs_edge.retain(|x| !same(*x, ix, ctol));
            }
            if !face_y(iy) {
                ys_edge.retain(|y| !same(*y, iy, ctol));
            }
        }
    }
    xs_edge.extend([bounds.min.x, bounds.max.x]);
    ys_edge.extend([bounds.min.y, bounds.max.y]);
    let xs_edge = uniq(xs_edge, ctol);
    let ys_edge = uniq(ys_edge, ctol);
    // A turned part (drawn along a centre line it is symmetric about) takes diameters across
    // its axis instead of positions from one side.
    let centre_lines: Vec<&Feat> = feats.iter().filter(|f| f.centre && f.kind == Kind::Line).collect();
    let axis = if disc.is_none() { turned_axis(&centre_lines, &edge_lines, &circles, &bounds, tol, ctol) } else { None };
    let on_axis = |h: &Hole, q: Vec2| match axis {
        Some((false, y)) => same(q.y, y, ctol) && h.py.len() == 1,
        Some((true, x)) => same(q.x, x, ctol) && h.px.len() == 1,
        None => false,
    };
    // Holes positioned from the edges: not a disc's centre bore, not a bore on a turned
    // part's axis (across it).
    let mut pos_holes: Vec<&Hole> = holes.iter().filter(|h| !(h.px.len() == 1 && disc.is_some_and(|(dc, _)| near(dc, h.tip_c)))).collect();
    // A centre line drawn across the part where no hole is drawn marks one (a hole seen from
    // the side): its position is dimensioned like a hole's.
    let mut marks: Vec<Hole> = Vec::new();
    if disc.is_none() && axis.is_none() {
        for f in centre_lines.iter().take(1000) {
            let (lo, hi) = (f.a.min(f.b), f.a.max(f.b));
            let across_x = f.is_v(tol)
                && f.a.x > bounds.min.x + ctol
                && f.a.x < bounds.max.x - ctol
                && lo.y <= bounds.min.y + ctol
                && hi.y >= bounds.max.y - ctol;
            let across_y = f.is_h(tol)
                && f.a.y > bounds.min.y + ctol
                && f.a.y < bounds.max.y - ctol
                && lo.x <= bounds.min.x + ctol
                && hi.x >= bounds.max.x - ctol;
            let taken_x = |x: f64| holes.iter().chain(marks.iter()).any(|h| h.px.iter().any(|q| same(q.x, x, ctol)));
            let taken_y = |y: f64| holes.iter().chain(marks.iter()).any(|h| h.py.iter().any(|q| same(q.y, y, ctol)));
            if across_x && !taken_x(f.a.x) {
                marks.push(Hole { px: vec![hi], py: Vec::new(), tip_c: hi, tip_r: 0.0, label: "|".into() });
            } else if across_y && !taken_y(f.a.y) {
                marks.push(Hole { px: Vec::new(), py: vec![hi], tip_c: hi, tip_r: 0.0, label: "-".into() });
            }
        }
    }
    pos_holes.extend(marks.iter());
    pos_holes.extend(repeats.iter());
    // A hole in the middle of the part on its centre line is placed by the symmetry the
    // centre line shows: no position across it.
    let mid = bounds.center();
    let sym_x = |x: f64| disc.is_none() && same(x, mid.x, ctol) && centre_lines.iter().any(|f| f.is_v(tol) && same(f.a.x, x, ctol));
    let sym_y = |y: f64| disc.is_none() && same(y, mid.y, ctol) && centre_lines.iter().any(|f| f.is_h(tol) && same(f.a.y, y, ctol));
    let xs_hole = uniq(
        pos_holes
            .iter()
            .filter(|h| !(axis.is_some_and(|a| a.0) && on_axis(h, h.tip_c)))
            .flat_map(|h| h.px.iter().map(|c| c.x))
            .filter(|x| !sym_x(*x))
            .collect(),
        ctol,
    );
    let ys_hole = uniq(
        pos_holes
            .iter()
            .filter(|h| !(axis.is_some_and(|a| !a.0) && on_axis(h, h.tip_c)))
            .flat_map(|h| h.py.iter().map(|c| c.y))
            .filter(|y| !sym_y(*y))
            .collect(),
        ctol,
    );

    // Extension line origins: the outline point at that coordinate nearest the side the
    // dimensions go (top for X, right for Y); hole centres for holes.
    let mut pts: Vec<Vec2> = Vec::new();
    for l in &edge_lines {
        pts.extend([l.a, l.b]);
    }
    for a in &arcs {
        pts.extend([a.a, a.b]);
    }
    if let Some((c, r)) = disc {
        pts.extend([c + Vec2::new(r, 0.0), c - Vec2::new(r, 0.0), c + Vec2::new(0.0, r), c - Vec2::new(0.0, r)]);
    }
    // A hole's extension line starts where the draftsman picks it: the end of its centre line
    // (the top end for an X position, the right end for a Y one); the centre when it has none.
    let cl_end = |c: Vec2, vertical: bool| -> Option<Vec2> {
        centre_lines
            .iter()
            .filter(|f| {
                let (a, b) = (f.a, f.b);
                if vertical {
                    same(a.x, c.x, ctol) && same(b.x, c.x, ctol) && a.y.min(b.y) <= c.y + ctol && a.y.max(b.y) >= c.y - ctol
                } else {
                    same(a.y, c.y, ctol) && same(b.y, c.y, ctol) && a.x.min(b.x) <= c.x + ctol && a.x.max(b.x) >= c.x - ctol
                }
            })
            .flat_map(|f| [f.a, f.b])
            .max_by(|p, q| if vertical { p.y.total_cmp(&q.y) } else { p.x.total_cmp(&q.x) })
    };
    let hole_pts_x: Vec<Vec2> = pos_holes.iter().flat_map(|h| h.px.iter().map(|c| cl_end(*c, true).unwrap_or(*c))).collect();
    let hole_pts_y: Vec<Vec2> = pos_holes.iter().flat_map(|h| h.py.iter().map(|c| cl_end(*c, false).unwrap_or(*c))).collect();
    let origin_x = |x: f64| -> Vec2 {
        hole_pts_x
            .iter()
            .chain(pts.iter())
            .filter(|p| same(p.x, x, ctol))
            .copied()
            .max_by(|a, b| a.y.total_cmp(&b.y))
            .unwrap_or(Vec2::new(x, bounds.max.y))
    };
    let origin_y = |y: f64| -> Vec2 {
        hole_pts_y
            .iter()
            .chain(pts.iter())
            .filter(|p| same(p.y, y, ctol))
            .copied()
            .max_by(|a, b| a.x.total_cmp(&b.x))
            .unwrap_or(Vec2::new(bounds.max.x, y))
    };

    // Rows from the part outwards: hole positions, outline steps, the overall size.
    // Hole positions per kind of hole (by callout), for rows of their own when one row for
    // all of them would crowd.
    let mut by_label: BTreeMap<String, (Vec<f64>, Vec<f64>)> = BTreeMap::new();
    for h in &pos_holes {
        let e = by_label.entry(h.label.clone()).or_default();
        // On a turned part's axis a hole has no position across it.
        if !(axis.is_some_and(|a| a.0) && on_axis(h, h.tip_c)) {
            e.0.extend(h.px.iter().map(|c| c.x).filter(|x| !sym_x(*x)));
        }
        if !(axis.is_some_and(|a| !a.0) && on_axis(h, h.tip_c)) {
            e.1.extend(h.py.iter().map(|c| c.y).filter(|y| !sym_y(*y)));
        }
    }
    // (positions along the axis, positions across it) per kind of hole.
    let gx: Vec<(Vec<f64>, Vec<f64>)> = by_label.values().map(|v| (v.0.clone(), v.1.clone())).collect();
    let gy: Vec<(Vec<f64>, Vec<f64>)> = by_label.values().map(|v| (v.1.clone(), v.0.clone())).collect();
    // A step is crowded when its number and arrows don't fit between its extension lines.
    let crowded = |row: &[f64]| {
        row.windows(2).any(|w| w.get(1).zip(w.first()).is_some_and(|(b, a)| b - a < th * (0.8 * fmt_len(b - a).chars().count() as f64 + 2.0)))
    };
    let rows = |hole: &[f64], groups: &[(Vec<f64>, Vec<f64>)], edge: &[f64], lo: f64, hi: f64| -> Vec<Vec<f64>> {
        let mut rows: Vec<Vec<f64>> = Vec::new();
        if !hole.is_empty() {
            // A kind of hole in an evenly spaced row gets a row of its own, so it takes a pitch.
            let pitched = |g: &[f64]| {
                let v = uniq(g.to_vec(), ctol);
                v.len() >= o.pitch_min.max(3)
                    && chain(&uniq(v.iter().copied().chain([lo, hi]).collect(), ctol), o.pitch_min, ctol).iter().any(|c| !c.2.is_empty())
            };
            let (own, rest): (Vec<&(Vec<f64>, Vec<f64>)>, Vec<&(Vec<f64>, Vec<f64>)>) =
                groups.iter().partition(|g| groups.len() > 1 && pitched(&g.0));
            let merged = uniq(rest.iter().flat_map(|g| g.0.iter().copied()).chain([lo, hi]).collect(), ctol);
            // Kinds of hole lying in separate bands across the axis (a row up here, another down
            // there) are dimensioned row by row when one chain would crowd.
            if !rest.is_empty() {
                // One row per kind of hole when a single chain would crowd (as hand-drafted:
                // the bolt holes in one row, the slots in another).
                if crowded(&merged) && rest.len() > 1 {
                    for g in &rest {
                        rows.push(uniq(g.0.iter().copied().chain([lo, hi]).collect(), ctol));
                    }
                } else {
                    rows.push(merged);
                }
            }
            for g in own {
                rows.push(uniq(g.0.iter().copied().chain([lo, hi]).collect(), ctol));
            }
        }
        if edge.len() > 2 {
            rows.push(edge.to_vec());
        }
        rows.push(vec![lo, hi]);
        let mut out: Vec<Vec<f64>> = Vec::new();
        for r in rows {
            if r.len() >= 2 && !out.iter().any(|o| o.len() == r.len() && o.iter().zip(&r).all(|(a, b)| same(*a, *b, ctol))) {
                out.push(r);
            }
        }
        out
    };
    let mut p = Plan::default();
    let gap0 = th * 3.0;
    let step = th * 2.2;
    if let Some((c, r)) = disc {
        // A disc: its diameter, and the hole positions from its centre.
        let at = c + Vec2::from_angle(std::f64::consts::FRAC_PI_4) * r;
        p.dims.push(super::annotate::dim(s, DimKind::Diameter, c * 2.0 - at, Vec2::ZERO, Vec2::ZERO, at, Vec2::ZERO, ""));
        // The rings inside, each pointed at a different angle so the notes don't collide.
        for (i, r) in rings.iter().rev().enumerate().take(16) {
            let ang = std::f64::consts::FRAC_PI_4 + std::f64::consts::FRAC_PI_2 * (1 + i) as f64 + 0.2 * (i / 3) as f64;
            let at = c + Vec2::from_angle(ang) * *r;
            p.dims.push(super::annotate::dim(s, DimKind::Diameter, c * 2.0 - at, Vec2::ZERO, Vec2::ZERO, at, Vec2::ZERO, ""));
        }
    }
    for (c, rs) in &bores {
        for (i, r) in rs.iter().rev().enumerate().take(8) {
            // Up-right, then down-left (a counterbore's two), then the other diagonal.
            let ang = std::f64::consts::FRAC_PI_4 + std::f64::consts::PI * (i % 2) as f64 + std::f64::consts::FRAC_PI_2 * (i / 2) as f64;
            let at = *c + Vec2::from_angle(ang) * *r;
            p.dims.push(super::annotate::dim(s, DimKind::Diameter, *c * 2.0 - at, Vec2::ZERO, Vec2::ZERO, at, Vec2::ZERO, ""));
        }
    }
    if let Some((c, _)) = disc {
        disc_bore_and_key(s, &mut p, c, &arcs, &edge_lines, ctol, gap0);
    }
    if let Some((vertical, at)) = axis {
        axis_diameters(s, &mut p, &edge_lines, &outline, vertical, at, ctol, th);
    }
    // Steps of the outline that can't be seen from above (hidden under another edge: the lower
    // edge of a bent plate) are dimensioned from below, in a row of their own under the part;
    // likewise those hidden from the right go to the left.
    let mut segs: Vec<(Vec2, Vec2)> = edge_lines.iter().map(|l| (l.a, l.b)).collect();
    for (k, a) in arcs.iter().enumerate() {
        if used_arcs.get(k).copied().unwrap_or(false) {
            continue;
        }
        let a0 = a.c.angle_to(a.a);
        let mut prev = a.a;
        for j in 1..=16 {
            let q = a.c + Vec2::from_angle(a0 + a.sweep * j as f64 / 16.0) * a.r;
            segs.push((prev, q));
            prev = q;
        }
    }
    let clear = |q: Vec2, d: Vec2| !segs.iter().any(|(a, b)| ray_hits(q + d * (ctol * 2.0), d, *a, *b));
    let extreme = |x: f64, vertical: bool, most: bool| -> Option<Vec2> {
        let on = pts.iter().filter(|q| same(if vertical { q.x } else { q.y }, x, ctol)).copied();
        let key = |q: &Vec2| if vertical { q.y } else { q.x };
        if most { on.max_by(|a, b| key(a).total_cmp(&key(b))) } else { on.min_by(|a, b| key(a).total_cmp(&key(b))) }
    };
    let low_point = |v: f64, vertical: bool| -> bool {
        let (out, back) = if vertical { (Vec2::new(0.0, 1.0), Vec2::new(0.0, -1.0)) } else { (Vec2::new(1.0, 0.0), Vec2::new(-1.0, 0.0)) };
        let (Some(t), Some(b)) = (extreme(v, vertical, true), extreme(v, vertical, false)) else { return false };
        if !clear(b, back) {
            return false;
        }
        // Hidden from the far side, or much nearer this one (the foot of a tall part: a short
        // extension line from below, not a long one from above).
        let (far, near) = if vertical { (bounds.max.y - t.y, b.y - bounds.min.y) } else { (bounds.max.x - t.x, b.x - bounds.min.x) };
        !clear(t, out) || far > 2.0 * near + th * 4.0
    };
    let hidden_side = |vals: &[f64], vertical: bool, lo: f64, hi: f64| -> Vec<f64> {
        vals.iter().copied().filter(|v| !same(*v, lo, ctol) && !same(*v, hi, ctol)).filter(|v| low_point(*v, vertical)).collect()
    };
    // Along a turned part's axis only its faces are dimensioned, not where a taper turns
    // (a sprocket's tooth flanks).
    let face_only = |vals: Vec<f64>, vertical: bool| -> Vec<f64> {
        let ls: Vec<f64> =
            edge_lines.iter().filter(|l| if vertical { l.is_v(tol) } else { l.is_h(tol) }).map(|l| if vertical { l.a.x } else { l.a.y }).collect();
        let lim = if vertical { (bounds.min.x, bounds.max.x) } else { (bounds.min.y, bounds.max.y) };
        vals.into_iter()
            .filter(|v| {
                let sloping = diag.iter().any(|q| same(if vertical { q.x } else { q.y }, *v, ctol));
                !sloping || ls.iter().any(|f| same(*f, *v, ctol)) || same(*v, lim.0, ctol) || same(*v, lim.1, ctol)
            })
            .collect()
    };
    let (xs_edge, ys_edge) = match axis {
        Some((false, _)) => (face_only(xs_edge, true), ys_edge),
        Some((true, _)) => (xs_edge, face_only(ys_edge, false)),
        None => (xs_edge, ys_edge),
    };
    let x_edge_rows = disc.is_none() && !axis.is_some_and(|a| a.0);
    let y_edge_rows = disc.is_none() && !axis.is_some_and(|a| !a.0);
    let xs_low = if x_edge_rows { hidden_side(&xs_edge, true, bounds.min.x, bounds.max.x) } else { Vec::new() };
    let ys_left = if y_edge_rows { hidden_side(&ys_edge, false, bounds.min.y, bounds.max.y) } else { Vec::new() };
    let xs_top: Vec<f64> = xs_edge.iter().copied().filter(|x| !xs_low.iter().any(|l| same(*l, *x, ctol))).collect();
    let ys_right: Vec<f64> = ys_edge.iter().copied().filter(|y| !ys_left.iter().any(|l| same(*l, *y, ctol))).collect();
    // A disc's holes are placed from its centre or from each other, never from the rim.
    let disc_row = |v: &[f64], c: f64| -> Vec<Vec<f64>> {
        let row = if v.len() >= 2 {
            v.to_vec()
        } else if v.first().is_some_and(|x| !same(*x, c, ctol)) {
            uniq(v.iter().copied().chain([c]).collect(), ctol)
        } else {
            Vec::new()
        };
        if row.len() >= 2 { vec![row] } else { Vec::new() }
    };
    // Both ends of the part at its foot (a base plate under an upright): the row above spans
    // the upright only, and the overall size goes below with the foot's steps.
    let top_inner: Vec<f64> = xs_top.iter().copied().filter(|x| !same(*x, bounds.min.x, ctol) && !same(*x, bounds.max.x, ctol)).collect();
    let foot = x_edge_rows
        && top_inner.len() >= 2
        && low_point(bounds.min.x, true)
        && low_point(bounds.max.x, true)
        && xs_hole.iter().all(|x| top_inner.first().is_some_and(|a| *x >= *a - ctol) && top_inner.last().is_some_and(|b| *x <= *b + ctol));
    let x_rows = if let Some((c, _)) = disc {
        disc_row(&xs_hole, c.x)
    } else if !x_edge_rows {
        rows(&xs_hole, &gx, &[], bounds.min.x, bounds.max.x).into_iter().filter(|r| r.len() > 2).collect()
    } else if foot {
        let (a, b) = (top_inner.first().copied().unwrap_or(bounds.min.x), top_inner.last().copied().unwrap_or(bounds.max.x));
        rows(&xs_hole, &gx, &top_inner, a, b)
    } else {
        rows(&xs_hole, &gx, &xs_top, bounds.min.x, bounds.max.x)
    };
    let y_rows = if let Some((c, _)) = disc {
        disc_row(&ys_hole, c.y)
    } else if !y_edge_rows {
        rows(&ys_hole, &gy, &[], bounds.min.y, bounds.max.y).into_iter().filter(|r| r.len() > 2).collect()
    } else {
        rows(&ys_hole, &gy, &ys_right, bounds.min.y, bounds.max.y)
    };
    // A V's opening width, nearest the part on the side its face is on.
    let (mut x_rows, mut y_rows) = (x_rows, y_rows);
    for (_, a, b, vertical) in &vees {
        if *vertical {
            y_rows.insert(0, vec![a.y.min(b.y), a.y.max(b.y)]);
        } else {
            x_rows.insert(0, vec![a.x.min(b.x), a.x.max(b.x)]);
        }
    }
    for (v, a, b, _) in &vees {
        let bis = ((*a - *v).normalized() + (*b - *v).normalized()).normalized();
        let at = *v + bis * (a.dist(*v) * 0.6);
        p.dims.push(super::annotate::dim(s, DimKind::Angular3P, at, *a, *b, *v, Vec2::ZERO, ""));
    }
    let mut low_rows: Vec<Vec<f64>> =
        if xs_low.is_empty() { Vec::new() } else { vec![uniq(xs_low.iter().copied().chain([bounds.min.x, bounds.max.x]).collect(), ctol)] };
    if foot {
        low_rows.push(vec![bounds.min.x, bounds.max.x]);
    }
    let left_rows: Vec<Vec<f64>> =
        if ys_left.is_empty() { Vec::new() } else { vec![uniq(ys_left.iter().copied().chain([bounds.min.y, bounds.max.y]).collect(), ctol)] };
    // Each row from the part outwards; a step too short for its text alternates with its
    // neighbour onto a row of its own further out, so numbers don't run into each other.
    // Each row on one straight line from the part outwards, as drafted by hand: a step too
    // short for its number keeps its place in the row (the style's fit puts the number
    // outside the extension lines).
    let place = |rows: &[Vec<f64>]| -> Vec<(usize, f64, f64, String)> {
        let mut out = Vec::new();
        for (level, row) in rows.iter().enumerate() {
            for (a, b, text) in chain(row, o.pitch_min, ctol) {
                out.push((level, a, b, text));
            }
        }
        out
    };
    let x_place = place(&x_rows);
    let y_place = place(&y_rows);
    let levels = |v: &[(usize, f64, f64, String)]| v.iter().map(|x| x.0 + 1).max().unwrap_or(0);
    // How far out the dimension rows reach above and to the right of the part.
    let top_reach = if x_place.is_empty() { 0.0 } else { gap0 + step * levels(&x_place) as f64 };
    let right_reach = if y_place.is_empty() { 0.0 } else { gap0 + step * levels(&y_place) as f64 };
    let low_place = place(&low_rows);
    let left_place = place(&left_rows);
    let bottom_reach = if low_place.is_empty() { 0.0 } else { gap0 + step * levels(&low_place) as f64 };
    let left_reach = if left_place.is_empty() { 0.0 } else { gap0 + step * levels(&left_place) as f64 };
    // A step too short for its number has it outside, past one extension line: the outer one
    // at a row's ends, the roomier side in between, so it doesn't land on its neighbour's.
    // 0: the style's place; 1: past the first extension line; 2: lifted clear of the row over
    // its own dimension line (no room either side).
    let first_side = |v: &[(usize, f64, f64, String)]| -> Vec<u8> {
        let shown = |a: f64, b: f64, t: &str| if t.is_empty() { fmt_len(b - a) } else { t.replace("<>", &fmt_len(b - a)) };
        let width = |a: f64, b: f64, t: &str| th * 0.8 * shown(a, b, t).chars().count() as f64;
        let need = |a: f64, b: f64, t: &str| width(a, b, t) + th * 2.4;
        (0..v.len())
            .map(|i| {
                let Some((k, a, b, t)) = v.get(i) else { return 0 };
                if b - a >= need(*a, *b, t) {
                    return 0;
                }
                let prev = i.checked_sub(1).and_then(|j| v.get(j)).filter(|q| q.0 == *k);
                let next = v.get(i + 1).filter(|q| q.0 == *k);
                match (prev, next) {
                    (None, _) => 1,
                    (Some(_), None) => 0,
                    (Some(pq), Some(nq)) => {
                        // Room beside a neighbour's own centred number.
                        let room = |q: &(usize, f64, f64, String)| ((q.2 - q.1) - width(q.1, q.2, &q.3)) / 2.0 - th * 1.5;
                        let own = width(*a, *b, t);
                        if room(pq) >= own && room(pq) >= room(nq) {
                            1
                        } else if room(nq) >= own {
                            0
                        } else {
                            2
                        }
                    }
                }
            })
            .collect()
    };
    let lift = |e: &mut EntityKind, at: Vec2| {
        if let EntityKind::Dimension(dm) = e {
            dm.text_mid = cadcraft_geom::Vec3::new(at.x, at.y, 0.0);
            dm.user_text_pos = true;
        }
    };
    let text_first = |e: &mut EntityKind, at: Vec2| {
        if let EntityKind::Dimension(dm) = e {
            dm.text_mid = cadcraft_geom::Vec3::new(at.x, at.y, 0.0);
        }
    };
    let (sx, sy, sl, sf) = (first_side(&x_place), first_side(&y_place), first_side(&low_place), first_side(&left_place));
    for (i, (k, a, b, text)) in low_place.into_iter().enumerate() {
        let y = bounds.min.y - gap0 - step * k as f64;
        let o_low = |x: f64| extreme(x, true, false).unwrap_or(Vec2::new(x, bounds.min.y));
        let mut e = super::annotate::dim(s, DimKind::Linear { rotation: 0.0 }, Vec2::new(a, y), o_low(a), o_low(b), Vec2::ZERO, Vec2::ZERO, &text);
        match sl.get(i).copied().unwrap_or(0) {
            1 => text_first(&mut e, Vec2::new(a - th * 3.0, y)),
            2 => lift(&mut e, Vec2::new((a + b) / 2.0, y - th * 1.4)),
            _ => {}
        }
        p.dims.push(e);
    }
    for (i, (k, a, b, text)) in left_place.into_iter().enumerate() {
        let x = bounds.min.x - gap0 - step * k as f64;
        let o_left = |y: f64| extreme(y, false, false).unwrap_or(Vec2::new(bounds.min.x, y));
        let mut e = super::annotate::dim(
            s,
            DimKind::Linear { rotation: std::f64::consts::FRAC_PI_2 },
            Vec2::new(x, a),
            o_left(a),
            o_left(b),
            Vec2::ZERO,
            Vec2::ZERO,
            &text,
        );
        match sf.get(i).copied().unwrap_or(0) {
            1 => text_first(&mut e, Vec2::new(x, a - th * 3.0)),
            2 => lift(&mut e, Vec2::new(x - th * 1.4, (a + b) / 2.0)),
            _ => {}
        }
        p.dims.push(e);
    }
    for (i, (k, a, b, text)) in x_place.into_iter().enumerate() {
        let y = bounds.max.y + gap0 + step * k as f64;
        let mut e =
            super::annotate::dim(s, DimKind::Linear { rotation: 0.0 }, Vec2::new(a, y), origin_x(a), origin_x(b), Vec2::ZERO, Vec2::ZERO, &text);
        match sx.get(i).copied().unwrap_or(0) {
            1 => text_first(&mut e, Vec2::new(a - th * 3.0, y)),
            2 => lift(&mut e, Vec2::new((a + b) / 2.0, y + th * 1.4)),
            _ => {}
        }
        p.dims.push(e);
    }
    for (i, (k, a, b, text)) in y_place.into_iter().enumerate() {
        let x = bounds.max.x + gap0 + step * k as f64;
        let mut e = super::annotate::dim(
            s,
            DimKind::Linear { rotation: std::f64::consts::FRAC_PI_2 },
            Vec2::new(x, a),
            origin_y(a),
            origin_y(b),
            Vec2::ZERO,
            Vec2::ZERO,
            &text,
        );
        match sy.get(i).copied().unwrap_or(0) {
            1 => text_first(&mut e, Vec2::new(x, a - th * 3.0)),
            2 => lift(&mut e, Vec2::new(x + th * 1.4, (a + b) / 2.0)),
            _ => {}
        }
        p.dims.push(e);
    }

    // --- callouts below and to the left of the part
    if o.callouts {
        let mut groups: BTreeMap<String, Vec<&Hole>> = BTreeMap::new();
        for h in &holes {
            groups.entry(h.label.clone()).or_default().push(h);
        }
        // Notes: each leaves the part the shortest way at 45° (preferring down and left, away
        // from the dimension rows; past them when it must go their way), and moves further
        // out until its text clears the notes already placed.
        let dirs = [Vec2::new(-1.0, -1.0), Vec2::new(1.0, -1.0), Vec2::new(-1.0, 1.0), Vec2::new(1.0, 1.0)];
        let exit = |c: Vec2, d: Vec2| -> f64 {
            let tx = if d.x < 0.0 { c.x - bounds.min.x } else { bounds.max.x - c.x };
            let ty = if d.y < 0.0 { c.y - bounds.min.y } else { bounds.max.y - c.y };
            tx.min(ty).max(0.0) * std::f64::consts::SQRT_2
        };
        let over = |d: Vec2| {
            let rx = if d.x > 0.0 { right_reach } else { left_reach };
            let ry = if d.y > 0.0 { top_reach } else { bottom_reach };
            rx.max(ry) * std::f64::consts::SQRT_2
        };
        let penalty = [1.0, 1.15, 3.0, 4.0];
        // (text, candidates: tip, direction index, distance out to the part's edge)
        let mut wanted: Vec<(String, Vec<(Vec2, usize, f64)>)> = Vec::new();
        for (label, hs) in &groups {
            if label.is_empty() {
                continue;
            }
            let count: usize = hs.len();
            let text = if count > 1 { format!("{count}-{label}") } else { label.clone() };
            let mut c = Vec::new();
            for h in hs.iter() {
                for (i, d) in dirs.iter().enumerate() {
                    let d = d.normalized();
                    c.push((h.tip_c + d * h.tip_r, i, (exit(h.tip_c, d) - h.tip_r).max(0.0)));
                }
            }
            wanted.push((text, c));
        }
        // Inside the part's material: an odd number of outline crossings to the right.
        let inside_material = |q: Vec2| -> bool {
            let mut n = 0usize;
            let mut cross = |a: Vec2, b: Vec2| {
                if (a.y > q.y) != (b.y > q.y) {
                    let x = a.x + (q.y - a.y) / (b.y - a.y) * (b.x - a.x);
                    if x > q.x {
                        n += 1;
                    }
                }
            };
            for l in &lines {
                cross(l.a, l.b);
            }
            for a in &arcs {
                let k = 16;
                let a0 = a.c.angle_to(a.a);
                let mut prev = a.a;
                for j in 1..=k {
                    let pt = a.c + Vec2::from_angle(a0 + a.sweep * j as f64 / k as f64) * a.r;
                    cross(prev, pt);
                    prev = pt;
                }
            }
            n % 2 == 1
        };
        let mut seen_r: Vec<f64> = Vec::new();
        for a in &fillets {
            if seen_r.iter().any(|r| same(*r, a.r, tol.max(a.r * 1e-4))) {
                continue;
            }
            seen_r.push(a.r);
            let all: Vec<&&Feat> = fillets.iter().filter(|f| same(f.r, a.r, tol.max(a.r * 1e-4))).collect();
            let n = all.len();
            let text = if n > 1 { format!("{n}-R{}", fmt_len(a.r)) } else { format!("R{}", fmt_len(a.r)) };
            let mut c = Vec::new();
            for f in &all {
                let mid = f.c + Vec2::from_angle(f.a.sub_angle(f.c) + f.sweep / 2.0) * f.r;
                let out = (mid - f.c).normalized();
                // A rounded corner has its centre in the material; a notch or slot end (a
                // hollow) has it in the open, and its note may come out through the hollow.
                let hollow = !inside_material(f.c);
                for (i, d) in dirs.iter().enumerate() {
                    let d = d.normalized();
                    if d.dot(out) > 0.3 || (hollow && d.dot(out) < -0.3) {
                        let len = if d.dot(out) > 0.0 { exit(mid, d) } else { exit(mid, d).max(2.0 * f.r) };
                        c.push((mid, i, len));
                    }
                }
            }
            wanted.push((text, c));
        }
        let mut seen_c: Vec<f64> = Vec::new();
        for (_, _, leg) in &chamfers {
            if seen_c.iter().any(|c| same(*c, *leg, tol.max(leg * 1e-4))) {
                continue;
            }
            seen_c.push(*leg);
            let all: Vec<&(Vec2, Vec2, f64)> = chamfers.iter().filter(|(_, _, l)| same(*l, *leg, tol.max(leg * 1e-4))).collect();
            // On a turned part each chamfer is drawn twice, mirrored about the axis.
            let n = match axis {
                Some((v, at)) => all
                    .iter()
                    .filter(|(a, b, _)| {
                        let m = (*a + *b) * 0.5;
                        if v { m.x > at } else { m.y > at }
                    })
                    .count()
                    .max(1),
                None => all.len(),
            };
            let text = if n > 1 { format!("{n}-C{}", fmt_len(*leg)) } else { format!("C{}", fmt_len(*leg)) };
            let mut c = Vec::new();
            for (a, b, _) in &all {
                let mid = (*a + *b) * 0.5;
                let nrm = (*b - *a).perp().normalized();
                let nrm = if nrm.dot(mid - bounds.center()) < 0.0 { -nrm } else { nrm };
                for (i, d) in dirs.iter().enumerate() {
                    let d = d.normalized();
                    if d.dot(nrm) > 0.3 {
                        c.push((mid, i, exit(mid, d)));
                    }
                }
            }
            wanted.push((text, c));
        }
        let mut used = [0usize; 4];
        let mut boxes: Vec<Bounds2> = Vec::new();
        for (text, cands) in wanted {
            let Some((tip, i, ex)) = cands.iter().copied().min_by(|x, y| {
                let cost = |c: &(Vec2, usize, f64)| {
                    let d = dirs.get(c.1).copied().unwrap_or_default().normalized();
                    (c.2 + th + over(d)) * penalty.get(c.1).copied().unwrap_or(2.0) + used.get(c.1).copied().unwrap_or(0) as f64 * th * 2.0
                };
                cost(x).total_cmp(&cost(y))
            }) else {
                continue;
            };
            if let Some(u) = used.get_mut(i) {
                *u += 1;
            }
            let d = dirs.get(i).copied().unwrap_or(Vec2::new(-1.0, -1.0)).normalized();
            let mut reach = ex + th * 2.0;
            // Past the dimension rows when it goes their way.
            let k0 = tip + d * reach;
            // Only where the rows are: beside the part for the right ones, above it for the top.
            // (Row numbers can sit past a row's ends, outside short steps.)
            let beside = k0.y > bounds.min.y - bottom_reach - th * 6.0 && k0.y < bounds.max.y + top_reach + th * 6.0;
            let above = k0.x > bounds.min.x - left_reach - th * 6.0 && k0.x < bounds.max.x + right_reach + th * 6.0;
            let need_x = if d.x > 0.0 && beside {
                (bounds.max.x + right_reach + th - k0.x).max(0.0)
            } else if d.x < 0.0 && beside && left_reach > 0.0 {
                (k0.x - (bounds.min.x - left_reach - th)).max(0.0)
            } else {
                0.0
            };
            let need_y = if d.y > 0.0 && above {
                (bounds.max.y + top_reach + th - k0.y).max(0.0)
            } else if d.y < 0.0 && above && bottom_reach > 0.0 {
                (k0.y - (bounds.min.y - bottom_reach - th)).max(0.0)
            } else {
                0.0
            };
            reach += need_x.max(need_y) * std::f64::consts::SQRT_2;
            let w = th * (0.8 * text.chars().count() as f64 + 1.0);
            // The text sits on the leader's landing, which runs on under it.
            let text_box = |knee: Vec2| {
                let x0 = if d.x < 0.0 { knee.x - w - th } else { knee.x };
                Bounds2::new(Vec2::new(x0, knee.y - th * 0.3), Vec2::new(x0 + w + th, knee.y + th * 1.6))
            };
            let overlaps = |a: &Bounds2, b: &Bounds2| a.min.x < b.max.x && b.min.x < a.max.x && a.min.y < b.max.y && b.min.y < a.max.y;
            let mut knee = tip + d * reach;
            for _ in 0..40 {
                let tb = text_box(knee);
                if !boxes.iter().any(|b| overlaps(b, &tb)) {
                    break;
                }
                knee += d * (th * 1.2);
            }
            boxes.push(text_box(knee));
            p.notes.push((vec![tip, knee], text));
        }
    }
    if let Some(t) = o.thickness.filter(|t| *t > 0.0 && t.is_finite()) {
        p.labels.push((bounds.center(), format!("t={}", fmt_len(t))));
    }
    Ok(p)
}

/// A gear or sprocket outline without its teeth: when eight or more outline arcs of one radius
/// sit evenly round one centre (the tooth roots), the curves out beyond the roots are replaced
/// by the tip circle. None when the outline has no teeth.
fn without_teeth(feats: &[Feat]) -> Option<Vec<Feat>> {
    let arcs: Vec<&Feat> = feats.iter().filter(|f| !f.hidden && f.kind == Kind::Arc).collect();
    let mut tried: Vec<f64> = Vec::new();
    for a in arcs.iter().take(2000) {
        if tried.iter().any(|r| same(*r, a.r, a.r * 1e-3)) {
            continue;
        }
        tried.push(a.r);
        let ring: Vec<&&Feat> = arcs.iter().filter(|b| same(b.r, a.r, a.r * 1e-3)).collect();
        if ring.len() < 8 {
            continue;
        }
        let n = ring.len() as f64;
        let o = ring.iter().fold(Vec2::ZERO, |acc, b| acc + b.c) * (1.0 / n);
        let d: Vec<f64> = ring.iter().map(|b| b.c.dist(o)).collect();
        let mean = d.iter().sum::<f64>() / n;
        if mean <= 2.0 * a.r || d.iter().any(|x| !same(*x, mean, mean * 0.01)) {
            continue;
        }
        let root = mean - a.r;
        let beyond = |q: Vec2| q.dist(o) >= root * 0.97;
        let tooth = |f: &Feat| -> bool {
            !f.hidden
                && match f.kind {
                    Kind::Line => beyond(f.a) && beyond(f.b),
                    Kind::Arc => beyond(f.a) && beyond(f.b) && f.c.dist(o) > root * 0.5,
                    Kind::Circle => false,
                }
        };
        let mut tip = 0.0_f64;
        for f in feats.iter().filter(|f| tooth(f)) {
            tip = tip.max(f.a.dist(o)).max(f.b.dist(o));
            if f.kind == Kind::Arc {
                let mid = f.c + Vec2::from_angle(f.c.angle_to(f.a) + f.sweep / 2.0) * f.r;
                tip = tip.max(mid.dist(o));
            }
        }
        if tip <= root {
            continue;
        }
        let mut out: Vec<Feat> = feats.iter().filter(|f| !tooth(f)).copied().collect();
        out.push(Feat {
            kind: Kind::Circle,
            hidden: false,
            a: o,
            b: o,
            c: o,
            r: tip,
            sweep: std::f64::consts::TAU,
            block_area: None,
            layer_annot: false,
            centre: false,
        });
        return Some(out);
    }
    None
}

/// Small rectangles (under a twentieth of the part's area) each drawn as four lines of its
/// own: (x0, x1, y0, y1, the four lines' indices).
fn small_rects(lines: &[&Feat], b: &Bounds2, tol: f64, ctol: f64) -> Vec<(f64, f64, f64, f64, [usize; 4])> {
    let mut out = Vec::new();
    if lines.len() > 4000 {
        return out;
    }
    let area = b.width() * b.height();
    let span = |l: &Feat, h: bool| if h { (l.a.x.min(l.b.x), l.a.x.max(l.b.x)) } else { (l.a.y.min(l.b.y), l.a.y.max(l.b.y)) };
    for (i, h1) in lines.iter().enumerate() {
        if !h1.is_h(tol) {
            continue;
        }
        let (x0, x1) = span(h1, true);
        for (j, h2) in lines.iter().enumerate() {
            if j <= i || !h2.is_h(tol) || same(h1.a.y, h2.a.y, ctol) {
                continue;
            }
            let (u0, u1) = span(h2, true);
            if !same(x0, u0, ctol) || !same(x1, u1, ctol) {
                continue;
            }
            let (y0, y1) = (h1.a.y.min(h2.a.y), h1.a.y.max(h2.a.y));
            if (x1 - x0) * (y1 - y0) > area * 0.05 {
                continue;
            }
            let side = |x: f64| {
                lines.iter().position(|v| {
                    v.is_v(tol) && same(v.a.x, x, ctol) && {
                        let (a, c) = span(v, false);
                        same(a, y0, ctol) && same(c, y1, ctol)
                    }
                })
            };
            if let (Some(l), Some(r)) = (side(x0), side(x1)) {
                out.push((x0, x1, y0, y1, [i, j, l, r]));
                if out.len() >= 2000 {
                    return out;
                }
            }
        }
    }
    out
}

/// The axis of a turned part: a centre line running most of the part's length, with the
/// outline mirrored about it. `(true, x)` for a vertical axis at `x`, `(false, y)` for a
/// horizontal one.
fn turned_axis(centre_lines: &[&Feat], edges: &[&Feat], circles: &[&Feat], b: &Bounds2, tol: f64, ctol: f64) -> Option<(bool, f64)> {
    for vertical in [false, true] {
        let sw = |v: Vec2| if vertical { Vec2::new(v.y, v.x) } else { v };
        let (lo, hi) = (sw(b.min), sw(b.max));
        let (len, across) = (hi.x - lo.x, hi.y - lo.y);
        if len <= 0.0 || across <= 0.0 {
            continue;
        }
        // Centre lines of this direction across the part: more than one are hole marks, not an axis.
        let marks: Vec<&&Feat> = centre_lines
            .iter()
            .filter(|cl| {
                let (a, e) = (sw(cl.a), sw(cl.b));
                same(a.y, e.y, tol) && a.y > lo.y && a.y < hi.y
            })
            .collect();
        // One position across the part (a hole's short centre line may lie on the axis too):
        // the longest line there.
        let levels = uniq(marks.iter().map(|cl| sw(cl.a).y).collect(), ctol);
        if levels.len() != 1 {
            continue;
        }
        let longest = marks.iter().max_by(|p, q| (sw(p.a).x - sw(p.b).x).abs().total_cmp(&(sw(q.a).x - sw(q.b).x).abs()));
        for cl in longest.into_iter() {
            let (a, e) = (sw(cl.a), sw(cl.b));
            // A hole's centre line runs through its circle (just past it); an axis runs on
            // along the part, and may have a cross hole on it.
            let on_line: Vec<&&Feat> = circles.iter().filter(|c| same(sw(c.c).y, a.y, ctol) && sw(c.c).x > lo.x && sw(c.c).x < hi.x).collect();
            if on_line.iter().any(|c| (a.x - e.x).abs() < 5.0 * c.r) || uniq(on_line.iter().map(|c| sw(c.c).x).collect(), ctol).len() >= 2 {
                continue;
            }
            if !same(a.y, e.y, tol) || (a.x - e.x).abs() < 0.6 * len || a.y <= lo.y || a.y >= hi.y {
                continue;
            }
            let y = a.y;
            if !same(hi.y - y, y - lo.y, (across * 0.01).max(ctol)) {
                continue;
            }
            // Lines along the axis (off it) mirrored about it.
            let along: Vec<(Vec2, Vec2)> = edges
                .iter()
                .map(|l| (sw(l.a), sw(l.b)))
                .filter(|(p, q)| same(p.y, q.y, tol) && (p.x - q.x).abs() > tol && !same(p.y, y, ctol))
                .collect();
            if along.len() < 2 {
                continue;
            }
            let mirrored = along
                .iter()
                .filter(|(p, q)| {
                    let (x0, x1) = (p.x.min(q.x), p.x.max(q.x));
                    along.iter().any(|(r, t)| same(r.y, 2.0 * y - p.y, ctol) && r.x.min(t.x) < x1 - ctol && r.x.max(t.x) > x0 + ctol)
                })
                .count();
            if mirrored * 5 >= along.len() * 4 {
                return Some((vertical, y));
            }
        }
    }
    None
}

/// Diameters of a turned part across its axis: one per diameter, on the longest stretch at that
/// diameter (dimension lines kept apart along the part).
#[allow(clippy::too_many_arguments)]
fn axis_diameters(s: &Session, p: &mut Plan, edges: &[&Feat], all: &[&Feat], vertical: bool, y: f64, ctol: f64, th: f64) {
    let sw = |v: Vec2| if vertical { Vec2::new(v.y, v.x) } else { v };
    // (radius, from x, to x) of each stretch on one side of the axis that has a mirror.
    let along: Vec<(f64, f64, f64)> = edges
        .iter()
        .map(|l| (sw(l.a), sw(l.b)))
        .filter(|(a, b)| same(a.y, b.y, ctol) && (a.x - b.x).abs() > ctol && a.y > y + ctol)
        .map(|(a, b)| (a.y - y, a.x.min(b.x), a.x.max(b.x)))
        .filter(|(r, x0, x1)| {
            edges.iter().any(|l| {
                let (c, d) = (sw(l.a), sw(l.b));
                same(c.y, y - r, ctol) && same(d.y, y - r, ctol) && c.x.min(d.x) < x1 - ctol && c.x.max(d.x) > x0 + ctol
            })
        })
        .collect();
    let radii = uniq(along.iter().map(|a| a.0).collect(), ctol);
    let mut placed: Vec<f64> = Vec::new();
    let xs_all: Vec<f64> = edges.iter().flat_map(|l| [sw(l.a).x, sw(l.b).x]).collect();
    let xmin = xs_all.iter().copied().fold(f64::INFINITY, f64::min);
    let xmax = xs_all.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let (mut out_left, mut out_right) = (0usize, 0usize);
    // Largest first: the outside diameters take the middle of their stretch.
    for r in radii.iter().rev().take(64) {
        let Some((_, x0, x1)) = along.iter().filter(|a| same(a.0, *r, ctol)).copied().max_by(|a, b| (a.2 - a.1).total_cmp(&(b.2 - b.1))) else {
            continue;
        };
        let Some(x) = [0.5, 0.3, 0.7, 0.15, 0.85].iter().map(|f| x0 + (x1 - x0) * f).find(|x| {
            // Clear of the other dimensions, and not across a feature inside (a slot, a bore).
            !placed.iter().any(|q| (q - x).abs() < th * 2.5)
                && !all.iter().any(|f| {
                    // Anything drawn inside this diameter (a keyway slot, a cross hole) under it.
                    // The part's own steps are no obstacle: lines along the axis at one of its
                    // diameters, and faces across it (the dimension line runs beside them).
                    let (a, c) = (sw(f.a), sw(f.b));
                    if f.kind == Kind::Line {
                        if same(a.y, c.y, ctol) && radii.iter().any(|q| same((a.y - y).abs(), *q, ctol)) {
                            return false;
                        }
                        if same(a.x, c.x, ctol) {
                            return a.y.min(c.y) > y - r + ctol && a.y.max(c.y) < y + r - ctol && (a.x - *x).abs() <= ctol;
                        }
                    }
                    let b = f.bounds();
                    let (lo, hi) = (sw(b.min), sw(b.max));
                    let (lo, hi) = (lo.min(hi), lo.max(hi));
                    lo.y > y - r + ctol && hi.y < y + r - ctol && lo.x - th < *x && hi.x + th > *x
                })
        }) else {
            // No room on the part (a short hub beside a flange): beside the part's end nearest
            // the stretch, one row out per diameter put there.
            let right = (xmax - x1) <= (x0 - xmin);
            let n = if right { &mut out_right } else { &mut out_left };
            let xd = if right { xmax + th * (3.0 + 2.2 * *n as f64) } else { xmin - th * (3.0 + 2.2 * *n as f64) };
            *n += 1;
            let xe = if right { x1 } else { x0 };
            let (top, bot) = (sw(Vec2::new(xe, y + r)), sw(Vec2::new(xe, y - r)));
            let rotation = if vertical { 0.0 } else { std::f64::consts::FRAC_PI_2 };
            p.dims.push(super::annotate::dim(s, DimKind::Linear { rotation }, sw(Vec2::new(xd, y + r)), top, bot, Vec2::ZERO, Vec2::ZERO, "%%c<>"));
            continue;
        };
        placed.push(x);
        let (top, bot) = (sw(Vec2::new(x, y + r)), sw(Vec2::new(x, y - r)));
        let rotation = if vertical { 0.0 } else { std::f64::consts::FRAC_PI_2 };
        p.dims.push(super::annotate::dim(s, DimKind::Linear { rotation }, top, top, bot, Vec2::ZERO, Vec2::ZERO, "%%c<>"));
    }
}

/// A disc's bore (an arc round its centre broken by a keyway) as a diameter, and the keyway's
/// width and depth.
fn disc_bore_and_key(s: &Session, p: &mut Plan, c: Vec2, arcs: &[&Feat], edges: &[&Feat], ctol: f64, gap: f64) {
    let Some(bore) = arcs.iter().filter(|a| a.c.dist(c) <= ctol && a.sweep > std::f64::consts::PI).max_by(|a, b| a.sweep.total_cmp(&b.sweep)) else {
        return;
    };
    // The bore's diameter, pointed at the middle of its arc (away from the keyway).
    let at = c + Vec2::from_angle(bore.c.angle_to(bore.a) + bore.sweep / 2.0) * bore.r;
    p.dims.push(super::annotate::dim(s, DimKind::Diameter, c * 2.0 - at, Vec2::ZERO, Vec2::ZERO, at, Vec2::ZERO, ""));
    // The keyway: lines next to the bore's gap.
    let reach = bore.r * 1.8;
    let key: Vec<&&Feat> = edges.iter().filter(|l| l.a.dist(c) < reach && l.b.dist(c) < reach).collect();
    let gap_dir = -(at - c).normalized();
    if gap_dir.y.abs() >= gap_dir.x.abs() {
        let xs = uniq(key.iter().filter(|l| same(l.a.x, l.b.x, ctol)).map(|l| l.a.x).collect(), ctol);
        let floor =
            key.iter().filter(|l| same(l.a.y, l.b.y, ctol)).map(|l| l.a.y).max_by(|a, b| ((a - c.y) * gap_dir.y).total_cmp(&((b - c.y) * gap_dir.y)));
        if let (Some(x0), Some(x1), Some(fy)) = (xs.first().copied(), xs.last().copied(), floor) {
            if x1 - x0 > ctol {
                let y = fy + gap_dir.y.signum() * gap;
                p.dims.push(super::annotate::dim(
                    s,
                    DimKind::Linear { rotation: 0.0 },
                    Vec2::new(x0, y),
                    Vec2::new(x0, fy),
                    Vec2::new(x1, fy),
                    Vec2::ZERO,
                    Vec2::ZERO,
                    "",
                ));
            }
            let far = Vec2::new(c.x, c.y - gap_dir.y.signum() * bore.r);
            let x = x1.max(c.x + bore.r) + gap;
            p.dims.push(super::annotate::dim(
                s,
                DimKind::Linear { rotation: std::f64::consts::FRAC_PI_2 },
                Vec2::new(x, fy),
                Vec2::new(x1, fy),
                far,
                Vec2::ZERO,
                Vec2::ZERO,
                "",
            ));
        }
    } else {
        let ys = uniq(key.iter().filter(|l| same(l.a.y, l.b.y, ctol)).map(|l| l.a.y).collect(), ctol);
        let floor =
            key.iter().filter(|l| same(l.a.x, l.b.x, ctol)).map(|l| l.a.x).max_by(|a, b| ((a - c.x) * gap_dir.x).total_cmp(&((b - c.x) * gap_dir.x)));
        if let (Some(y0), Some(y1), Some(fx)) = (ys.first().copied(), ys.last().copied(), floor) {
            if y1 - y0 > ctol {
                let x = fx + gap_dir.x.signum() * gap;
                p.dims.push(super::annotate::dim(
                    s,
                    DimKind::Linear { rotation: std::f64::consts::FRAC_PI_2 },
                    Vec2::new(x, y0),
                    Vec2::new(fx, y0),
                    Vec2::new(fx, y1),
                    Vec2::ZERO,
                    Vec2::ZERO,
                    "",
                ));
            }
            let far = Vec2::new(c.x - gap_dir.x.signum() * bore.r, c.y);
            let y = y1.max(c.y + bore.r) + gap;
            p.dims.push(super::annotate::dim(
                s,
                DimKind::Linear { rotation: 0.0 },
                Vec2::new(fx, y),
                Vec2::new(fx, y1),
                far,
                Vec2::ZERO,
                Vec2::ZERO,
                "",
            ));
        }
    }
}

/// A layout drawing (a line of machines, 1:100): the length of each section along the line's
/// main beam, and the overall length, above the view. The beam is the line along the view with
/// the most length drawn on it; the sections are the long lines on it, short pieces (under a
/// twenty-fifth of the length) going with their neighbour.
fn layout_plan(s: &Session, outline: &[&Feat], b: &Bounds2, tol: f64, ctol: f64, th: f64) -> Plan {
    let mut p = Plan::default();
    let vertical = b.height() > b.width();
    let sw = |v: Vec2| if vertical { Vec2::new(v.y, v.x) } else { v };
    let (lo, hi) = (sw(b.min).min(sw(b.max)), sw(b.min).max(sw(b.max)));
    let len = hi.x - lo.x;
    if len <= 0.0 {
        return p;
    }
    let along: Vec<(f64, f64, f64)> = outline
        .iter()
        .filter(|f| f.kind == Kind::Line)
        .map(|f| (sw(f.a), sw(f.b)))
        .filter(|(a, c)| same(a.y, c.y, tol) && (a.x - c.x).abs() > tol)
        .map(|(a, c)| (a.y, a.x.min(c.x), a.x.max(c.x)))
        .collect();
    // The beam: the level with the most length on it.
    let mut best: Option<(f64, f64)> = None;
    for (y, _, _) in along.iter().take(20_000) {
        let total: f64 = along.iter().filter(|l| same(l.0, *y, ctol)).map(|l| l.2 - l.1).sum();
        if best.is_none_or(|(_, t)| total > t) {
            best = Some((*y, total));
        }
    }
    let Some((beam, _)) = best else { return p };
    // Which way the beam's body lies (its sections' ends run down from it, or up): a bar
    // lying on the beam has its ends the other way and is no section.
    let ends: Vec<(f64, f64)> = outline
        .iter()
        .filter(|f| f.kind == Kind::Line)
        .map(|f| (sw(f.a), sw(f.b)))
        .filter(|(a, c)| same(a.x, c.x, tol) && (a.y - c.y).abs() > tol)
        .filter_map(|(a, c)| {
            if same(a.y, beam, ctol) {
                Some((a.x, c.y - beam))
            } else if same(c.y, beam, ctol) {
                Some((a.x, a.y - beam))
            } else {
                None
            }
        })
        .collect();
    let side = |x: f64| -> f64 { ends.iter().filter(|e| same(e.0, x, ctol)).map(|e| e.1.signum()).sum::<f64>().signum() };
    let long: Vec<&(f64, f64, f64)> = along.iter().filter(|(y, x0, x1)| same(*y, beam, ctol) && x1 - x0 >= 0.1 * len).collect();
    let body: f64 = long.iter().map(|(_, x0, x1)| (x1 - x0) * (side(*x0) + side(*x1))).sum::<f64>().signum();
    let mut xs: Vec<f64> = vec![lo.x, hi.x];
    for (_, x0, x1) in long {
        if body == 0.0 || side(*x0) == body || side(*x1) == body {
            xs.extend([*x0, *x1]);
        }
    }
    let mut xs = uniq(xs, ctol);
    // Short pieces join their neighbour (an end plate, a guard between two sections).
    for _ in 0..xs.len() {
        let short = xs
            .windows(2)
            .enumerate()
            .filter_map(|(i, w)| Some((i, w.get(1)? - w.first()?)))
            .filter(|(_, d)| *d < len / 25.0)
            .min_by(|a, c| a.1.total_cmp(&c.1));
        let Some((i, _)) = short else { break };
        // Drop the inner end of the short piece (never the overall ends).
        let k = if i + 1 == xs.len() - 1 { i } else { i + 1 };
        if k == 0 || k + 1 >= xs.len() {
            break;
        }
        xs.remove(k);
    }
    let gap0 = th * 3.0;
    let step = th * 2.2;
    let (rotation, edge) = if vertical { (std::f64::consts::FRAC_PI_2, hi.y) } else { (0.0, hi.y) };
    let mut rows: Vec<Vec<f64>> = Vec::new();
    if xs.len() > 2 {
        rows.push(xs);
    }
    rows.push(vec![lo.x, hi.x]);
    for (k, row) in rows.iter().enumerate() {
        let at = edge + gap0 + step * k as f64;
        for w in row.windows(2) {
            let (Some(a), Some(c)) = (w.first(), w.get(1)) else { continue };
            p.dims.push(super::annotate::dim(
                s,
                DimKind::Linear { rotation },
                sw(Vec2::new(*a, at)),
                sw(Vec2::new(*a, beam)),
                sw(Vec2::new(*c, beam)),
                Vec2::ZERO,
                Vec2::ZERO,
                "",
            ));
        }
    }
    p
}

/// The filled regions (solids, solid hatches) among `ents`: weld beads on a fabrication drawing.
fn weld_polygons(ents: &[Entity]) -> Vec<Vec<Vec2>> {
    let mut out = Vec::new();
    for e in ents {
        match &e.kind {
            EntityKind::Solid(so) => {
                let mut v: Vec<Vec2> = Vec::new();
                for c in so.corners.iter().map(|c| c.xy()) {
                    if !v.iter().any(|q| q.near(c, 1e-9)) {
                        v.push(c);
                    }
                }
                // SOLID's corner order is 1-2-4-3.
                if v.len() == 4 {
                    v.swap(2, 3);
                }
                out.push(v);
            }
            EntityKind::Hatch(h) if h.solid => {
                for lp in h.loops.iter().take(64) {
                    let v: Vec<Vec2> = lp.vertices.iter().take(256).map(|v| v.p).collect();
                    if v.len() >= 3 {
                        out.push(v);
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// Lines and arcs drawn round a weld bead (along its filled region's edges) are the weld's own,
/// not the part's: they take no dimension.
fn drop_weld_outlines(feats: &mut Vec<Feat>, welds: &[Vec<Vec2>]) {
    if welds.is_empty() {
        return;
    }
    let on_boundary = |q: Vec2, w: &[Vec2]| -> bool {
        let size = Bounds2::from_points(w.iter().copied());
        let tol = (size.width().max(size.height()) * 0.02).max(1e-6);
        let n = w.len();
        (0..n).any(|i| match (w.get(i), w.get((i + 1) % n)) {
            (Some(a), Some(b)) => seg_dist(q, *a, *b) <= tol,
            _ => false,
        })
    };
    feats.retain(|f| {
        let pts = match f.kind {
            Kind::Line => vec![f.a, f.b, (f.a + f.b) * 0.5],
            _ => return true,
        };
        !welds.iter().any(|w| pts.iter().all(|q| on_boundary(*q, w)))
    });
}

/// Whether the ray from `q` along `d` meets the segment `a`-`b`.
fn ray_hits(q: Vec2, d: Vec2, a: Vec2, b: Vec2) -> bool {
    let cr = |u: Vec2, v: Vec2| u.x * v.y - u.y * v.x;
    let e = b - a;
    let den = cr(d, e);
    if den.abs() < 1e-12 {
        return false;
    }
    let t = cr(a - q, e) / den;
    let u = cr(a - q, d) / den;
    t > 0.0 && (0.0..=1.0).contains(&u)
}

/// Distance from `q` to the segment `a`-`b`.
fn seg_dist(q: Vec2, a: Vec2, b: Vec2) -> f64 {
    let ab = b - a;
    let l2 = ab.dot(ab);
    let t = if l2 > 0.0 { ((q - a).dot(ab) / l2).clamp(0.0, 1.0) } else { 0.0 };
    q.dist(a + ab * t)
}

/// A metric thread drawn as its tap drill circle inside its nominal circle: the nominal size
/// when `inner` is the tap drill (nominal less the coarse pitch) of `outer`.
fn thread_size(inner: f64, outer: f64) -> Option<f64> {
    const COARSE: [(f64, f64); 21] = [
        (2.0, 0.4),
        (2.5, 0.45),
        (3.0, 0.5),
        (4.0, 0.7),
        (5.0, 0.8),
        (6.0, 1.0),
        (8.0, 1.25),
        (10.0, 1.5),
        (12.0, 1.75),
        (14.0, 2.0),
        (16.0, 2.0),
        (20.0, 2.5),
        (24.0, 3.0),
        (30.0, 3.5),
        (36.0, 4.0),
        (42.0, 4.5),
        (48.0, 5.0),
        (56.0, 5.5),
        (64.0, 6.0),
        (72.0, 6.0),
        (80.0, 6.0),
    ];
    COARSE.iter().find(|(d, pch)| (outer - d).abs() < 0.05 && (inner - (d - pch)).abs() < (0.012 * d).max(0.3)).map(|(d, _)| *d)
}

/// The angle of `p` seen from `c`.
trait AngleFrom {
    fn sub_angle(self, c: Vec2) -> f64;
}
impl AngleFrom for Vec2 {
    fn sub_angle(self, c: Vec2) -> f64 {
        c.angle_to(self)
    }
}

/// Text height and scale of the current dimension style.
pub(super) fn style_height(s: &Session) -> Result<f64> {
    let d = s.doc()?;
    // With the drawing's overrides (DIMSCALE set for this sheet), as the dimensions are drawn.
    let st = d.dim_style(&d.header.str("DIMSTYLE", "Standard")).cloned().unwrap_or_default().with_overrides(&d.dim_overrides());
    let k = st.effective_scale(d.header.f64("DIMSCALE", 1.0));
    let th = st.text_height * k;
    Ok(if th.is_finite() && th > 0.0 { th } else { 2.5 })
}

/// `window`: [[x, y], [x, y]], the area of the part to dimension.
fn window_param(p: &Value) -> Option<Bounds2> {
    let w = p.get("window")?.as_array()?;
    let pt = |v: &Value| -> Option<Vec2> {
        let a = v.as_array()?;
        let q = Vec2::new(a.first()?.as_f64()?, a.get(1)?.as_f64()?);
        q.is_finite().then_some(q)
    };
    let (a, b) = (pt(w.first()?)?, pt(w.get(1)?)?);
    Some(Bounds2::new(Vec2::new(a.x.min(b.x), a.y.min(b.y)), Vec2::new(a.x.max(b.x), a.y.max(b.y))))
}

/// The drawing objects lying wholly inside `w` (a window selection): lines running on past
/// it (a shaft through the plate, a neighbouring part) are left out. Dimensions, text and
/// leaders already there are not geometry.
pub(super) fn in_window(s: &Session, w: &Bounds2, among: Option<&[Handle]>) -> Result<Vec<Handle>> {
    let d = s.doc()?;
    let win = w.expand((w.width().max(w.height()) * 1e-9).max(1e-9));
    let mut out = Vec::new();
    for e in d.model.iter() {
        if matches!(e.kind, EntityKind::Dimension(_) | EntityKind::Text(_) | EntityKind::MText(_) | EntityKind::Leader(_) | EntityKind::MLeader(_)) {
            continue;
        }
        if among.is_some_and(|a| !a.contains(&e.handle)) {
            continue;
        }
        let b = cadcraft_doc::entity_bounds(d, e, 0);
        if !b.is_empty() && win.contains_box(&b) {
            out.push(e.handle);
        }
    }
    Ok(out)
}

/// The dimension style AUTODIM uses: the one asked for, else the one the drawing's dimensions
/// mostly use (a sheet drawn at 1:5 has its style for 1:5), else none (the current one).
fn choose_style(s: &Session, p: &Value) -> Result<Option<String>> {
    Ok(match str_param(p, "style") {
        Some(st) => Some(s.doc()?.dim_style(st).map(|d| d.name.clone()).ok_or_else(|| bad("autodim", format!("no dimension style \"{st}\"")))?),
        None => {
            let d = s.doc()?;
            let mut count: BTreeMap<String, usize> = BTreeMap::new();
            for e in d.model.iter() {
                if let EntityKind::Dimension(dm) = &e.kind {
                    *count.entry(dm.style.clone()).or_default() += 1;
                }
            }
            count.into_iter().max_by_key(|(_, n)| *n).map(|(k, _)| k).filter(|k| d.dim_style(k).is_some())
        }
    })
}

/// What AUTODIM would add for objects `hs`, in the current dimension style.
fn plan_for(s: &Session, hs: &[Handle], p: &Value) -> Result<Plan> {
    let th = style_height(s)?;
    let feats = {
        let d = s.doc()?;
        let ents: Vec<Entity> = hs.iter().filter_map(|h| d.entity(*h)).map(|e| (**e).clone()).collect();
        let mut f = Vec::new();
        collect(d, &ents, &mut f, 0, None);
        drop_weld_outlines(&mut f, &weld_polygons(&ents));
        f
    };
    let pitch_min = p.get("pitchMin").and_then(Value::as_u64).map_or(4, |v| v.clamp(3, 1000) as usize);
    let thickness = p.get("thickness").and_then(Value::as_f64);
    let layout = p.get("layout").and_then(Value::as_bool);
    let o = Opts { pitch_min, callouts: bool_or(p, "callouts", true), thickness, th, layout };
    plan(s, &feats, &o)
}

fn run_autodim(s: &mut Session, p: &Value) -> Result<Value> {
    let hs = match window_param(p) {
        Some(w) => {
            let given = if p.get("handles").is_some() || p.get("handle").is_some() { Some(targets(s, p)?) } else { None };
            in_window(s, &w, given.as_deref())?
        }
        None => targets(s, p)?,
    };
    if hs.is_empty() {
        return Err(bad("autodim", "select the part to dimension (its outline, holes and slots)"));
    }
    if let Some(name) = choose_style(s, p)? {
        s.doc_mut()?.header.set_str("DIMSTYLE", &name);
    }
    let plan = plan_for(s, &hs, p)?;
    // Dimensions and notes go on the drawing's dimension layer (the company's "2-DIM") when it
    // has one; the current layer is put back after.
    let old_layer = s.doc()?.header.str("CLAYER", "0");
    let dim_layer = s.doc()?.layers.iter().map(|l| l.name.clone()).find(|n| n.eq_ignore_ascii_case("2-DIM"));
    if let Some(l) = &dim_layer {
        s.doc_mut()?.header.set_str("CLAYER", l);
    }
    let placed = place_plan(s, plan);
    if dim_layer.is_some() {
        s.doc_mut()?.header.set_str("CLAYER", &old_layer);
    }
    let (out, ndims, nnotes) = placed?;
    s.echo(format!("AUTODIM: {ndims} dimension(s), {nnotes} note(s)."));
    let mut res = json!({ "handles": out, "dimensions": ndims, "notes": nnotes });
    if bool_or(p, "check", true) {
        // The part's own geometry and what was just added: anything still loose is reported.
        let mut all: Vec<Value> = hs.iter().map(|h| json!(h.hex())).collect();
        all.extend(res.get("handles").and_then(Value::as_array).cloned().unwrap_or_default());
        let chk = super::autodim_check::run_dimcheck(s, &json!({ "handles": all, "mark": false, "quiet": true }))?;
        if let Some(n) = chk.get("issues").and_then(Value::as_array).map(Vec::len) {
            if n == 0 {
                s.echo("DIMCHECK: every position is dimensioned.");
            } else {
                s.echo(format!("DIMCHECK: {n} position(s) still need a dimension; run DIMCHECK to mark them."));
            }
        }
        if let Some(m) = res.as_object_mut() {
            m.insert("check".into(), chk);
        }
    }
    Ok(res)
}

// ------------------------------------------------------------------ prompts

/// AUTODIM at the command line: a rectangle round the part (only what lies wholly inside it
/// counts), shown with the dimensions it gives; clicking moves the nearest corner of the
/// rectangle to adjust it, a number is the plate thickness, Enter places the dimensions.
/// Objects selected beforehand are used as they are.
#[derive(Default)]
struct AutoDimM {
    first: Option<Vec2>,
    win: Option<Bounds2>,
    objs: Vec<Handle>,
    preselected: bool,
    thickness: Option<f64>,
    shown: Vec<EntityKind>,
    /// The drawing's DIMSTYLE before AUTODIM chose one, put back on cancel.
    old_style: Option<String>,
}

fn rect_lines(b: &Bounds2) -> Vec<EntityKind> {
    let c = [b.min, Vec2::new(b.max.x, b.min.y), b.max, Vec2::new(b.min.x, b.max.y)];
    (0..4)
        .filter_map(|i| {
            let (a, z) = (c.get(i)?, c.get((i + 1) % 4)?);
            Some(EntityKind::Line(cadcraft_doc::Line { a: cadcraft_geom::Vec3::new(a.x, a.y, 0.0), b: cadcraft_geom::Vec3::new(z.x, z.y, 0.0) }))
        })
        .collect()
}

impl AutoDimM {
    fn params(&self) -> Value {
        let mut p = json!({ "handles": self.objs.iter().map(|h| h.hex()).collect::<Vec<_>>() });
        if let (Some(t), Some(m)) = (self.thickness, p.as_object_mut()) {
            m.insert("thickness".into(), json!(t));
        }
        p
    }
    /// Recomputes what lies in the rectangle and the dimensions it gives.
    fn refresh(&mut self, s: &Session) -> Result<()> {
        if let Some(w) = self.win {
            self.objs = in_window(s, &w, None)?;
        }
        self.shown.clear();
        if self.objs.is_empty() {
            return Ok(());
        }
        if let Ok(plan) = plan_for(s, &self.objs, &self.params()) {
            self.shown.extend(plan.dims);
            for (pts, _) in plan.notes {
                for w in pts.windows(2) {
                    if let (Some(a), Some(b)) = (w.first(), w.get(1)) {
                        self.shown.push(EntityKind::Line(cadcraft_doc::Line {
                            a: cadcraft_geom::Vec3::new(a.x, a.y, 0.0),
                            b: cadcraft_geom::Vec3::new(b.x, b.y, 0.0),
                        }));
                    }
                }
            }
        }
        Ok(())
    }
    fn set_style(&mut self, s: &mut Session) -> Result<()> {
        if let Some(name) = choose_style(s, &Value::Null)? {
            let d = s.doc_mut()?;
            self.old_style = Some(d.header.str("DIMSTYLE", "Standard"));
            d.header.set_str("DIMSTYLE", &name);
        }
        Ok(())
    }
    fn finish(&mut self, s: &mut Session) -> Result<Step> {
        if self.objs.is_empty() {
            return Err(crate::EngineError::Other("Nothing lies wholly inside the rectangle; draw it round the part.".into()));
        }
        run_autodim(s, &self.params())?;
        Ok(Step::Done)
    }
}

impl Interactive for AutoDimM {
    fn name(&self) -> &'static str {
        "AUTODIM"
    }
    fn begin(&mut self, s: &mut Session) -> Result<Step> {
        self.set_style(s)?;
        let sel = s.selection();
        if !sel.is_empty() {
            self.objs = sel;
            self.preselected = true;
            self.refresh(s)?;
        }
        Ok(Step::Continue)
    }
    fn prompt(&self, _s: &Session) -> Prompt {
        if self.preselected || self.win.is_some() {
            let msg = if self.preselected {
                "Plate thickness t= (Enter to place the dimensions)"
            } else {
                "Click to move the nearest corner of the rectangle, t= thickness, or Enter to place"
            };
            let accept = Accept { point: !self.preselected, number: true, text: true, select: false, enter: true };
            let p = Prompt::new(msg, accept);
            if self.preselected { p } else { p.kw(&["Window"]) }
        } else if let Some(a) = self.first {
            Prompt::new("Specify opposite corner", Accept::POINT).base_opt(Some(a))
        } else {
            Prompt::new("Specify first corner of a rectangle round the part", Accept::POINT)
        }
    }
    fn preview(&self, _s: &Session, c: Vec2) -> Vec<EntityKind> {
        if let (Some(a), None) = (self.first, self.win) {
            return rect_lines(&Bounds2::new(Vec2::new(a.x.min(c.x), a.y.min(c.y)), Vec2::new(a.x.max(c.x), a.y.max(c.y))));
        }
        let mut v = self.win.map(|w| rect_lines(&w)).unwrap_or_default();
        v.extend(self.shown.iter().cloned());
        v
    }
    fn input(&mut self, s: &mut Session, i: Input) -> Result<Step> {
        if matches!(i, Input::Cancel) {
            if let Some(old) = self.old_style.take() {
                s.doc_mut()?.header.set_str("DIMSTYLE", &old);
            }
            return Ok(Step::Done);
        }
        if !self.preselected && self.win.is_none() {
            let Input::Point(q) = i else {
                return if matches!(i, Input::Enter) { Ok(Step::Done) } else { Err(crate::EngineError::Other("Specify a point.".into())) };
            };
            match self.first {
                None => self.first = Some(q),
                Some(a) => {
                    let w = Bounds2::new(Vec2::new(a.x.min(q.x), a.y.min(q.y)), Vec2::new(a.x.max(q.x), a.y.max(q.y)));
                    if w.width() <= 0.0 || w.height() <= 0.0 {
                        return Err(crate::EngineError::Other("The rectangle has no area.".into()));
                    }
                    self.win = Some(w);
                    self.refresh(s)?;
                }
            }
            return Ok(Step::Continue);
        }
        match i {
            Input::Enter => self.finish(s),
            Input::Point(q) if !self.preselected => {
                // The nearest corner goes to the click; the opposite one stays.
                if let Some(w) = self.win {
                    let corners = [w.min, Vec2::new(w.max.x, w.min.y), w.max, Vec2::new(w.min.x, w.max.y)];
                    let k = (0..4).min_by(|x, y| {
                        let d = |i: &usize| corners.get(*i).map_or(f64::INFINITY, |c| c.dist(q));
                        d(x).total_cmp(&d(y))
                    });
                    if let Some(opp) = k.and_then(|k| corners.get((k + 2) % 4)) {
                        let nw = Bounds2::new(Vec2::new(opp.x.min(q.x), opp.y.min(q.y)), Vec2::new(opp.x.max(q.x), opp.y.max(q.y)));
                        if nw.width() > 0.0 && nw.height() > 0.0 {
                            self.win = Some(nw);
                            self.refresh(s)?;
                        }
                    }
                }
                Ok(Step::Continue)
            }
            Input::Keyword(k) | Input::Text(k) if k.trim().eq_ignore_ascii_case("window") || k.trim().eq_ignore_ascii_case("w") => {
                self.first = None;
                self.win = None;
                self.shown.clear();
                Ok(Step::Continue)
            }
            Input::Text(t) | Input::Keyword(t) => {
                let t = t.trim().trim_start_matches(['t', 'T', '=']).trim();
                match t.parse::<f64>() {
                    Ok(v) if v > 0.0 && v.is_finite() => {
                        self.thickness = Some(v);
                        self.refresh(s)?;
                        if self.preselected { self.finish(s) } else { Ok(Step::Continue) }
                    }
                    _ if t.is_empty() => self.finish(s),
                    _ => Err(crate::EngineError::Other("Requires a positive number.".into())),
                }
            }
            _ => Ok(Step::Continue),
        }
    }
}

#[derive(Default)]
struct DimCheckM {
    sel: SelectPhase,
}

impl Interactive for DimCheckM {
    fn name(&self) -> &'static str {
        "DIMCHECK"
    }
    fn begin(&mut self, s: &mut Session) -> Result<Step> {
        self.sel = SelectPhase::begin(s);
        if self.sel.done {
            let hs = self.sel.picked.clone();
            super::autodim_check::run_dimcheck(s, &json!({ "handles": hs.iter().map(|h| h.hex()).collect::<Vec<_>>() }))?;
            return Ok(Step::Done);
        }
        Ok(Step::Continue)
    }
    fn prompt(&self, _s: &Session) -> Prompt {
        let mut p = self.sel.prompt();
        p.message = format!("{} (Enter: the whole drawing)", p.message);
        p
    }
    fn input(&mut self, s: &mut Session, i: Input) -> Result<Step> {
        if matches!(i, Input::Cancel) {
            return Ok(Step::Done);
        }
        match self.sel.feed(s, &i)? {
            SelOutcome::More => Ok(Step::Continue),
            SelOutcome::Empty => {
                super::autodim_check::run_dimcheck(s, &json!({ "handles": [] }))?;
                Ok(Step::Done)
            }
            SelOutcome::Done(hs) => {
                super::autodim_check::run_dimcheck(s, &json!({ "handles": hs.iter().map(|h| h.hex()).collect::<Vec<_>>() }))?;
                Ok(Step::Done)
            }
        }
    }
}

/// Adds a plan's dimensions, leader notes and labels; their handles and counts.
fn place_plan(s: &mut Session, plan: Plan) -> Result<(Vec<String>, usize, usize)> {
    let mut out: Vec<String> = Vec::new();
    let ndims = plan.dims.len();
    for k in plan.dims {
        let h = s.add_entity(k)?;
        s.last_dim = Some(h);
        out.push(h.hex());
    }
    let nnotes = plan.notes.len();
    for (pts, text) in plan.notes {
        let r = super::annotate::run_leader(
            s,
            &json!({ "points": pts.iter().map(|q| [q.x, q.y]).collect::<Vec<_>>(), "text": text, "underline": true }),
        )?;
        if let Some(a) = r.get("handles").and_then(Value::as_array) {
            out.extend(a.iter().filter_map(Value::as_str).map(str::to_string));
        } else if let Some(h) = r.get("handle").and_then(Value::as_str) {
            out.push(h.to_string());
        }
    }
    let th = style_height(s)?;
    for (at, text) in plan.labels {
        let k = super::draw::make_text_to(s, at, None, &text, th, 0.0, (cadcraft_doc::HAlign::Center, cadcraft_doc::VAlign::Middle));
        out.push(s.add_entity(k)?.hex());
    }
    Ok((out, ndims, nnotes))
}
