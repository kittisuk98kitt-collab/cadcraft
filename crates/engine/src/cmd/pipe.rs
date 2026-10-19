//! Piping in plan (double line): PIPE draws a run through picked points and fits the fittings
//! itself: long-radius elbows at the bends, flanges at the ends, and a tee where a run starts on
//! another run. Pipe sizes: SGP (JIS G3452), SCH40 and SCH60 (JIS G3454), 15A–500A; elbows and
//! tees to butt-welding fitting dimensions; flanges JIS B 2220 10K slip-on.
//!
//! Each run is a group (`PIPE_RUN_<n>`) whose description keeps the run's specification, so
//! PIPEEDIT can change its size, schedule or flanges and draw it again; picking any part of a
//! run picks the run (group selection).

use cadcraft_doc::{EntityKind, Group, Handle, Lineweight};
use cadcraft_geom::{Arc, Vec2};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::helpers::{arc, line};
use super::*;
use crate::{Accept, Input, Interactive, Prompt, Result, Session, Step};

/// One nominal size: OD, wall thickness SGP / SCH40 / SCH60, 90° long elbow radius (centre to
/// face), tee centre to end, and the 10K flange (outside diameter, thickness, bolt circle,
/// bolts, bolt hole).
#[derive(Clone, Copy, Debug)]
pub struct Size {
    pub name: &'static str,
    pub od: f64,
    pub sgp: f64,
    pub sch40: f64,
    pub sch60: f64,
    pub elbow: f64,
    pub tee: f64,
    pub flange: (f64, f64, f64, u32, f64),
}

const fn z(name: &'static str, od: f64, sgp: f64, sch40: f64, sch60: f64, elbow: f64, tee: f64, flange: (f64, f64, f64, u32, f64)) -> Size {
    Size { name, od, sgp, sch40, sch60, elbow, tee, flange }
}

pub const SIZES: &[Size] = &[
    z("15A", 21.7, 2.8, 2.8, 3.2, 38.1, 25.4, (95.0, 12.0, 70.0, 4, 15.0)),
    z("20A", 27.2, 2.8, 2.9, 3.4, 38.1, 28.6, (100.0, 14.0, 75.0, 4, 15.0)),
    z("25A", 34.0, 3.2, 3.4, 3.9, 38.1, 38.1, (125.0, 14.0, 90.0, 4, 19.0)),
    z("32A", 42.7, 3.5, 3.6, 4.5, 47.6, 47.6, (135.0, 16.0, 100.0, 4, 19.0)),
    z("40A", 48.6, 3.5, 3.7, 4.5, 57.2, 57.2, (140.0, 16.0, 105.0, 4, 19.0)),
    z("50A", 60.5, 3.8, 3.9, 4.9, 76.2, 63.5, (155.0, 16.0, 120.0, 4, 19.0)),
    z("65A", 76.3, 4.2, 5.2, 6.0, 95.3, 76.2, (175.0, 18.0, 140.0, 4, 19.0)),
    z("80A", 89.1, 4.2, 5.5, 6.6, 114.3, 85.7, (185.0, 18.0, 150.0, 8, 19.0)),
    z("90A", 101.6, 4.2, 5.7, 7.0, 133.4, 95.3, (195.0, 18.0, 160.0, 8, 19.0)),
    z("100A", 114.3, 4.5, 6.0, 7.1, 152.4, 104.8, (210.0, 18.0, 175.0, 8, 19.0)),
    z("125A", 139.8, 4.5, 6.6, 8.1, 190.5, 123.8, (250.0, 20.0, 210.0, 8, 23.0)),
    z("150A", 165.2, 5.0, 7.1, 9.3, 228.6, 142.9, (280.0, 22.0, 240.0, 8, 23.0)),
    z("200A", 216.3, 5.8, 8.2, 10.3, 304.8, 177.8, (330.0, 22.0, 290.0, 12, 23.0)),
    z("250A", 267.4, 6.6, 9.3, 12.7, 381.0, 215.9, (400.0, 24.0, 355.0, 12, 25.0)),
    z("300A", 318.5, 6.9, 10.3, 14.3, 457.2, 254.0, (445.0, 24.0, 400.0, 16, 25.0)),
    z("350A", 355.6, 7.9, 11.1, 15.1, 533.4, 279.4, (490.0, 26.0, 445.0, 16, 25.0)),
    z("400A", 406.4, 7.9, 12.7, 16.7, 609.6, 304.8, (560.0, 28.0, 510.0, 16, 27.0)),
    z("450A", 457.2, 7.9, 14.3, 19.0, 685.8, 342.9, (620.0, 30.0, 565.0, 20, 27.0)),
    z("500A", 508.0, 7.9, 15.1, 20.6, 762.0, 381.0, (675.0, 30.0, 620.0, 20, 27.0)),
];

pub const SCHEDULES: &[&str] = &["SGP", "SCH40", "SCH60"];

/// The most points a run takes.
const MAX_POINTS: usize = 500;
/// Group descriptions of pipe runs start with this.
const TAG: &str = "CADCRAFT_PIPE ";

/// Nominal sizes in inches (B sizes) beside the A sizes.
pub const INCHES: &[(&str, &str)] = &[
    ("15A", "1/2"),
    ("20A", "3/4"),
    ("25A", "1"),
    ("32A", "1-1/4"),
    ("40A", "1-1/2"),
    ("50A", "2"),
    ("65A", "2-1/2"),
    ("80A", "3"),
    ("90A", "3-1/2"),
    ("100A", "4"),
    ("125A", "5"),
    ("150A", "6"),
    ("200A", "8"),
    ("250A", "10"),
    ("300A", "12"),
    ("350A", "14"),
    ("400A", "16"),
    ("450A", "18"),
    ("500A", "20"),
];

/// The size in inches ("2", "1-1/2") of an A size.
pub fn inch(name: &str) -> &'static str {
    INCHES.iter().find(|(a, _)| *a == name).map_or("", |(_, i)| i)
}

/// A size by its A designation ("50A", "50") or in inches ("2\"", "2B", "1-1/2", "1 1/2\"").
pub fn size(name: &str) -> Option<Size> {
    let n = name.trim().to_ascii_uppercase();
    let n = n.split_once(' ').filter(|(_, r)| r.contains('/')).map_or(n.clone(), |(a, b)| format!("{a}-{b}"));
    let bare = n.trim_end_matches(['"', 'B', '″']).trim().to_string();
    let by_inch = INCHES.iter().find(|(_, i)| *i == bare).map(|(a, _)| (*a).to_string());
    let a = match by_inch {
        Some(a) if n.ends_with(['"', 'B', '″']) || bare.contains('/') || !SIZES.iter().any(|s| s.name == format!("{bare}A")) => a,
        _ => {
            if n.ends_with('A') {
                n
            } else {
                format!("{n}A")
            }
        }
    };
    SIZES.iter().find(|s| s.name == a).copied()
}

/// A tee on a run: its centre on the run and the side its branch leaves to (+1 left of the run
/// direction, −1 right).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Tee {
    pub at: [f64; 2],
    pub side: f64,
}

/// A run: what PIPE drew and PIPEEDIT changes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RunSpec {
    pub schedule: String,
    pub size: String,
    pub points: Vec<[f64; 2]>,
    pub flange_start: bool,
    pub flange_end: bool,
    /// The run starts at a tee on another run (its first point is the tee centre).
    pub from_tee: bool,
    pub tees: Vec<Tee>,
    pub label: bool,
}

impl Default for RunSpec {
    fn default() -> Self {
        RunSpec {
            schedule: "SGP".into(),
            size: "50A".into(),
            points: Vec::new(),
            flange_start: false,
            flange_end: false,
            from_tee: false,
            tees: Vec::new(),
            label: true,
        }
    }
}

/// PIPE's settings (the Pipe dialog).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PipeSettings {
    pub schedule: String,
    pub size: String,
    pub flange_start: bool,
    pub flange_end: bool,
    pub label: bool,
}

impl Default for PipeSettings {
    fn default() -> Self {
        PipeSettings { schedule: "SGP".into(), size: "50A".into(), flange_start: false, flange_end: false, label: true }
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("pipe", "Pipe", run_pipe)
            .menu(&["Draw", "Pipe..."])
            .alias(&["pp"])
            .params("{points: [[x,y]...], schedule?: SGP|SCH40|SCH60, size?: \"50A\", flangeStart?, flangeEnd?, label?} (a run whose first point lies on another run starts at a tee)")
            .interactive(|_| Ok(Box::new(PipeM::default()))),
        CommandSpec::new("pipeedit", "Edit Pipe", run_pipeedit)
            .menu(&["Modify", "Object", "Pipe..."])
            .params("{handle (any part of a run), schedule?, size?, flangeStart?, flangeEnd?, label?, points?}")
            .interactive(|_| Ok(Box::new(PipeEditM))),
        CommandSpec::new("pipe.settings", "Pipe Settings", run_settings)
            .params("{schedule?, size?, flangeStart?, flangeEnd?, label?} → the settings, sizes and schedules")
            .noundo(),
        CommandSpec::new("pipe.run", "Pipe Run", run_info).params("{handle} → the run's specification").noundo(),
    ]
}

fn check_spec(sch: &str, sz: &str) -> Result<(String, Size)> {
    let sch = sch.trim().to_ascii_uppercase().replace(' ', "");
    let sch = match sch.as_str() {
        "SGP" => "SGP",
        "SCH40" | "40" | "STPG40" => "SCH40",
        "SCH60" | "60" | "STPG60" => "SCH60",
        _ => return Err(bad("pipe", "`schedule` is SGP, SCH40 or SCH60")),
    };
    let size = size(sz).ok_or_else(|| bad("pipe", format!("no pipe size \"{sz}\" (15A to 500A)")))?;
    Ok((sch.to_string(), size))
}

fn merge<T: Serialize + for<'de> Deserialize<'de>>(cur: &T, p: &Value) -> Result<T> {
    let mut v = serde_json::to_value(cur).map_err(|e| bad("pipe", e.to_string()))?;
    if let (Some(c), Some(o)) = (v.as_object_mut(), p.as_object()) {
        for (k, val) in o {
            if c.contains_key(k) {
                c.insert(k.clone(), val.clone());
            }
        }
    }
    serde_json::from_value(v).map_err(|e| bad("pipe", e.to_string()))
}

fn run_settings(s: &mut Session, p: &Value) -> Result<Value> {
    let mut next: PipeSettings = merge(&s.pipe, p)?;
    let (sch, sz) = check_spec(&next.schedule, &next.size)?;
    next.schedule = sch;
    next.size = sz.name.into();
    s.pipe = next;
    let mut v = serde_json::to_value(&s.pipe).map_err(|e| bad("pipe", e.to_string()))?;
    v["sizes"] =
        json!(SIZES.iter().map(|z| json!({ "name": z.name, "od": z.od, "sgp": z.sgp, "sch40": z.sch40, "sch60": z.sch60 })).collect::<Vec<_>>());
    v["schedules"] = json!(SCHEDULES);
    Ok(v)
}

/// A piece of drawing: kind, thin line, on the CENTER linetype.
type Piece = (EntityKind, bool, bool);

fn v2(p: [f64; 2]) -> Vec2 {
    Vec2::new(p[0], p[1])
}

fn perp(u: Vec2) -> Vec2 {
    Vec2::new(-u.y, u.x)
}

/// A straight pipe between a and b: two outlines and the centre line.
fn straight(out: &mut Vec<Piece>, a: Vec2, b: Vec2, r: f64) {
    if a.dist(b) < 1e-9 {
        return;
    }
    let n = perp((b - a).normalized()) * r;
    out.push((line(a + n, b + n), false, false));
    out.push((line(a - n, b - n), false, false));
    out.push((line(a, b), true, true));
}

/// A joint (weld line) across the pipe at p, direction u.
fn joint(out: &mut Vec<Piece>, p: Vec2, u: Vec2, r: f64) {
    let n = perp(u) * r;
    out.push((line(p + n, p - n), true, false));
}

/// The geometry of a run, and where its tees' branches begin (for checks).
fn geometry(spec: &RunSpec, sz: &Size) -> std::result::Result<Vec<Piece>, String> {
    let pts: Vec<Vec2> = spec.points.iter().map(|p| v2(*p)).collect();
    if pts.len() < 2 {
        return Err("a run needs two points".into());
    }
    if pts.windows(2).any(|w| w[0].dist(w[1]) < 1e-9) {
        return Err("two points of the run are the same".into());
    }
    let r = sz.od / 2.0;
    let rad = sz.elbow;
    let n = pts.len();
    // Tangent length of each bend.
    let mut tl = vec![0.0; n];
    let mut out: Vec<Piece> = Vec::new();
    for i in 1..n - 1 {
        let u = (pts[i] - pts[i - 1]).normalized();
        let v = (pts[i + 1] - pts[i]).normalized();
        let turn = u.cross(v).atan2(u.dot(v));
        if turn.abs() < 1e-6 {
            continue;
        }
        if turn.abs() > 179f64.to_radians() {
            return Err("a run cannot turn back on itself".into());
        }
        tl[i] = rad * (turn.abs() / 2.0).tan();
        let t0 = pts[i] - u * tl[i];
        let t1 = pts[i] + v * tl[i];
        // Arc centre on the inside of the bend.
        let side = turn.signum();
        let c = t0 + perp(u) * (rad * side);
        let (a0, a1) = ((t0 - c).angle(), (t1 - c).angle());
        let (s, e) = if side > 0.0 { (a0, a1) } else { (a1, a0) };
        for (rr, thin, center) in [(rad - r, false, false), (rad + r, false, false), (rad, true, true)] {
            out.push((arc(&Arc::new(c, rr, s, e)), thin, center));
        }
        joint(&mut out, t0, u, r);
        joint(&mut out, t1, v, r);
    }
    let (ft, fd) = (sz.flange.1, sz.flange.0 / 2.0);
    // Segment ends after the fittings.
    for i in 0..n - 1 {
        let u = (pts[i + 1] - pts[i]).normalized();
        let len = pts[i].dist(pts[i + 1]);
        let mut a_off = tl[i];
        let mut b_off = tl[i + 1];
        if i == 0 && spec.from_tee {
            a_off = sz.tee;
        }
        if i == 0 && spec.flange_start {
            a_off = ft;
        }
        if i == n - 2 && spec.flange_end {
            b_off = ft;
        }
        if a_off + b_off > len + 1e-9 {
            return Err(format!("segment {} is too short for its fittings ({:.1} needed)", i + 1, a_off + b_off));
        }
        // Tees on this segment split it.
        let mut cuts: Vec<(f64, f64)> = Vec::new();
        for t in &spec.tees {
            let q = v2(t.at);
            let along = (q - pts[i]).dot(u);
            let off = (q - pts[i]).cross(u).abs();
            if off < 1e-6 && along > a_off + sz.tee - 1e-9 && along < len - b_off - sz.tee + 1e-9 {
                cuts.push((along - sz.tee, along + sz.tee));
                // The tee: the run straight through on the far side, the branch side opened for
                // the branch, the branch stub, and its three joints.
                let w = perp(u) * t.side.signum();
                let far = q - w * r;
                out.push((line(far - u * sz.tee, far + u * sz.tee), false, false));
                let near = q + w * r;
                out.push((line(near - u * sz.tee, near - u * r), false, false));
                out.push((line(near + u * r, near + u * sz.tee), false, false));
                out.push((line(q + w * r - u * r, q + w * sz.tee - u * r), false, false));
                out.push((line(q + w * r + u * r, q + w * sz.tee + u * r), false, false));
                out.push((line(q - u * sz.tee, q + u * sz.tee), true, true));
                out.push((line(q, q + w * sz.tee), true, true));
                joint(&mut out, q - u * sz.tee, u, r);
                joint(&mut out, q + u * sz.tee, u, r);
                joint(&mut out, q + w * sz.tee, w, r);
            }
        }
        cuts.sort_by(|x, y| x.0.total_cmp(&y.0));
        let mut from = a_off;
        for (c0, c1) in cuts {
            straight(&mut out, pts[i] + u * from, pts[i] + u * c0, r);
            from = c1;
        }
        straight(&mut out, pts[i] + u * from, pts[i] + u * (len - b_off), r);
    }
    // Flanges: the plate across the end, the pipe in it, the bolt circle as thin lines.
    let mut flange = |at: Vec2, u: Vec2| {
        let n = perp(u);
        let (a, b) = (at - u * ft, at);
        for (x, y) in [(a + n * fd, b + n * fd), (a - n * fd, b - n * fd), (a + n * fd, a - n * fd), (b + n * fd, b - n * fd)] {
            out.push((line(x, y), false, false));
        }
        let bc = sz.flange.2 / 2.0;
        out.push((line(a + n * bc, b + n * bc), true, true));
        out.push((line(a - n * bc, b - n * bc), true, true));
    };
    if spec.flange_start {
        flange(pts[0] + (pts[1] - pts[0]).normalized() * ft, (pts[1] - pts[0]).normalized());
    }
    if spec.flange_end {
        let u = (pts[n - 1] - pts[n - 2]).normalized();
        flange(pts[n - 1], u);
    }
    if spec.from_tee {
        // The branch's joint at the tee is drawn by the tee itself.
    }
    Ok(out)
}

/// The label along the longest segment.
fn label_text(spec: &RunSpec) -> String {
    match inch(&spec.size) {
        "" => format!("{} {}", spec.size, spec.schedule),
        i => format!("{} ({i}\") {}", spec.size, spec.schedule),
    }
}

fn add_run(s: &mut Session, spec: &RunSpec, group: Option<String>) -> Result<(String, Vec<Handle>)> {
    let (sch, sz) = check_spec(&spec.schedule, &spec.size)?;
    let mut spec = spec.clone();
    spec.schedule = sch;
    spec.size = sz.name.into();
    if spec.points.len() > MAX_POINTS {
        return Err(bad("pipe", format!("a run takes up to {MAX_POINTS} points")));
    }
    let pieces = geometry(&spec, &sz).map_err(|m| bad("pipe", m))?;
    {
        let d = s.doc_mut()?;
        if d.linetype("CENTER").is_none()
            && let Some(lt) = cadcraft_doc::library::standard_linetypes_for(d).into_iter().find(|l| l.name == "CENTER")
        {
            d.linetypes.push(lt);
        }
    }
    let mut hs = Vec::new();
    for (k, thin, center) in pieces {
        let h = s.add_entity(k)?;
        s.doc_mut()?
            .modify_entity(h, |e| {
                e.common.lineweight = Lineweight::Mm100(if thin { 18 } else { 35 });
                if center {
                    e.common.linetype = "CENTER".into();
                }
            })
            .map_err(|e| bad("pipe", e.to_string()))?;
        hs.push(h);
    }
    if spec.label {
        let pts: Vec<Vec2> = spec.points.iter().map(|p| v2(*p)).collect();
        if let Some(w) = pts.windows(2).max_by(|a, b| a[0].dist(a[1]).total_cmp(&b[0].dist(b[1]))) {
            let u = (w[1] - w[0]).normalized();
            let mut ang = u.angle();
            // Text reads left to right or bottom to top.
            if ang > std::f64::consts::FRAC_PI_2 + 1e-9 || ang <= -std::f64::consts::FRAC_PI_2 {
                ang += std::f64::consts::PI;
            }
            let up = Vec2::new(-ang.sin(), ang.cos());
            let height = (sz.od * 0.35).clamp(2.5, 60.0);
            let mid = (w[0] + w[1]) * 0.5 + up * (sz.od / 2.0 + height * 0.6);
            let t = EntityKind::MText(cadcraft_doc::MText {
                insert: super::helpers::v3(mid),
                height,
                width: 0.0,
                attach: 8,
                rotation: ang,
                style: s.doc().map(|d| d.header.str("TEXTSTYLE", "Standard")).unwrap_or_else(|_| "Standard".into()),
                contents: label_text(&spec),
                line_spacing: 1.0,
                line_spacing_exact: false,
                columns: None,
            });
            hs.push(s.add_entity(t)?);
        }
    }
    let d = s.doc_mut()?;
    let name = group.unwrap_or_else(|| {
        (1..).map(|i| format!("PIPE_RUN_{i}")).find(|n| !d.groups.iter().any(|g| &g.name == n)).unwrap_or_else(|| "PIPE_RUN".into())
    });
    let desc = format!("{TAG}{}", serde_json::to_string(&spec).map_err(|e| bad("pipe", e.to_string()))?);
    match d.groups.iter_mut().find(|g| g.name == name) {
        Some(g) => {
            g.members = hs.clone();
            g.description = desc;
        }
        None => d.groups.push(Group { name: name.clone(), description: desc, selectable: true, members: hs.clone() }),
    }
    Ok((name, hs))
}

/// The run (group name and spec) an object belongs to.
pub fn run_of(s: &Session, h: Handle) -> Option<(String, RunSpec)> {
    let d = s.doc().ok()?;
    d.groups.iter().find(|g| g.members.contains(&h)).and_then(|g| {
        let spec: RunSpec = serde_json::from_str(g.description.strip_prefix(TAG)?).ok()?;
        Some((g.name.clone(), spec))
    })
}

fn erase_run(s: &mut Session, name: &str) -> Result<()> {
    let d = s.doc_mut()?;
    let members = d.groups.iter().find(|g| g.name == name).map(|g| g.members.clone()).unwrap_or_default();
    for h in members {
        d.remove_entity(h);
    }
    Ok(())
}

/// Redraw run `name` with `spec`; on failure the run stays as it was.
fn redraw(s: &mut Session, name: &str, spec: &RunSpec) -> Result<Vec<Handle>> {
    let before = s.doc()?.clone();
    erase_run(s, name)?;
    match add_run(s, spec, Some(name.to_string())) {
        Ok((_, hs)) => Ok(hs),
        Err(e) => {
            if let Ok(st) = s.state_mut() {
                st.doc = std::sync::Arc::new(before);
            }
            Err(e)
        }
    }
}

/// A run starting on another run: a tee goes on that run and the branch leaves it square.
fn start_on_run(s: &mut Session, mut spec: RunSpec) -> Result<RunSpec> {
    let (Some(p0), Some(p1)) = (spec.points.first().copied().map(v2), spec.points.get(1).copied().map(v2)) else { return Ok(spec) };
    let ap = s.pixel_size() * s.settings.pickbox.max(1.0) * 1.5;
    let space = s.space();
    let Some(h) = crate::select::pick(s.doc()?, &space, p0, ap.max(1e-6)) else { return Ok(spec) };
    let Some((name, mut parent)) = run_of(s, h) else { return Ok(spec) };
    let (_, psz) = check_spec(&parent.schedule, &parent.size)?;
    let pp: Vec<Vec2> = parent.points.iter().map(|p| v2(*p)).collect();
    // The parent segment nearest the pick, and the foot of the perpendicular on it.
    let mut best: Option<(f64, Vec2, Vec2)> = None;
    for w in pp.windows(2) {
        let u = (w[1] - w[0]).normalized();
        let t = (p0 - w[0]).dot(u).clamp(0.0, w[0].dist(w[1]));
        let q = w[0] + u * t;
        let dd = q.dist(p0);
        if best.is_none_or(|b| dd < b.0) {
            best = Some((dd, q, u));
        }
    }
    let Some((dist, q, u)) = best else { return Ok(spec) };
    if dist > psz.od {
        return Ok(spec);
    }
    let side = if (p1 - q).cross(u) < 0.0 { 1.0 } else { -1.0 };
    let w = perp(u) * side;
    let reach = (p1 - q).dot(w);
    if reach <= psz.tee {
        return Err(bad("pipe", "the branch must leave the run to one side"));
    }
    parent.tees.push(Tee { at: [q.x, q.y], side });
    redraw(s, &name, &parent)?;
    spec.points[0] = [q.x, q.y];
    // The branch leaves square to the run.
    let b = q + w * reach;
    spec.points[1] = [b.x, b.y];
    spec.from_tee = true;
    spec.flange_start = false;
    Ok(spec)
}

fn run_pipe(s: &mut Session, p: &Value) -> Result<Value> {
    let st = merge(&s.pipe, p)?;
    let pts = points_param(p, "points").ok_or_else(|| bad("pipe", "`points` is required"))?;
    let spec = RunSpec {
        schedule: st.schedule,
        size: st.size,
        points: pts.iter().map(|q| [q.x, q.y]).collect(),
        flange_start: st.flange_start,
        flange_end: st.flange_end,
        from_tee: false,
        tees: Vec::new(),
        label: st.label,
    };
    let spec = start_on_run(s, spec)?;
    let (name, hs) = add_run(s, &spec, None)?;
    Ok(json!({ "run": name, "handles": hs.iter().map(|h| h.hex()).collect::<Vec<_>>(), "fromTee": spec.from_tee }))
}

fn run_info(s: &mut Session, p: &Value) -> Result<Value> {
    let h = p.get("handle").and_then(super::curves::handle_value).ok_or_else(|| bad("pipe.run", "`handle` is required"))?;
    let (name, spec) = run_of(s, h).ok_or_else(|| bad("pipe.run", "not part of a pipe run"))?;
    let mut v = serde_json::to_value(&spec).map_err(|e| bad("pipe.run", e.to_string()))?;
    v["run"] = json!(name);
    Ok(v)
}

fn run_pipeedit(s: &mut Session, p: &Value) -> Result<Value> {
    let h = p.get("handle").and_then(super::curves::handle_value).ok_or_else(|| bad("pipeedit", "`handle` is required"))?;
    let (name, spec) = run_of(s, h).ok_or_else(|| bad("pipeedit", "not part of a pipe run"))?;
    let next: RunSpec = merge(&spec, p)?;
    let hs = redraw(s, &name, &next)?;
    s.set_selection(Vec::new());
    Ok(json!({ "run": name, "handles": hs.iter().map(|h| h.hex()).collect::<Vec<_>>() }))
}

/// PIPE typed: points until Enter; Settings (the first prompt's default) opens the Pipe dialog.
#[derive(Default)]
struct PipeM {
    pts: Vec<Vec2>,
}

impl Interactive for PipeM {
    fn name(&self) -> &'static str {
        "PIPE"
    }
    fn begin(&mut self, s: &mut Session) -> Result<Step> {
        let st = &s.pipe;
        s.echo(format!(
            "Pipe: {} {}{}{}.",
            st.size,
            st.schedule,
            if st.flange_start { ", flange at start" } else { "" },
            if st.flange_end { ", flange at end" } else { "" }
        ));
        Ok(Step::Continue)
    }
    fn prompt(&self, _s: &Session) -> Prompt {
        match self.pts.last() {
            None => Prompt::new("Specify start point of pipe", Accept::POINT).kw(&["Settings"]).default("Settings"),
            Some(l) => Prompt::new("Specify next point", Accept::POINT).kw(&["Undo"]).base(*l),
        }
    }
    fn input(&mut self, s: &mut Session, i: Input) -> Result<Step> {
        match i {
            Input::Point(p) => {
                if self.pts.last().is_none_or(|l| l.dist(p) > 1e-9) && self.pts.len() < MAX_POINTS {
                    self.pts.push(p);
                }
                Ok(Step::Continue)
            }
            Input::Keyword(k) | Input::Text(k) if k.to_ascii_lowercase().starts_with('u') && !self.pts.is_empty() => {
                self.pts.pop();
                Ok(Step::Continue)
            }
            Input::Enter | Input::Keyword(_) | Input::Text(_) if self.pts.is_empty() => {
                s.pipe_dialog = true;
                Ok(Step::Continue)
            }
            Input::Enter if self.pts.len() >= 2 => {
                let pts: Vec<[f64; 2]> = self.pts.iter().map(|p| [p.x, p.y]).collect();
                match run_pipe(s, &json!({ "points": pts })) {
                    Ok(_) => Ok(Step::Done),
                    Err(e) => {
                        s.echo(e.to_string());
                        Ok(Step::Continue)
                    }
                }
            }
            Input::Enter | Input::Cancel => {
                s.pipe_dialog = false;
                Ok(Step::Done)
            }
            _ => Ok(Step::Continue),
        }
    }
    fn preview(&self, s: &Session, c: Vec2) -> Vec<EntityKind> {
        if self.pts.is_empty() {
            return Vec::new();
        }
        let mut pts: Vec<[f64; 2]> = self.pts.iter().map(|p| [p.x, p.y]).collect();
        if self.pts.last().is_some_and(|l| l.dist(c) > 1e-9) {
            pts.push([c.x, c.y]);
        }
        let st = &s.pipe;
        let spec = RunSpec { schedule: st.schedule.clone(), size: st.size.clone(), points: pts.clone(), label: false, ..RunSpec::default() };
        match size(&st.size).map(|z| geometry(&spec, &z)) {
            Some(Ok(g)) => g.into_iter().map(|(k, _, _)| k).collect(),
            _ => pts.windows(2).map(|w| line(v2(w[0]), v2(w[1]))).collect(),
        }
    }
}

/// PIPEEDIT typed: pick a run; the UI opens the Pipe dialog on it.
struct PipeEditM;

impl Interactive for PipeEditM {
    fn name(&self) -> &'static str {
        "PIPEEDIT"
    }
    fn begin(&mut self, s: &mut Session) -> Result<Step> {
        if let Some(h) = s.selection().into_iter().find(|h| run_of(s, *h).is_some()) {
            s.pipe_edit = Some(h);
            return Ok(Step::Done);
        }
        Ok(Step::Continue)
    }
    fn prompt(&self, _s: &Session) -> Prompt {
        Prompt::new("Select a pipe run", Accept::SELECT)
    }
    fn input(&mut self, s: &mut Session, i: Input) -> Result<Step> {
        match i {
            Input::Pick(hs) => match hs.into_iter().find(|h| run_of(s, *h).is_some()) {
                Some(h) => {
                    s.pipe_edit = Some(h);
                    Ok(Step::Done)
                }
                None => {
                    s.echo("Not a pipe run.");
                    Ok(Step::Continue)
                }
            },
            Input::Enter | Input::Cancel => Ok(Step::Done),
            _ => Ok(Step::Continue),
        }
    }
}
