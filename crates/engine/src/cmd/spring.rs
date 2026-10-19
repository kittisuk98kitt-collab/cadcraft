//! SPRING: a helical compression spring drawn in side view, as a mechanical package's spring
//! generator places it: wire diameter, outside diameter, free length, active coils, end type
//! (ground and closed, closed, open) and representation (full, section, simplified). The spring
//! becomes one block reference at the start point, along the direction picked. Its rate and
//! solid length (steel, G = 79 300 N/mm² unless given) are reported.

use cadcraft_doc::{EntityKind, Handle};
use cadcraft_geom::{Arc, Mat3, Vec2};
use serde_json::{Value, json};

use super::helpers::{arc, circle, line, lwpoly};
use super::*;
use crate::{Accept, Input, Interactive, Prompt, Result, Session, Step};

/// The most coils drawn (input-derived loop bound).
const MAX_COILS: f64 = 500.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ends {
    Ground,
    Closed,
    Open,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rep {
    Full,
    Section,
    Simplified,
}

#[derive(Clone, Debug)]
pub struct Spec {
    /// Wire diameter.
    pub d: f64,
    /// Outside diameter.
    pub od: f64,
    /// Free length.
    pub len: f64,
    /// Active coils.
    pub n: f64,
    pub ends: Ends,
    pub rep: Rep,
    /// Shear modulus (N/mm²) for the rate.
    pub g: f64,
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("spring", "Compression Spring", run_spring)
            .menu(&["Draw", "Spring..."])
            .alias(&["amspring", "amcompspring", "compspring"])
            .params("{at: [x,y], angle? (degrees, axis direction; default 0), wire: d, od: outside diameter, length: free length, coils: active coils, ends?: ground|closed|open (default ground), view?: full|section|simplified (default full), g?: shear modulus N/mm² (default 79300, steel)}")
            .interactive(|s| Ok(Box::new(SpringM::new(s)))),
    ]
}

fn num(p: &Value, k: &str) -> Option<f64> {
    p.get(k).and_then(Value::as_f64).filter(|v| v.is_finite())
}

fn ends_of(t: &str) -> Option<Ends> {
    match t.trim().to_ascii_lowercase().chars().next()? {
        'g' => Some(Ends::Ground),
        'c' => Some(Ends::Closed),
        'o' => Some(Ends::Open),
        _ => None,
    }
}

fn rep_of(t: &str) -> Option<Rep> {
    match t.trim().to_ascii_lowercase().get(..2)? {
        "fu" => Some(Rep::Full),
        "se" => Some(Rep::Section),
        "si" => Some(Rep::Simplified),
        _ => None,
    }
}

impl Spec {
    fn check(&self) -> std::result::Result<(), String> {
        if !(self.d > 0.0 && self.od > self.d && self.len > 0.0 && self.n >= 1.0 && self.n <= MAX_COILS && self.g > 0.0) {
            return Err("needs wire > 0, outside diameter > wire, length > 0 and 1 to 500 active coils".into());
        }
        if self.pitch() <= self.d {
            return Err(format!("the spring is solid at this length: free length must exceed {:.3}", self.solid_length()));
        }
        Ok(())
    }
    /// Dead (closed) coils at each end.
    fn dead(&self) -> f64 {
        if self.ends == Ends::Open { 0.0 } else { 1.0 }
    }
    fn total_coils(&self) -> f64 {
        self.n + 2.0 * self.dead()
    }
    /// Pitch of the active coils.
    fn pitch(&self) -> f64 {
        (self.len - self.d - 2.0 * self.dead() * self.d) / self.n
    }
    fn solid_length(&self) -> f64 {
        (self.total_coils() + if self.ends == Ends::Ground { -0.5 } else { 1.0 }) * self.d
    }
    /// Mean coil diameter.
    fn mean(&self) -> f64 {
        self.od - self.d
    }
    /// Rate k = G d⁴ / (8 D³ n), N/mm.
    pub fn rate(&self) -> f64 {
        self.g * self.d.powi(4) / (8.0 * self.mean().powi(3) * self.n)
    }
    /// Axial position of the wire centre at turn parameter `t` (0 … total coils).
    fn x(&self, t: f64) -> f64 {
        let dead = self.dead();
        let half = self.d / 2.0;
        if t <= dead {
            half + t * self.d
        } else if t <= dead + self.n {
            half + dead * self.d + (t - dead) * self.pitch()
        } else {
            half + dead * self.d + self.n * self.pitch() + (t - dead - self.n) * self.d
        }
    }
    /// The turning points of the wire centre in side view: (x, ±R), top at whole turns, bottom at
    /// half turns.
    fn points(&self) -> Vec<Vec2> {
        let r = self.mean() / 2.0;
        let steps = (self.total_coils() * 2.0).round() as usize;
        (0..=steps)
            .map(|i| {
                let t = i as f64 / 2.0;
                Vec2::new(self.x(t), if i % 2 == 0 { r } else { -r })
            })
            .collect()
    }

    /// The spring's geometry along +X from the origin: (kind, on the CENTER linetype), plus the
    /// wire sections to fill (Section view).
    pub fn geometry(&self) -> (Vec<(EntityKind, bool)>, Vec<usize>) {
        let pts = self.points();
        let r = self.mean() / 2.0;
        let h = self.d / 2.0;
        let mut out: Vec<(EntityKind, bool)> = Vec::new();
        let mut fill = Vec::new();
        // The two outlines of the wire between turning points a and b.
        let strand = |out: &mut Vec<(EntityKind, bool)>, a: Vec2, b: Vec2| {
            let dir = (b - a).normalized();
            let nrm = Vec2::new(-dir.y, dir.x) * h;
            out.push((line(a + nrm, b + nrm), false));
            out.push((line(a - nrm, b - nrm), false));
        };
        match self.rep {
            Rep::Full => {
                // Near halves (top to bottom) in full; of the far halves only what shows between
                // them, as a drawing of the spring shows it.
                let band = |a: Vec2, b: Vec2| {
                    let dir = (b - a).normalized();
                    let nrm = Vec2::new(-dir.y, dir.x) * h;
                    [a + nrm, b + nrm, b - nrm, a - nrm]
                };
                let near: Vec<[Vec2; 4]> = pts.windows(2).step_by(2).map(|w| band(w[0], w[1])).collect();
                for (i, w) in pts.windows(2).enumerate() {
                    if i % 2 == 0 {
                        strand(&mut out, w[0], w[1]);
                        continue;
                    }
                    let q = band(w[0], w[1]);
                    for (a, b) in [(q[0], q[1]), (q[3], q[2])] {
                        for (pa, pb) in visible_parts(a, b, &near) {
                            out.push((line(pa, pb), false));
                        }
                    }
                }
                // Rounded turns: the wire's outer half at each turning point.
                for p in &pts {
                    let (s, e) = if p.y > 0.0 { (0.0, std::f64::consts::PI) } else { (std::f64::consts::PI, 2.0 * std::f64::consts::PI) };
                    out.push((arc(&Arc::new(*p, h, s, e)), false));
                }
            }
            Rep::Section => {
                // Wire sections at every turn, and behind the cut the far halves of the coils
                // (bottom to the next top).
                for p in &pts {
                    fill.push(out.len());
                    out.push((circle(*p, h), false));
                }
                for (i, w) in pts.windows(2).enumerate() {
                    if i % 2 == 1 {
                        strand(&mut out, w[0], w[1]);
                    }
                }
            }
            Rep::Simplified => {
                out.push((lwpoly(pts.iter().map(|p| cadcraft_geom::PolyVertex::new(*p)).collect(), false), false));
            }
        }
        // Ground ends: the flat faces.
        if self.ends == Ends::Ground {
            let y = r + h;
            out.push((line(Vec2::new(0.0, -y), Vec2::new(0.0, y)), false));
            out.push((line(Vec2::new(self.len, -y), Vec2::new(self.len, y)), false));
        }
        // The spring's axis.
        let ext = self.d.max(self.od * 0.1);
        out.push((line(Vec2::new(-ext, 0.0), Vec2::new(self.len + ext, 0.0)), true));
        (out, fill)
    }
}

/// The parts of segment a–b outside every convex quad in `covers`.
fn visible_parts(a: Vec2, b: Vec2, covers: &[[Vec2; 4]]) -> Vec<(Vec2, Vec2)> {
    let mut hidden: Vec<(f64, f64)> = covers.iter().filter_map(|q| clip_convex(a, b, q)).collect();
    hidden.sort_by(|x, y| x.0.total_cmp(&y.0));
    let mut out = Vec::new();
    let mut t = 0.0;
    for (t0, t1) in hidden {
        if t0 > t + 1e-9 {
            out.push((a + (b - a) * t, a + (b - a) * t0));
        }
        t = f64::max(t, t1);
    }
    if t < 1.0 - 1e-9 {
        out.push((a + (b - a) * t, b));
    }
    out
}

/// Cyrus–Beck: the parameter range of a–b inside convex polygon `q`.
fn clip_convex(a: Vec2, b: Vec2, q: &[Vec2; 4]) -> Option<(f64, f64)> {
    let d = b - a;
    // Orientation of the polygon, so the inward normals point inside.
    let area: f64 = (0..4).map(|i| q[i].cross(q[(i + 1) % 4])).sum();
    let sign = if area >= 0.0 { 1.0 } else { -1.0 };
    let (mut t0, mut t1) = (0.0_f64, 1.0_f64);
    for i in 0..4 {
        let (p, r) = (q[i], q[(i + 1) % 4]);
        let e = r - p;
        let inward = Vec2::new(-e.y, e.x) * sign;
        let num = inward.dot(a - p);
        let den = inward.dot(d);
        if den.abs() < 1e-12 {
            if num < 0.0 {
                return None;
            }
            continue;
        }
        let t = -num / den;
        if den > 0.0 {
            t0 = t0.max(t);
        } else {
            t1 = t1.min(t);
        }
        if t0 > t1 {
            return None;
        }
    }
    (t1 - t0 > 1e-9).then_some((t0, t1))
}

fn spec_of(p: &Value) -> Result<Spec> {
    let need = |k: &str| num(p, k).ok_or_else(|| bad("spring", format!("`{k}` is required")));
    let spec = Spec {
        d: need("wire")?,
        od: need("od")?,
        len: need("length")?,
        n: need("coils")?,
        ends: match str_param(p, "ends") {
            Some(t) => ends_of(t).ok_or_else(|| bad("spring", "`ends` is ground, closed or open"))?,
            None => Ends::Ground,
        },
        rep: match str_param(p, "view") {
            Some(t) => rep_of(t).ok_or_else(|| bad("spring", "`view` is full, section or simplified"))?,
            None => Rep::Full,
        },
        g: num(p, "g").unwrap_or(79_300.0),
    };
    spec.check().map_err(|m| bad("spring", m))?;
    Ok(spec)
}

/// Draw `spec` from `at` along `angle` (radians) as one block reference.
fn place(s: &mut Session, spec: &Spec, at: Vec2, angle: f64) -> Result<Value> {
    let (geo, fill) = spec.geometry();
    let m = Mat3::translate(at).then_before(Mat3::rotate(angle));
    {
        let d = s.doc_mut()?;
        if d.linetype("CENTER").is_none()
            && let Some(lt) = cadcraft_doc::library::standard_linetypes_for(d).into_iter().find(|l| l.name == "CENTER")
        {
            d.linetypes.push(lt);
        }
    }
    let mut hs: Vec<Handle> = Vec::new();
    let mut fill_hs = Vec::new();
    for (i, (mut k, center)) in geo.into_iter().enumerate() {
        k.transform(&m);
        let h = s.add_entity(k)?;
        if center {
            s.doc_mut()?.modify_entity(h, |e| e.common.linetype = "CENTER".into()).map_err(|e| bad("spring", e.to_string()))?;
        }
        if fill.contains(&i) {
            fill_hs.push(h);
        }
        hs.push(h);
    }
    // Section view: the wire sections filled solid.
    if !fill_hs.is_empty()
        && let Some(hatch) = find_command("hatch")
    {
        let before: Vec<Handle> = s.doc()?.space(&s.space()).map(|st| st.handles()).unwrap_or_default();
        (hatch.run)(s, &json!({ "handles": fill_hs.iter().map(|h| h.hex()).collect::<Vec<_>>(), "pattern": "SOLID", "associative": false }))?;
        let after: Vec<Handle> = s.doc()?.space(&s.space()).map(|st| st.handles()).unwrap_or_default();
        hs.extend(after.into_iter().filter(|h| !before.contains(h)));
    }
    // One block per spring: SPRING_<wire>x<od>x<length>, numbered if taken.
    let base = format!("SPRING_{}x{}x{}", fmt(spec.d), fmt(spec.od), fmt(spec.len));
    let d = s.doc()?;
    let name = std::iter::once(base.clone()).chain((2..).map(|i| format!("{base}_{i}"))).find(|n| d.block(n).is_none()).unwrap_or(base);
    let block = find_command("block").ok_or_else(|| bad("spring", "BLOCK is not available"))?;
    let r = (block.run)(
        s,
        &json!({ "name": name, "base": [at.x, at.y], "handles": hs.iter().map(|h| h.hex()).collect::<Vec<_>>(), "keep": "convert", "description": "Compression spring" }),
    )?;
    let rate = spec.rate();
    let msg = format!(
        "Spring {name}: rate {rate:.3} N/mm, solid length {:.3}, travel to solid {:.3}, spring index {:.2}.",
        spec.solid_length(),
        spec.len - spec.solid_length(),
        spec.mean() / spec.d
    );
    s.echo(msg);
    Ok(json!({
        "block": name,
        "insert": r.get("handle").or_else(|| r.get("insert")).cloned().unwrap_or(Value::Null),
        "rate": rate,
        "solidLength": spec.solid_length(),
        "travel": spec.len - spec.solid_length(),
        "index": spec.mean() / spec.d,
    }))
}

fn fmt(v: f64) -> String {
    let s = format!("{v:.3}");
    s.trim_end_matches('0').trim_end_matches('.').replace('.', "_")
}

fn run_spring(s: &mut Session, p: &Value) -> Result<Value> {
    let spec = spec_of(p)?;
    let at = points_param(p, "at").and_then(|v| v.first().copied()).or_else(|| {
        let a = p.get("at")?.as_array()?;
        Some(Vec2::new(a.first()?.as_f64()?, a.get(1)?.as_f64()?))
    });
    let at = at.ok_or_else(|| bad("spring", "`at` [x,y] is required"))?;
    let angle = num(p, "angle").unwrap_or(0.0).to_radians();
    place(s, &spec, at, angle)
}

/// SPRING typed: start point, end point (the free length and direction), then the sizes, end
/// type and view, each with the last value as default.
struct SpringM {
    start: Option<Vec2>,
    end: Option<Vec2>,
    step: usize,
    spec: Spec,
}

impl SpringM {
    fn new(s: &Session) -> Self {
        let spec = s.last_spring.clone().unwrap_or(Spec { d: 2.0, od: 20.0, len: 50.0, n: 6.0, ends: Ends::Ground, rep: Rep::Full, g: 79_300.0 });
        SpringM { start: None, end: None, step: 0, spec }
    }
}

impl Interactive for SpringM {
    fn name(&self) -> &'static str {
        "SPRING"
    }
    fn prompt(&self, _s: &Session) -> Prompt {
        let num_prompt =
            |m: &str, v: f64| Prompt::new(m, Accept { point: false, number: true, text: true, select: false, enter: true }).default(format!("{v}"));
        match (self.start, self.end, self.step) {
            (None, _, _) => Prompt::new("Specify start point of spring", Accept::POINT),
            (Some(a), None, _) => Prompt::new("Specify end point (free length)", Accept::POINT).base(a),
            (_, _, 0) => num_prompt("Outside diameter", self.spec.od),
            (_, _, 1) => num_prompt("Wire diameter", self.spec.d),
            (_, _, 2) => num_prompt("Active coils", self.spec.n),
            (_, _, 3) => Prompt::new("Ends", crate::cmd::curves::KW).kw(&["Ground", "Closed", "Open"]).default(match self.spec.ends {
                Ends::Ground => "Ground",
                Ends::Closed => "Closed",
                Ends::Open => "Open",
            }),
            _ => Prompt::new("View", crate::cmd::curves::KW).kw(&["Full", "Section", "SImplified"]).default(match self.spec.rep {
                Rep::Full => "Full",
                Rep::Section => "Section",
                Rep::Simplified => "SImplified",
            }),
        }
    }
    fn input(&mut self, s: &mut Session, i: Input) -> Result<Step> {
        if i == Input::Cancel {
            return Ok(Step::Done);
        }
        let Some(a) = self.start else {
            if let Input::Point(p) = i {
                self.start = Some(p);
            }
            return Ok(Step::Continue);
        };
        if self.end.is_none() {
            match i {
                Input::Point(p) if p.dist(a) > 1e-9 => {
                    self.end = Some(p);
                    self.spec.len = p.dist(a);
                }
                _ => {}
            }
            return Ok(Step::Continue);
        }
        let text = match &i {
            Input::Text(t) | Input::Keyword(t) => Some(t.trim().to_string()),
            Input::Enter => None,
            _ => return Ok(Step::Continue),
        };
        match self.step {
            0..=2 => {
                if let Some(t) = text {
                    match t.parse::<f64>() {
                        Ok(v) if v.is_finite() && v > 0.0 => match self.step {
                            0 => self.spec.od = v,
                            1 => self.spec.d = v,
                            _ => self.spec.n = v,
                        },
                        _ => {
                            s.echo("Requires a positive number.");
                            return Ok(Step::Continue);
                        }
                    }
                }
            }
            3 => {
                if let Some(e) = text.as_deref().and_then(ends_of) {
                    self.spec.ends = e;
                }
            }
            _ => {
                if let Some(r) = text.as_deref().and_then(rep_of) {
                    self.spec.rep = r;
                }
                if let Err(m) = self.spec.check() {
                    s.echo(m);
                    self.step = 0;
                    return Ok(Step::Continue);
                }
                let end = self.end.unwrap_or(a);
                s.last_spring = Some(self.spec.clone());
                place(s, &self.spec, a, (end - a).angle())?;
                return Ok(Step::Done);
            }
        }
        self.step += 1;
        Ok(Step::Continue)
    }
    fn preview(&self, _s: &Session, c: Vec2) -> Vec<EntityKind> {
        let Some(a) = self.start else { return Vec::new() };
        let end = self.end.unwrap_or(c);
        if end.dist(a) <= 1e-9 {
            return Vec::new();
        }
        let mut sp = self.spec.clone();
        sp.len = end.dist(a);
        sp.rep = Rep::Simplified;
        if sp.check().is_err() {
            return vec![line(a, end)];
        }
        let m = Mat3::translate(a).then_before(Mat3::rotate((end - a).angle()));
        sp.geometry()
            .0
            .into_iter()
            .map(|(mut k, _)| {
                k.transform(&m);
                k
            })
            .collect()
    }
}
