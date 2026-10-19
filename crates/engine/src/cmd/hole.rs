//! TAPHOLE: tapped and drilled holes drawn to JIS drafting practice, with sizes from the JIS
//! tables: metric coarse and fine threads (M, JIS B 0205), parallel pipe threads (G, JIS B 0202)
//! and taper pipe threads (Rc / PT, JIS B 0203), each with its recommended tap drill.
//!
//! Top view: the tap drill as a thick circle, the thread's major diameter as a thin
//! three-quarter circle, and centre lines. Side (section) view: the drill with its 118° point,
//! thin lines at the major diameter down to the thread depth and a thick line where the full
//! thread ends. Each hole is a block reference (one block per size and view), so holes copy and
//! move as one. The settings (TAPHOLE's Settings option, `taphole.settings`) are remembered.

use cadcraft_doc::{EntityKind, Handle, Lineweight};
use cadcraft_geom::{Arc, Mat3, Vec2};
use serde_json::{Value, json};

use super::helpers::{arc, circle, line};
use super::*;
use crate::{Accept, HoleSettings, Input, Interactive, Prompt, Result, Session, Step};

/// One thread size: designation, major diameter, pitch, recommended tap drill and the default
/// tapped depth (taper threads: the usable thread length).
#[derive(Clone, Copy, Debug)]
pub struct Thread {
    pub name: &'static str,
    pub major: f64,
    pub pitch: f64,
    pub drill: f64,
    pub depth: f64,
}

const fn t(name: &'static str, major: f64, pitch: f64, drill: f64, depth: f64) -> Thread {
    Thread { name, major, pitch, drill, depth }
}

/// JIS B 0205 metric coarse threads; tap drills after JIS B 1004.
pub const M_COARSE: &[Thread] = &[
    t("M1", 1.0, 0.25, 0.75, 1.5),
    t("M1.2", 1.2, 0.25, 0.95, 2.0),
    t("M1.4", 1.4, 0.3, 1.1, 2.0),
    t("M1.6", 1.6, 0.35, 1.25, 2.5),
    t("M2", 2.0, 0.4, 1.6, 3.0),
    t("M2.5", 2.5, 0.45, 2.05, 4.0),
    t("M3", 3.0, 0.5, 2.5, 4.5),
    t("M4", 4.0, 0.7, 3.3, 6.0),
    t("M5", 5.0, 0.8, 4.2, 7.5),
    t("M6", 6.0, 1.0, 5.0, 9.0),
    t("M8", 8.0, 1.25, 6.8, 12.0),
    t("M10", 10.0, 1.5, 8.5, 15.0),
    t("M12", 12.0, 1.75, 10.2, 18.0),
    t("M14", 14.0, 2.0, 12.0, 21.0),
    t("M16", 16.0, 2.0, 14.0, 24.0),
    t("M18", 18.0, 2.5, 15.5, 27.0),
    t("M20", 20.0, 2.5, 17.5, 30.0),
    t("M22", 22.0, 2.5, 19.5, 33.0),
    t("M24", 24.0, 3.0, 21.0, 36.0),
    t("M27", 27.0, 3.0, 24.0, 40.5),
    t("M30", 30.0, 3.5, 26.5, 45.0),
    t("M33", 33.0, 3.5, 29.5, 49.5),
    t("M36", 36.0, 4.0, 32.0, 54.0),
    t("M39", 39.0, 4.0, 35.0, 58.5),
    t("M42", 42.0, 4.5, 37.5, 63.0),
    t("M48", 48.0, 5.0, 43.0, 72.0),
];

/// JIS B 0205 metric fine threads (common sizes); tap drill = major − pitch.
pub const M_FINE: &[Thread] = &[
    t("M3x0.35", 3.0, 0.35, 2.65, 4.5),
    t("M4x0.5", 4.0, 0.5, 3.5, 6.0),
    t("M5x0.5", 5.0, 0.5, 4.5, 7.5),
    t("M6x0.75", 6.0, 0.75, 5.25, 9.0),
    t("M8x1", 8.0, 1.0, 7.0, 12.0),
    t("M10x1.25", 10.0, 1.25, 8.8, 15.0),
    t("M10x1", 10.0, 1.0, 9.0, 15.0),
    t("M12x1.5", 12.0, 1.5, 10.5, 18.0),
    t("M12x1.25", 12.0, 1.25, 10.8, 18.0),
    t("M14x1.5", 14.0, 1.5, 12.5, 21.0),
    t("M16x1.5", 16.0, 1.5, 14.5, 24.0),
    t("M18x1.5", 18.0, 1.5, 16.5, 27.0),
    t("M20x1.5", 20.0, 1.5, 18.5, 30.0),
    t("M22x1.5", 22.0, 1.5, 20.5, 33.0),
    t("M24x2", 24.0, 2.0, 22.0, 36.0),
    t("M27x2", 27.0, 2.0, 25.0, 40.5),
    t("M30x2", 30.0, 2.0, 28.0, 45.0),
];

const fn tpi(n: f64) -> f64 {
    25.4 / n
}

/// JIS B 0202 (ISO 228) parallel pipe threads G.
pub const G_PARALLEL: &[Thread] = &[
    t("G1/16", 7.723, tpi(28.0), 6.8, 6.5),
    t("G1/8", 9.728, tpi(28.0), 8.8, 6.5),
    t("G1/4", 13.157, tpi(19.0), 11.8, 9.7),
    t("G3/8", 16.662, tpi(19.0), 15.25, 10.1),
    t("G1/2", 20.955, tpi(14.0), 19.0, 13.2),
    t("G5/8", 22.911, tpi(14.0), 21.0, 13.2),
    t("G3/4", 26.441, tpi(14.0), 24.5, 14.5),
    t("G7/8", 30.201, tpi(14.0), 28.25, 14.5),
    t("G1", 33.249, tpi(11.0), 30.75, 16.8),
    t("G1-1/4", 41.910, tpi(11.0), 39.5, 19.1),
    t("G1-1/2", 47.803, tpi(11.0), 45.25, 19.1),
    t("G2", 59.614, tpi(11.0), 57.0, 23.4),
];

/// JIS B 0203 taper pipe threads (internal Rc, formerly PT): diameters at the gauge plane, the
/// taper tap drill and the usable thread length.
pub const PT_TAPER: &[Thread] = &[
    t("PT1/16", 7.723, tpi(28.0), 6.6, 6.5),
    t("PT1/8", 9.728, tpi(28.0), 8.4, 6.5),
    t("PT1/4", 13.157, tpi(19.0), 11.2, 9.7),
    t("PT3/8", 16.662, tpi(19.0), 14.7, 10.1),
    t("PT1/2", 20.955, tpi(14.0), 18.3, 13.2),
    t("PT3/4", 26.441, tpi(14.0), 23.6, 14.5),
    t("PT1", 33.249, tpi(11.0), 29.7, 16.8),
    t("PT1-1/4", 41.910, tpi(11.0), 38.3, 19.1),
    t("PT1-1/2", 47.803, tpi(11.0), 44.2, 19.1),
    t("PT2", 59.614, tpi(11.0), 55.8, 23.4),
];

/// The table of a thread kind: M (coarse), MF (fine), G, PT. DRILL has none.
pub fn table(kind: &str) -> Option<&'static [Thread]> {
    match kind.to_ascii_uppercase().as_str() {
        "M" => Some(M_COARSE),
        "MF" => Some(M_FINE),
        "G" => Some(G_PARALLEL),
        "PT" | "RC" => Some(PT_TAPER),
        _ => None,
    }
}

pub fn find(kind: &str, size: &str) -> Option<Thread> {
    let want = size.trim().to_ascii_uppercase().replace(['×', '*'], "X").replace(' ', "");
    let want = want.strip_prefix("RC").map(|r| format!("PT{r}")).unwrap_or(want);
    table(kind)?
        .iter()
        .find(|th| th.name.to_ascii_uppercase() == want || th.name.to_ascii_uppercase().trim_start_matches(['M', 'G']) == want)
        .copied()
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("taphole", "Tapped Hole", run_taphole)
            .menu(&["Draw", "Hole..."])
            .alias(&["hole", "amhole", "tap"])
            .params("{at: [[x,y]...], kind?: M|MF|G|PT|DRILL, size?: \"M8\" | \"G1/4\" | \"PT1/4\", view?: top|side, through?: bool, thickness? (through, side view), threadDepth?, drillDepth?, drill? (DRILL diameter), angle? (side view: degrees the hole turns from straight down), note?: bool} — missing values from the hole settings")
            .interactive(|s| Ok(Box::new(TapHoleM::new(s)))),
        CommandSpec::new("taphole.m", "Tapped Hole M", |s, p| run_kind(s, p, "M")).interactive(|s| Ok(Box::new(TapHoleM::preset(s, "M")))),
        CommandSpec::new("taphole.mf", "Tapped Hole M Fine", |s, p| run_kind(s, p, "MF")).interactive(|s| Ok(Box::new(TapHoleM::preset(s, "MF")))),
        CommandSpec::new("taphole.g", "Tapped Hole G", |s, p| run_kind(s, p, "G")).interactive(|s| Ok(Box::new(TapHoleM::preset(s, "G")))),
        CommandSpec::new("taphole.pt", "Tapped Hole PT", |s, p| run_kind(s, p, "PT")).interactive(|s| Ok(Box::new(TapHoleM::preset(s, "PT")))),
        CommandSpec::new("taphole.drill", "Drilled Hole", |s, p| run_kind(s, p, "DRILL")).interactive(|s| Ok(Box::new(TapHoleM::preset(s, "DRILL")))),
        CommandSpec::new("taphole.settings", "Hole Settings", run_settings)
            .params("{kind?, size?, view?, through?, thickness?, threadDepth? (0 = from the table), drillDepth? (0 = thread depth + 4 pitches), drill?, angle?, note?} → the settings and the sizes of the kind")
            .noundo(),
    ]
}

fn sizes_json(kind: &str) -> Value {
    json!(
        table(kind)
            .unwrap_or(&[])
            .iter()
            .map(|th| json!({ "name": th.name, "major": th.major, "pitch": th.pitch, "drill": th.drill, "depth": th.depth }))
            .collect::<Vec<_>>()
    )
}

fn merge(st: &HoleSettings, p: &Value) -> Result<HoleSettings> {
    let mut cur = serde_json::to_value(st).map_err(|e| bad("taphole", e.to_string()))?;
    if let (Some(c), Some(o)) = (cur.as_object_mut(), p.as_object()) {
        for (k, v) in o {
            if c.contains_key(k) {
                c.insert(k.clone(), v.clone());
            }
        }
    }
    let mut next: HoleSettings = serde_json::from_value(cur).map_err(|e| bad("taphole", e.to_string()))?;
    next.kind = next.kind.trim().to_ascii_uppercase();
    if next.kind == "RC" {
        next.kind = "PT".into();
    }
    if !matches!(next.kind.as_str(), "M" | "MF" | "G" | "PT" | "DRILL") {
        return Err(bad("taphole", "`kind` is M, MF, G, PT or DRILL"));
    }
    next.view = next.view.trim().to_ascii_lowercase();
    if !matches!(next.view.as_str(), "top" | "side") {
        return Err(bad("taphole", "`view` is top or side"));
    }
    if next.kind != "DRILL" {
        match find(&next.kind, &next.size) {
            Some(th) => next.size = th.name.into(),
            None if p.get("size").and_then(Value::as_str).is_some() => {
                return Err(bad("taphole", format!("no {} size \"{}\"", next.kind, next.size)));
            }
            // Another kind chosen: its first common size.
            None => next.size = table(&next.kind).and_then(|t| t.get(t.len().min(10) / 2)).map(|th| th.name.to_string()).unwrap_or_default(),
        }
    }
    for v in [next.thickness, next.thread_depth, next.drill_depth, next.drill] {
        if !v.is_finite() || !(0.0..=1e6).contains(&v) {
            return Err(bad("taphole", "depths and diameters are 0 or more"));
        }
    }
    Ok(next)
}

fn run_settings(s: &mut Session, p: &Value) -> Result<Value> {
    s.hole = merge(&s.hole, p)?;
    let mut v = serde_json::to_value(&s.hole).map_err(|e| bad("taphole", e.to_string()))?;
    v["sizes"] = sizes_json(&s.hole.kind);
    Ok(v)
}

/// The resolved hole: diameters and depths.
struct Hole {
    label: String,
    major: Option<f64>,
    drill: f64,
    thread_depth: f64,
    drill_depth: f64,
    through: Option<f64>,
}

fn resolve(st: &HoleSettings) -> Result<Hole> {
    if st.kind == "DRILL" {
        if st.drill <= 0.0 {
            return Err(bad("taphole", "`drill` (diameter) is required for a drilled hole"));
        }
        let depth = if st.drill_depth > 0.0 { st.drill_depth } else { 2.0 * st.drill };
        return Ok(Hole {
            label: format!("%%c{}", fmt(st.drill)),
            major: None,
            drill: st.drill,
            thread_depth: 0.0,
            drill_depth: depth,
            through: st.through.then_some(st.thickness.max(depth)),
        });
    }
    let th = find(&st.kind, &st.size).ok_or_else(|| bad("taphole", format!("no {} size \"{}\"", st.kind, st.size)))?;
    let label = match st.kind.as_str() {
        "M" => format!("{}x{}", th.name, fmt(th.pitch)),
        "PT" => format!("Rc{} ({})", th.name.trim_start_matches("PT"), th.name),
        _ => th.name.to_string(),
    };
    let thread_depth = if st.thread_depth > 0.0 { st.thread_depth } else { th.depth };
    let drill_depth = if st.drill_depth > 0.0 { st.drill_depth.max(thread_depth) } else { round_half(thread_depth + 4.0 * th.pitch) };
    Ok(Hole {
        label,
        major: Some(th.major),
        drill: th.drill,
        thread_depth,
        drill_depth,
        through: st.through.then_some(st.thickness.max(thread_depth)),
    })
}

fn round_half(v: f64) -> f64 {
    (v * 2.0).ceil() / 2.0
}

fn fmt(v: f64) -> String {
    let s = format!("{v:.3}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// (kind, thin line, centre line) in hole coordinates: top view centred on the origin; side
/// view from the surface at the origin down −Y.
fn geometry(h: &Hole, side: bool) -> Vec<(EntityKind, bool, bool)> {
    let mut v = Vec::new();
    let r = h.drill / 2.0;
    let big = h.major.unwrap_or(h.drill) / 2.0;
    let ext = (big * 0.25).max(1.0);
    if !side {
        v.push((circle(Vec2::ZERO, r), false, false));
        if let Some(m) = h.major {
            // Three quarters of the major diameter, open in the first quadrant (JIS B 0002).
            v.push((arc(&Arc::new(Vec2::ZERO, m / 2.0, std::f64::consts::FRAC_PI_2, 2.0 * std::f64::consts::PI)), true, false));
        }
        v.push((line(Vec2::new(-big - ext, 0.0), Vec2::new(big + ext, 0.0)), true, true));
        v.push((line(Vec2::new(0.0, -big - ext), Vec2::new(0.0, big + ext)), true, true));
        return v;
    }
    let bottom = h.through.unwrap_or(h.drill_depth);
    // Drill walls, and its 118° point when blind.
    v.push((line(Vec2::new(-r, 0.0), Vec2::new(-r, -bottom)), false, false));
    v.push((line(Vec2::new(r, 0.0), Vec2::new(r, -bottom)), false, false));
    let tip = if h.through.is_some() {
        bottom
    } else {
        let tip = bottom + r / 59f64.to_radians().tan();
        v.push((line(Vec2::new(-r, -bottom), Vec2::new(0.0, -tip)), false, false));
        v.push((line(Vec2::new(r, -bottom), Vec2::new(0.0, -tip)), false, false));
        tip
    };
    if let Some(m) = h.major {
        let td = if h.through.is_some() { bottom } else { h.thread_depth };
        let mr = m / 2.0;
        v.push((line(Vec2::new(-mr, 0.0), Vec2::new(-mr, -td)), true, false));
        v.push((line(Vec2::new(mr, 0.0), Vec2::new(mr, -td)), true, false));
        if h.through.is_none() {
            // Where the full thread ends.
            v.push((line(Vec2::new(-mr, -td), Vec2::new(mr, -td)), false, false));
        }
    }
    v.push((line(Vec2::new(0.0, ext), Vec2::new(0.0, -tip - ext)), true, true));
    v
}

fn block_name(st: &HoleSettings, h: &Hole) -> String {
    let base = if st.kind == "DRILL" { format!("D{}", fmt(h.drill)) } else { st.size.replace('/', "-") };
    let view = if st.view == "side" {
        match h.through {
            Some(t) => format!("SIDE_THRU{}", fmt(t)),
            None => format!("SIDE_{}_{}", fmt(h.thread_depth), fmt(h.drill_depth)),
        }
    } else {
        "TOP".into()
    };
    format!("HOLE_{base}_{view}").replace('.', "_")
}

/// The block for the hole, made once.
fn ensure_block(s: &mut Session, st: &HoleSettings, h: &Hole) -> Result<String> {
    let name = block_name(st, h);
    if s.doc()?.block(&name).is_some() {
        return Ok(name);
    }
    {
        let d = s.doc_mut()?;
        if d.linetype("CENTER").is_none()
            && let Some(lt) = cadcraft_doc::library::standard_linetypes_for(d).into_iter().find(|l| l.name == "CENTER")
        {
            d.linetypes.push(lt);
        }
    }
    let mut hs: Vec<Handle> = Vec::new();
    for (k, thin, center) in geometry(h, st.view == "side") {
        let e = s.add_entity(k)?;
        s.doc_mut()?
            .modify_entity(e, |x| {
                // JIS line widths: visible outlines thick, thread and centre lines thin.
                x.common.lineweight = Lineweight::Mm100(if thin { 25 } else { 50 });
                if center {
                    x.common.linetype = "CENTER".into();
                }
            })
            .map_err(|e| bad("taphole", e.to_string()))?;
        hs.push(e);
    }
    let block = find_command("block").ok_or_else(|| bad("taphole", "BLOCK is not available"))?;
    (block.run)(
        s,
        &json!({ "name": name, "base": [0, 0], "handles": hs.iter().map(|h| h.hex()).collect::<Vec<_>>(), "keep": "delete", "description": h.label }),
    )?;
    Ok(name)
}

/// The callout: size, depths and tap drill.
fn note_text(st: &HoleSettings, h: &Hole) -> String {
    match (st.kind.as_str(), h.through) {
        ("DRILL", Some(_)) => format!("{} THRU", h.label),
        ("DRILL", None) => format!("{} DEPTH {}", h.label, fmt(h.drill_depth)),
        (_, Some(_)) => format!("{} THRU\\PTAP DRILL %%c{}", h.label, fmt(h.drill)),
        _ => format!("{} DEPTH {}\\PTAP DRILL %%c{} DEPTH {}", h.label, fmt(h.thread_depth), fmt(h.drill), fmt(h.drill_depth)),
    }
}

fn place(s: &mut Session, st: &HoleSettings, at: &[Vec2]) -> Result<Value> {
    let h = resolve(st)?;
    let name = ensure_block(s, st, &h)?;
    let ins = find_command("insert").ok_or_else(|| bad("taphole", "INSERT is not available"))?;
    let mut out = Vec::new();
    for p in at.iter().take(10_000) {
        let rot = if st.view == "side" { st.angle } else { 0.0 };
        let r = (ins.run)(s, &json!({ "name": name, "at": [p.x, p.y], "rotation": rot }))?;
        out.push(r.get("handle").cloned().unwrap_or(Value::Null));
        if st.note {
            let big = h.major.unwrap_or(h.drill);
            let height = s.doc().map(|d| d.header.f64("TEXTSIZE", 2.5)).unwrap_or(2.5);
            let m = Mat3::rotate(rot.to_radians());
            let off = m.apply(Vec2::new(big * 0.8, if st.view == "side" { 0.0 } else { big * 0.8 }));
            let mt = EntityKind::MText(cadcraft_doc::MText {
                insert: super::helpers::v3(*p + off + Vec2::new(0.0, height * 2.0)),
                height,
                width: 0.0,
                attach: 7,
                rotation: 0.0,
                style: s.doc().map(|d| d.header.str("TEXTSTYLE", "Standard")).unwrap_or_else(|_| "Standard".into()),
                contents: note_text(st, &h),
                line_spacing: 1.0,
                line_spacing_exact: false,
                columns: None,
            });
            s.add_entity(mt)?;
        }
    }
    let msg = format!("{} hole(s): {}", out.len(), note_text(st, &h).replace("\\P", ", ").replace("%%c", "Ø"));
    s.echo(msg);
    Ok(json!({ "block": name, "handles": out, "drill": h.drill, "major": h.major, "threadDepth": h.thread_depth, "drillDepth": h.drill_depth }))
}

/// A tool for one thread kind: the hole settings switch to it.
fn run_kind(s: &mut Session, p: &Value, kind: &str) -> Result<Value> {
    let mut p = p.clone();
    if let Some(o) = p.as_object_mut() {
        o.insert("kind".into(), json!(kind));
    } else {
        p = json!({ "kind": kind });
    }
    if p.get("at").is_none() {
        return run_settings(s, &p);
    }
    run_taphole(s, &p)
}

fn run_taphole(s: &mut Session, p: &Value) -> Result<Value> {
    let st = merge(&s.hole, p)?;
    let at = points_param(p, "at")
        .or_else(|| {
            let a = p.get("at")?.as_array()?;
            Some(vec![Vec2::new(a.first()?.as_f64()?, a.get(1)?.as_f64()?)])
        })
        .filter(|v| !v.is_empty())
        .ok_or_else(|| bad("taphole", "`at` (a point or a list of points) is required"))?;
    place(s, &st, &at)
}

/// TAPHOLE typed: hole locations until Enter; Settings (the default) opens the Hole dialog.
struct TapHoleM {
    placed: usize,
    /// A Tool Sets tool's thread kind, set as the command starts.
    kind: Option<&'static str>,
}

impl TapHoleM {
    fn new(_s: &Session) -> Self {
        TapHoleM { placed: 0, kind: None }
    }
    fn preset(_s: &Session, kind: &'static str) -> Self {
        TapHoleM { placed: 0, kind: Some(kind) }
    }
}

impl Interactive for TapHoleM {
    fn name(&self) -> &'static str {
        "TAPHOLE"
    }
    fn begin(&mut self, s: &mut Session) -> Result<Step> {
        if let Some(k) = self.kind
            && s.hole.kind != k
        {
            s.hole = merge(&s.hole, &json!({ "kind": k }))?;
            // A new kind: choose its size first.
            s.hole_dialog = true;
        }
        let st = &s.hole;
        let what = if st.kind == "DRILL" { format!("drill %%c{}", fmt(st.drill)) } else { st.size.clone() };
        s.echo(format!("Hole: {what}, {} view{}.", st.view, if st.through { ", through" } else { "" }));
        Ok(Step::Continue)
    }
    fn prompt(&self, _s: &Session) -> Prompt {
        let p = Prompt::new("Specify hole location", Accept::POINT).kw(&["Settings"]);
        if self.placed == 0 { p.default("Settings") } else { p }
    }
    fn input(&mut self, s: &mut Session, i: Input) -> Result<Step> {
        match i {
            Input::Point(p) => {
                let st = s.hole.clone();
                place(s, &st, &[p])?;
                self.placed += 1;
                Ok(Step::Continue)
            }
            Input::Enter if self.placed > 0 => Ok(Step::Done),
            Input::Enter | Input::Keyword(_) | Input::Text(_) => {
                s.hole_dialog = true;
                Ok(Step::Continue)
            }
            Input::Cancel => {
                s.hole_dialog = false;
                Ok(Step::Done)
            }
            _ => Ok(Step::Continue),
        }
    }
    fn preview(&self, s: &Session, c: Vec2) -> Vec<EntityKind> {
        let st = &s.hole;
        let Ok(h) = resolve(st) else { return Vec::new() };
        let m = Mat3::translate(c).then_before(Mat3::rotate(if st.view == "side" { st.angle.to_radians() } else { 0.0 }));
        geometry(&h, st.view == "side")
            .into_iter()
            .map(|(mut k, _, _)| {
                k.transform(&m);
                k
            })
            .collect()
    }
}
