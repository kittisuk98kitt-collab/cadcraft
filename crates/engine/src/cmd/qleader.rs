//! QLEADER: a leader drawn by its Leader Settings (annotation type, MText options, reuse, line
//! type, arrowhead, number of points, angle constraints, text attachment). Its Settings option
//! asks the UI for the Leader Settings dialog; `qleader.settings` changes them (the dialog's OK).

use cadcraft_doc::{EntityKind, Handle};
use cadcraft_geom::{Mat3, Vec2};
use serde_json::{Value, json};

use super::tolerance::ToleranceM;
use super::*;
use crate::{Accept, Input, Interactive, Prompt, QAnnotation, QLeaderSettings, QReuse, Result, Session, Step};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("qleader", "Quick Leader", run_qleader)
            .alias(&["le"])
            .params("{points: [[x,y]...], text?: string | [lines], width?, block?: name, copy?: handle; the rest from the Leader Settings}")
            .interactive(|s| Ok(Box::new(QLeaderM::new(s)))),
        CommandSpec::new("qleader.settings", "Leader Settings", run_settings)
            .params("{annotation?: mtext|copy|tolerance|block|none, promptWidth?, leftJustify?, frame?, reuse?: none|next|current, spline?, arrow?: name (\"\" = dimension style), maxPoints? (0 = no limit), angle1?/angle2?: 0 any | -1 horizontal | 90|45|30|15, attachLeft?/attachRight?: 0..4, underline?} (no params: the current settings)")
            .noundo(),
    ]
}

/// Merge the given fields into the Leader Settings and return them.
fn run_settings(s: &mut Session, p: &Value) -> Result<Value> {
    let mut cur = serde_json::to_value(&s.qleader).map_err(|e| bad("qleader.settings", e.to_string()))?;
    if let (Some(c), Some(o)) = (cur.as_object_mut(), p.as_object()) {
        for (k, v) in o {
            if c.contains_key(k) {
                c.insert(k.clone(), v.clone());
            }
        }
    }
    let mut next: QLeaderSettings = serde_json::from_value(cur).map_err(|e| bad("qleader.settings", e.to_string()))?;
    next.attach_left = next.attach_left.min(4);
    next.attach_right = next.attach_right.min(4);
    if !matches!(next.angle1, -1 | 0 | 15 | 30 | 45 | 90) {
        next.angle1 = 0;
    }
    if !matches!(next.angle2, -1 | 0 | 15 | 30 | 45 | 90) {
        next.angle2 = 0;
    }
    // Reuse Current only has something to reuse after a leader made with Reuse Next.
    if next.reuse == QReuse::Current && s.qleader_reuse.is_none() {
        next.reuse = QReuse::None;
    }
    if next.reuse == QReuse::None {
        s.qleader_reuse = None;
    }
    s.qleader = next;
    serde_json::to_value(&s.qleader).map_err(|e| bad("qleader.settings", e.to_string()))
}

/// The leader parameters the settings give (everything but points and annotation).
fn leader_params(st: &QLeaderSettings, pts: &[Vec2]) -> Value {
    json!({
        "points": pts.iter().map(|p| [p.x, p.y]).collect::<Vec<_>>(),
        "spline": st.spline,
        "arrow": st.arrow,
        "attachLeft": st.attach_left,
        "attachRight": st.attach_right,
        "underline": st.underline,
        "frame": st.frame,
        "leftJustify": st.left_justify,
    })
}

fn run_qleader(s: &mut Session, p: &Value) -> Result<Value> {
    let st = s.qleader.clone();
    let pts = points_param(p, "points").ok_or_else(|| bad("qleader", "`points` is required"))?;
    let mut lp = leader_params(&st, &pts);
    if let Some(t) = p.get("text") {
        lp["text"] = t.clone();
    }
    if let Some(w) = p.get("width") {
        lp["width"] = w.clone();
    }
    let block = p.get("block").and_then(Value::as_str).map(str::to_string);
    let copy = p.get("copy").and_then(super::curves::handle_value);
    lp["annotated"] = json!(block.is_some() || copy.is_some());
    let r = super::annotate::run_leader(s, &lp)?;
    let (end, right) = leader_end(&r);
    let mut out: Vec<Value> = r.get("handles").and_then(Value::as_array).cloned().unwrap_or_default();
    if let Some(b) = block {
        out.extend(place_block(s, &b, end)?);
    }
    if let Some(h) = copy {
        out.push(json!(copy_to(s, h, end, right)?.hex()));
    }
    Ok(json!({ "handles": out }))
}

fn leader_end(r: &Value) -> (Vec2, bool) {
    let e = r.get("end").and_then(Value::as_array);
    let c = |i: usize| e.and_then(|a| a.get(i)).and_then(Value::as_f64).unwrap_or(0.0);
    (Vec2::new(c(0), c(1)), r.get("right").and_then(Value::as_bool).unwrap_or(true))
}

fn place_block(s: &mut Session, name: &str, at: Vec2) -> Result<Vec<Value>> {
    let ins = find_command("insert").ok_or_else(|| bad("qleader", "INSERT is not available"))?;
    let r = (ins.run)(s, &json!({ "name": name, "at": [at.x, at.y] }))?;
    Ok(match r.get("handle").or_else(|| r.get("handles")) {
        Some(Value::Array(a)) => a.clone(),
        Some(v) => vec![v.clone()],
        None => Vec::new(),
    })
}

/// Copy an Object: a copy of text, multiline text, a tolerance or a block reference at the end.
fn copy_to(s: &mut Session, h: Handle, end: Vec2, right: bool) -> Result<Handle> {
    let e = s.doc()?.entity(h).cloned().ok_or_else(|| bad("qleader", format!("no object {}", h.hex())))?;
    let anchor = match &e.kind {
        EntityKind::MText(t) => t.insert.xy(),
        EntityKind::Text(t) => t.insert.xy(),
        EntityKind::Tolerance(t) => t.insert.xy(),
        EntityKind::Insert(i) => i.insert.xy(),
        _ => return Err(bad("qleader", "copy text, multiline text, a tolerance or a block reference")),
    };
    let mut kind = e.kind.clone();
    // Multiline text keeps its attachment; text on the left of the end hangs off its right.
    if let EntityKind::MText(t) = &mut kind {
        let row = (t.attach.max(1) - 1) / 3;
        t.attach = row * 3 + if right { 1 } else { 3 };
    }
    kind.transform(&Mat3::translate(end - anchor));
    let n = s.add_entity(kind)?;
    let common = e.common.clone();
    s.doc_mut()?.modify_entity(n, |x| x.common = common).map_err(|e| bad("qleader", e.to_string()))?;
    Ok(n)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    First,
    Points,
    Width,
    Lines,
    CopyPick,
    BlockName,
    Tolerance,
}

struct QLeaderM {
    st: QLeaderSettings,
    stage: Stage,
    pts: Vec<Vec2>,
    width: f64,
    lines: Vec<String>,
    tol: Option<ToleranceM>,
    last_block: String,
}

impl QLeaderM {
    fn new(s: &Session) -> Self {
        QLeaderM {
            st: s.qleader.clone(),
            stage: Stage::First,
            pts: Vec::new(),
            width: 0.0,
            lines: Vec::new(),
            tol: None,
            last_block: s.last_used.mleader_block.clone(),
        }
    }

    /// The angle constraint of the segment that the next point ends.
    fn constrained(&self, p: Vec2) -> Vec2 {
        let Some(&prev) = self.pts.last() else { return p };
        let c = match self.pts.len() {
            1 => self.st.angle1,
            2 => self.st.angle2,
            _ => 0,
        };
        let len = prev.dist(p);
        if c == 0 || len <= 1e-12 {
            return p;
        }
        if c == -1 {
            return Vec2::new(p.x, prev.y);
        }
        let inc = f64::from(c).to_radians();
        Vec2::polar(prev, len, ((p - prev).angle() / inc).round() * inc)
    }

    /// All leader points are in: on to the annotation.
    fn points_done(&mut self, s: &mut Session) -> Result<Step> {
        if self.st.reuse == QReuse::Current
            && let Some(t) = s.qleader_reuse.clone()
        {
            self.lines = t.lines().map(str::to_string).collect();
            return self.finish(s, None, None);
        }
        self.stage = match self.st.annotation {
            QAnnotation::Mtext if self.st.prompt_width => Stage::Width,
            QAnnotation::Mtext => Stage::Lines,
            QAnnotation::Copy => Stage::CopyPick,
            QAnnotation::Block => Stage::BlockName,
            QAnnotation::Tolerance => {
                self.tol = Some(ToleranceM::new(s));
                Stage::Tolerance
            }
            QAnnotation::None => return self.finish(s, None, None),
        };
        Ok(Step::Continue)
    }

    fn finish(&mut self, s: &mut Session, block: Option<String>, copy: Option<Handle>) -> Result<Step> {
        let mut lp = leader_params(&self.st, &self.pts);
        if !self.lines.is_empty() {
            lp["text"] = json!(self.lines);
            lp["width"] = json!(self.width);
        }
        lp["annotated"] = json!(block.is_some() || copy.is_some() || self.tol.is_some());
        let r = super::annotate::run_leader(s, &lp)?;
        let (end, right) = leader_end(&r);
        if let Some(b) = block {
            place_block(s, &b, end)?;
            s.last_used.mleader_block = b;
        }
        if let Some(h) = copy {
            copy_to(s, h, end, right)?;
        }
        if let Some(t) = &self.tol {
            let gap = r.get("gap").and_then(Value::as_f64).unwrap_or(0.0);
            let at = if right { end + Vec2::new(gap, 0.0) } else { end - Vec2::new(gap, 0.0) };
            t.place_at(s, at, right)?;
        }
        // Reuse Next: this annotation is reused by the leaders that follow.
        if self.st.reuse == QReuse::Next && !self.lines.is_empty() {
            s.qleader_reuse = Some(self.lines.join("\n"));
            s.qleader.reuse = QReuse::Current;
        }
        Ok(Step::Done)
    }
}

impl Interactive for QLeaderM {
    fn name(&self) -> &'static str {
        "QLEADER"
    }
    fn prompt(&self, s: &Session) -> Prompt {
        if let (Stage::Tolerance, Some(t)) = (self.stage, &self.tol) {
            return t.prompt(s);
        }
        match self.stage {
            Stage::First => Prompt::new("Specify first leader point, or", Accept::POINT).kw(&["Settings"]).default("Settings"),
            Stage::Points => Prompt::new("Specify next point", Accept::POINT).base_opt(self.pts.last().copied()),
            Stage::Width => Prompt::new("Specify text width", Accept { point: true, number: true, text: false, select: false, enter: true })
                .default(format!("{}", self.width))
                .base_opt(self.pts.last().copied()),
            Stage::Lines if self.lines.is_empty() => Prompt::new("Enter first line of annotation text", Accept::TEXT).default("Mtext"),
            Stage::Lines => Prompt::new("Enter next line of annotation text", Accept::TEXT),
            Stage::CopyPick => Prompt::new("Select an object to copy", Accept::POINT),
            Stage::BlockName => {
                let p = Prompt::new("Enter block name", Accept::TEXT);
                if self.last_block.is_empty() { p } else { p.default(self.last_block.clone()) }
            }
            Stage::Tolerance => Prompt::new("Enter geometric characteristic", Accept::TEXT),
        }
    }
    fn input(&mut self, s: &mut Session, i: Input) -> Result<Step> {
        if i == Input::Cancel {
            s.qleader_dialog = false;
            return Ok(Step::Done);
        }
        match self.stage {
            Stage::First => match i {
                Input::Point(p) => {
                    // The dialog may have changed the settings since the command started.
                    self.st = s.qleader.clone();
                    self.pts.push(p);
                    s.last_point = p;
                    self.stage = Stage::Points;
                    Ok(Step::Continue)
                }
                Input::Enter | Input::Keyword(_) | Input::Text(_) => {
                    s.qleader_dialog = true;
                    Ok(Step::Continue)
                }
                _ => Ok(Step::Continue),
            },
            Stage::Points => match i {
                Input::Point(p) => {
                    let p = self.constrained(p);
                    if self.pts.last().is_some_and(|l| l.near(p, 1e-12)) {
                        return Ok(Step::Continue);
                    }
                    self.pts.push(p);
                    s.last_point = p;
                    if self.st.max_points >= 2 && self.pts.len() >= self.st.max_points as usize {
                        return self.points_done(s);
                    }
                    Ok(Step::Continue)
                }
                Input::Enter if self.pts.len() >= 2 => self.points_done(s),
                Input::Enter => Ok(Step::Continue),
                _ => Ok(Step::Continue),
            },
            Stage::Width => {
                match i {
                    Input::Point(p) => self.width = self.pts.last().map_or(0.0, |l| l.dist(p)),
                    Input::Text(t) | Input::Keyword(t) => match t.trim().parse::<f64>() {
                        Ok(w) if w.is_finite() && w >= 0.0 => self.width = w,
                        _ => {
                            s.echo("Requires a distance.");
                            return Ok(Step::Continue);
                        }
                    },
                    Input::Enter => {}
                    _ => return Ok(Step::Continue),
                }
                self.stage = Stage::Lines;
                Ok(Step::Continue)
            }
            Stage::Lines => match i {
                Input::Text(t) | Input::Keyword(t) if !t.trim().is_empty() => {
                    self.lines.push(t);
                    Ok(Step::Continue)
                }
                Input::Text(_) | Input::Keyword(_) | Input::Enter => self.finish(s, None, None),
                _ => Ok(Step::Continue),
            },
            Stage::CopyPick => match i {
                Input::Point(p) => match super::curves::pick_at(s, p) {
                    Some(h) => self.finish(s, None, Some(h)),
                    None => {
                        s.echo("No object found.");
                        Ok(Step::Continue)
                    }
                },
                Input::Pick(hs) => match hs.first() {
                    Some(h) => self.finish(s, None, Some(*h)),
                    None => Ok(Step::Continue),
                },
                Input::Enter => self.finish(s, None, None),
                _ => Ok(Step::Continue),
            },
            Stage::BlockName => {
                let name = match i {
                    Input::Text(t) | Input::Keyword(t) if !t.trim().is_empty() => t.trim().to_string(),
                    Input::Enter | Input::Text(_) | Input::Keyword(_) => self.last_block.clone(),
                    _ => return Ok(Step::Continue),
                };
                if name.is_empty() || s.doc()?.block(&name).is_none() {
                    s.echo(format!("Block \"{name}\" not found."));
                    return Ok(Step::Continue);
                }
                self.finish(s, Some(name), None)
            }
            Stage::Tolerance => {
                let Some(t) = self.tol.as_mut() else { return Ok(Step::Done) };
                match t.input(s, i)? {
                    Step::Continue if t.wants_location() => self.finish(s, None, None),
                    Step::Continue => Ok(Step::Continue),
                    // An empty frame: the leader alone.
                    _ => {
                        self.tol = None;
                        self.finish(s, None, None)
                    }
                }
            }
        }
    }
    fn preview(&self, _s: &Session, c: Vec2) -> Vec<EntityKind> {
        if self.stage != Stage::Points {
            return Vec::new();
        }
        let c = self.constrained(c);
        let mut pts = self.pts.clone();
        pts.push(c);
        let path = cadcraft_doc::Leader {
            vertices: pts.iter().map(|p| super::helpers::v3(*p)).collect(),
            arrow: true,
            spline: self.st.spline,
            style: String::new(),
            arrow_block: None,
        }
        .path();
        path.windows(2).map(|w| super::helpers::line(w[0], w[1])).collect()
    }
}
