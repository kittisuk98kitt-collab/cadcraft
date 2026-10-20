//! XCLIP: clip block references (and xrefs) to a boundary: a rectangle, a polygon or a closed
//! polyline, the inside shown (or, inverted, the outside). ON/OFF turn clipping on and off,
//! Delete removes the boundary and generate Polyline draws it.

use cadcraft_doc::{EntityKind, Handle, XClip};
use cadcraft_geom::{Mat3, PolyVertex, Vec2};
use serde_json::{Value, json};

use super::helpers::{line, lwpoly};
use super::machines::{SelOutcome, SelectPhase};
use super::*;
use crate::{Accept, Input, Interactive, Prompt, Result, Session, Step};

/// Keywords (or Enter) only.
const KW: Accept = Accept { point: false, number: false, text: false, select: false, enter: true };

/// Most boundary points.
const MAX_POINTS: usize = 10_000;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("xclip", "Clip", run_xclip)
            .menu(&["Modify", "Clip", "Xref"])
            .alias(&["xc", "clip"])
            .params(
                "{handles?: [hex] (block references; default the selection), op?: new|on|off|delete|polyline (default new), \
                 boundary?: [[x,y]...] (world; two points = a rectangle), invert?: bool}",
            )
            .interactive(|_| Ok(Box::new(XClipM::default()))),
    ]
}

/// The block-to-world transform of block reference `h`.
fn block_xf(s: &Session, h: Handle) -> Option<Mat3> {
    let d = s.doc().ok()?;
    let e = d.entity(h)?;
    let EntityKind::Insert(i) = &e.kind else { return None };
    let base = d.block(&i.block).map_or(Vec2::ZERO, |b| b.base.xy());
    Some(i.transform(base))
}

/// Two corners as a rectangle; more points as they are.
fn polygon(pts: &[Vec2]) -> Vec<Vec2> {
    match pts {
        [a, b] => vec![*a, Vec2::new(b.x, a.y), *b, Vec2::new(a.x, b.y)],
        _ => pts.iter().take(MAX_POINTS).copied().collect(),
    }
}

fn inserts(s: &Session, hs: &[Handle]) -> Vec<Handle> {
    let Ok(d) = s.doc() else { return Vec::new() };
    hs.iter().copied().filter(|h| d.entity(*h).is_some_and(|e| matches!(e.kind, EntityKind::Insert(_)))).collect()
}

/// The boundary of a clipped reference in world coordinates.
fn world_boundary(s: &Session, h: Handle) -> Option<Vec<Vec2>> {
    let d = s.doc().ok()?;
    let EntityKind::Insert(i) = &d.entity(h)?.kind else { return None };
    let c = i.clip.as_ref()?;
    let m = block_xf(s, h)?;
    Some(c.boundary.iter().map(|p| m.apply(*p)).collect())
}

fn run_xclip(s: &mut Session, p: &Value) -> Result<Value> {
    let hs = inserts(s, &targets(s, p)?);
    if hs.is_empty() {
        return Err(bad("xclip", "select one or more block references"));
    }
    let op = str_param(p, "op").unwrap_or("new").trim().to_ascii_lowercase();
    match op.as_str() {
        "new" | "n" => {
            let pts = points_param(p, "boundary").ok_or_else(|| bad("xclip", "`boundary` [[x,y]...] is required"))?;
            let ring = polygon(&pts);
            if ring.len() < 3 || !ring.iter().all(|q| q.is_finite()) {
                return Err(bad("xclip", "the boundary needs two corners or three or more points"));
            }
            let invert = p.get("invert").and_then(Value::as_bool).unwrap_or(false);
            let mut n = 0;
            for h in &hs {
                let Some(inv) = block_xf(s, *h).and_then(|m| m.inverse()) else { continue };
                let boundary: Vec<Vec2> = ring.iter().map(|q| inv.apply(*q)).collect();
                s.doc_mut()?
                    .modify_entity(*h, |e| {
                        if let EntityKind::Insert(i) = &mut e.kind {
                            i.clip = Some(XClip { boundary: boundary.clone(), enabled: true, inverted: invert });
                        }
                    })
                    .map_err(|e| bad("xclip", e.to_string()))?;
                n += 1;
            }
            Ok(json!({ "clipped": n }))
        }
        "on" | "off" => {
            let on = op == "on";
            let mut n = 0;
            for h in &hs {
                s.doc_mut()?
                    .modify_entity(*h, |e| {
                        if let EntityKind::Insert(i) = &mut e.kind
                            && let Some(c) = &mut i.clip
                        {
                            c.enabled = on;
                            n += 1;
                        }
                    })
                    .map_err(|e| bad("xclip", e.to_string()))?;
            }
            Ok(json!({ "changed": n }))
        }
        "delete" | "d" => {
            let mut n = 0;
            for h in &hs {
                s.doc_mut()?
                    .modify_entity(*h, |e| {
                        if let EntityKind::Insert(i) = &mut e.kind
                            && i.clip.take().is_some()
                        {
                            n += 1;
                        }
                    })
                    .map_err(|e| bad("xclip", e.to_string()))?;
            }
            Ok(json!({ "deleted": n }))
        }
        "polyline" | "p" => {
            let mut made = Vec::new();
            for h in &hs {
                if let Some(b) = world_boundary(s, *h) {
                    let nh = s.add_entity(lwpoly(b.into_iter().map(PolyVertex::new).collect(), true))?;
                    made.push(nh.hex());
                }
            }
            if made.is_empty() {
                return Err(bad("xclip", "no clip boundary"));
            }
            Ok(json!({ "polylines": made }))
        }
        o => Err(bad("xclip", format!("unknown option \"{o}\""))),
    }
}

#[derive(Default, PartialEq)]
enum Stage {
    #[default]
    Select,
    Option,
    DeleteOld,
    Boundary,
    PickPolyline,
    Corner,
    Polygon,
}

#[derive(Default)]
struct XClipM {
    sel: SelectPhase,
    hs: Vec<Handle>,
    stage: Stage,
    invert: bool,
    pts: Vec<Vec2>,
}

impl XClipM {
    fn new_clip(&self, s: &mut Session, pts: &[Vec2]) -> Result<Step> {
        let hs: Vec<String> = self.hs.iter().map(|h| h.hex()).collect();
        let pts: Vec<[f64; 2]> = pts.iter().map(|p| [p.x, p.y]).collect();
        run_xclip(s, &json!({ "handles": hs, "op": "new", "boundary": pts, "invert": self.invert }))?;
        s.set_selection(Vec::new());
        Ok(Step::Done)
    }
    fn op(&self, s: &mut Session, op: &str) -> Result<Step> {
        let hs: Vec<String> = self.hs.iter().map(|h| h.hex()).collect();
        if let Err(e) = run_xclip(s, &json!({ "handles": hs, "op": op })) {
            s.echo(e.to_string());
        }
        s.set_selection(Vec::new());
        Ok(Step::Done)
    }
    fn any_clipped(&self, s: &Session) -> bool {
        let Ok(d) = s.doc() else { return false };
        self.hs.iter().any(|h| d.entity(*h).is_some_and(|e| matches!(&e.kind, EntityKind::Insert(i) if i.clip.is_some())))
    }
    fn after_select(&mut self, s: &mut Session, hs: Vec<Handle>) -> Step {
        self.hs = inserts(s, &hs);
        if self.hs.is_empty() {
            s.echo("No block references selected.");
            return Step::Done;
        }
        self.stage = Stage::Option;
        Step::Continue
    }
    fn start_new(&mut self, s: &Session) {
        self.stage = if self.any_clipped(s) { Stage::DeleteOld } else { Stage::Boundary };
    }
}

impl Interactive for XClipM {
    fn name(&self) -> &'static str {
        "XCLIP"
    }
    fn begin(&mut self, s: &mut Session) -> Result<Step> {
        self.sel = SelectPhase::begin(s);
        if self.sel.done {
            let hs = self.sel.picked.clone();
            return Ok(self.after_select(s, hs));
        }
        Ok(Step::Continue)
    }
    fn prompt(&self, _s: &Session) -> Prompt {
        match self.stage {
            Stage::Select => self.sel.prompt(),
            Stage::Option => Prompt::new("Enter clipping option", KW).kw(&["ON", "OFF", "Clipdepth", "Delete", "Polyline", "New"]).default("New"),
            Stage::DeleteOld => Prompt::new("Delete old boundary(s)?", KW).kw(&["Yes", "No"]).default("Yes"),
            Stage::Boundary => Prompt::new(
                if self.invert {
                    "Inside mode - Objects inside boundary will be hidden.\nSpecify clipping boundary or select invert option"
                } else {
                    "Outside mode - Objects outside boundary will be hidden.\nSpecify clipping boundary or select invert option"
                },
                Accept::POINT,
            )
            .kw(&["Select", "Polygonal", "Rectangular", "Invert"])
            .default("Rectangular"),
            Stage::PickPolyline => Prompt::new("Select polyline", Accept::SELECT),
            Stage::Corner if self.pts.is_empty() => Prompt::new("Specify first corner", Accept::POINT),
            Stage::Corner => Prompt::new("Specify opposite corner", Accept::POINT).base(self.pts.first().copied().unwrap_or_default()),
            Stage::Polygon if self.pts.is_empty() => Prompt::new("Specify first point", Accept::POINT),
            Stage::Polygon => Prompt::new("Specify next point or", Accept::POINT).kw(&["Undo"]).base(self.pts.last().copied().unwrap_or_default()),
        }
    }
    fn preview(&self, _s: &Session, cursor: Vec2) -> Vec<EntityKind> {
        match (&self.stage, self.pts.as_slice()) {
            (Stage::Corner, [a]) => vec![lwpoly(super::helpers::rect_vertices(*a, cursor), true)],
            (Stage::Polygon, [a]) => vec![line(*a, cursor)],
            (Stage::Polygon, pts) if pts.len() >= 2 => {
                let vs = pts.iter().copied().chain(std::iter::once(cursor)).map(PolyVertex::new).collect();
                vec![lwpoly(vs, true)]
            }
            _ => Vec::new(),
        }
    }
    fn input(&mut self, s: &mut Session, i: Input) -> Result<Step> {
        if matches!(i, Input::Cancel) {
            return Ok(Step::Done);
        }
        let kw = match &i {
            Input::Keyword(k) | Input::Text(k) => k.trim().to_ascii_lowercase(),
            _ => String::new(),
        };
        match self.stage {
            Stage::Select => match self.sel.feed(s, &i)? {
                SelOutcome::More => Ok(Step::Continue),
                SelOutcome::Empty => Ok(Step::Done),
                SelOutcome::Done(hs) => Ok(self.after_select(s, hs)),
            },
            Stage::Option => {
                let k = if matches!(i, Input::Enter) { "new boundary".to_string() } else { kw };
                match k.as_str() {
                    "on" => self.op(s, "on"),
                    "off" => self.op(s, "off"),
                    "d" | "delete" => self.op(s, "delete"),
                    "p" | "polyline" | "generate polyline" => self.op(s, "polyline"),
                    "c" | "clipdepth" => {
                        s.echo("Front and back clipping planes are not available yet.");
                        Ok(Step::Continue)
                    }
                    "n" | "new" | "new boundary" => {
                        self.start_new(s);
                        Ok(Step::Continue)
                    }
                    _ => Ok(Step::Continue),
                }
            }
            Stage::DeleteOld => match (&i, kw.as_str()) {
                (Input::Enter, _) | (_, "y" | "yes") => {
                    self.stage = Stage::Boundary;
                    Ok(Step::Continue)
                }
                (_, "n" | "no") => Ok(Step::Done),
                _ => Ok(Step::Continue),
            },
            Stage::Boundary => match (&i, kw.as_str()) {
                (Input::Point(p), _) => {
                    self.pts = vec![*p];
                    self.stage = Stage::Corner;
                    Ok(Step::Continue)
                }
                (Input::Enter, _) | (_, "r" | "rectangular") => {
                    self.stage = Stage::Corner;
                    Ok(Step::Continue)
                }
                (_, "p" | "polygonal") => {
                    self.stage = Stage::Polygon;
                    Ok(Step::Continue)
                }
                (_, "s" | "select polyline" | "select") => {
                    self.stage = Stage::PickPolyline;
                    Ok(Step::Continue)
                }
                (_, "i" | "invert clip" | "invert") => {
                    self.invert = !self.invert;
                    Ok(Step::Continue)
                }
                _ => Ok(Step::Continue),
            },
            Stage::PickPolyline => {
                let h = match &i {
                    Input::Pick(hs) => hs.first().copied(),
                    _ => None,
                };
                let Some(h) = h else { return Ok(Step::Continue) };
                let poly = s.doc().ok().and_then(|d| d.entity(h)).and_then(|e| cadcraft_render::clip::boundary_polygon(&e.kind));
                match poly {
                    Some(pts) if pts.len() >= 3 => self.new_clip(s, &pts),
                    _ => {
                        s.echo("Select a closed polyline (or another closed shape).");
                        Ok(Step::Continue)
                    }
                }
            }
            Stage::Corner => match i {
                Input::Point(p) if self.pts.is_empty() => {
                    self.pts.push(p);
                    Ok(Step::Continue)
                }
                Input::Point(p) => {
                    let a = self.pts.first().copied().unwrap_or(p);
                    if (a.x - p.x).abs() < 1e-12 || (a.y - p.y).abs() < 1e-12 {
                        s.echo("The rectangle has no area.");
                        return Ok(Step::Continue);
                    }
                    self.new_clip(s, &[a, p])
                }
                _ => Ok(Step::Continue),
            },
            Stage::Polygon => match (&i, kw.as_str()) {
                (Input::Point(p), _) => {
                    if self.pts.len() < MAX_POINTS {
                        self.pts.push(*p);
                    }
                    Ok(Step::Continue)
                }
                (_, "u" | "undo") => {
                    self.pts.pop();
                    Ok(Step::Continue)
                }
                (Input::Enter, _) if self.pts.len() >= 3 => {
                    let pts = self.pts.clone();
                    self.new_clip(s, &pts)
                }
                (Input::Enter, _) => {
                    s.echo("A polygon needs three or more points.");
                    Ok(Step::Continue)
                }
                _ => Ok(Step::Continue),
            },
        }
    }
}
