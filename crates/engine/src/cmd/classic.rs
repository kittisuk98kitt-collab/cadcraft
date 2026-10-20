//! Command-line forms AutoCAD users type: BREAKATPOINT's prompts, CHPROP / CHANGE, MODEL,
//! -STYLE, -DIMSTYLE and the classic (non-associative) -ARRAY.

use cadcraft_doc::Handle;
use cadcraft_geom::Vec2;
use serde_json::{Value, json};

use super::machines::{SelOutcome, SelectPhase, number};
use super::*;
use crate::{Accept, Input, Interactive, Prompt, Result, Session, Step};

/// Keywords (or Enter) only.
const KW: Accept = Accept { point: false, number: false, text: false, select: false, enter: true };

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("chprop", "Change Properties", run_chprop)
            .alias(&["change", "-change"])
            .params("{handles?, layer?, color?, linetype?, lineweight?, ltscale?, transparency?}")
            .interactive(|_| Ok(Box::new(ChPropM::default()))),
        CommandSpec::new("model", "Model", run_model).params("{} (the Model tab)").noundo(),
        CommandSpec::new("-style", "Text Style (command line)", run_style_line)
            .params("{name, font?, height?, widthFactor?, oblique?, backwards?, upsideDown?}")
            .interactive(|s| Ok(Box::new(StyleM::new(s)))),
        CommandSpec::new("-dimstyle", "Dimension Style (command line)", run_dimstyle_line)
            .params("{option: restore|save|status, name?}")
            .noundo()
            .interactive(|_| Ok(Box::new(DimStyleM::default()))),
        CommandSpec::new("-array", "Array (classic)", run_array_line)
            .params("{handles?, type: rectangular|polar, rows?, cols?, rowSpacing?, colSpacing?, center?, count?, angle?, rotate?}")
            .interactive(|_| Ok(Box::new(ArrayM::default()))),
    ]
}

/// BREAKATPOINT's prompts: the object, then the break point.
#[derive(Default)]
pub struct BreakAtM {
    obj: Option<Handle>,
}

impl Interactive for BreakAtM {
    fn name(&self) -> &'static str {
        "BREAKATPOINT"
    }
    fn begin(&mut self, s: &mut Session) -> Result<Step> {
        // One object already selected: straight to the point.
        if let [h] = s.selection().as_slice() {
            self.obj = Some(*h);
        }
        Ok(Step::Continue)
    }
    fn prompt(&self, _s: &Session) -> Prompt {
        match self.obj {
            None => Prompt::new("Select object", Accept::SELECT),
            Some(_) => Prompt::new("Specify break point", Accept::POINT),
        }
    }
    fn input(&mut self, s: &mut Session, i: Input) -> Result<Step> {
        match (self.obj, i) {
            (None, Input::Pick(hs)) => {
                self.obj = hs.first().copied();
                Ok(Step::Continue)
            }
            (None, Input::Point(p)) => {
                self.obj = super::curves::pick_at(s, p);
                if self.obj.is_none() {
                    s.echo("No object found.");
                }
                Ok(Step::Continue)
            }
            (Some(h), Input::Point(p)) => {
                let f = find_command("breakatpoint").ok_or_else(|| bad("breakatpoint", "missing"))?;
                if let Err(e) = (f.run)(s, &json!({ "handles": [h.hex()], "at": [p.x, p.y] })) {
                    s.echo(e.to_string());
                }
                s.set_selection(Vec::new());
                Ok(Step::Done)
            }
            (_, Input::Enter | Input::Cancel) => Ok(Step::Done),
            _ => Ok(Step::Continue),
        }
    }
}

fn run_chprop(s: &mut Session, p: &Value) -> Result<Value> {
    let f = find_command("properties.set").ok_or_else(|| bad("chprop", "properties.set is missing"))?;
    (f.run)(s, p)
}

fn run_model(s: &mut Session, _p: &Value) -> Result<Value> {
    let f = find_command("layout.set").ok_or_else(|| bad("model", "layout.set is missing"))?;
    (f.run)(s, &json!({ "name": "Model" }))
}

/// The property CHPROP is asking a value for.
#[derive(Clone, Copy, PartialEq)]
enum Prop {
    Color,
    Layer,
    LType,
    LtScale,
    LWeight,
    Transparency,
}

#[derive(Default)]
struct ChPropM {
    sel: SelectPhase,
    hs: Vec<Handle>,
    selected: bool,
    asking: Option<Prop>,
}

impl ChPropM {
    fn set(&self, s: &mut Session, key: &str, v: Value) {
        let hs: Vec<String> = self.hs.iter().map(|h| h.hex()).collect();
        let mut p = json!({ "handles": hs });
        p[key] = v;
        if let Err(e) = run_chprop(s, &p) {
            s.echo(e.to_string());
        }
    }
}

impl Interactive for ChPropM {
    fn name(&self) -> &'static str {
        "CHPROP"
    }
    fn begin(&mut self, s: &mut Session) -> Result<Step> {
        self.sel = SelectPhase::begin(s);
        if self.sel.done {
            self.hs = self.sel.picked.clone();
            self.selected = !self.hs.is_empty();
        }
        Ok(Step::Continue)
    }
    fn prompt(&self, _s: &Session) -> Prompt {
        if !self.selected {
            return self.sel.prompt();
        }
        match self.asking {
            None => Prompt::new("Enter property to change", KW).kw(&["Color", "LAyer", "LType", "ltScale", "LWeight", "TRansparency"]),
            Some(Prop::Color) => Prompt::new("Enter new color [Truecolor/COlorbook]", Accept::TEXT).default("BYLAYER"),
            Some(Prop::Layer) => Prompt::new("Enter new layer name", Accept::TEXT),
            Some(Prop::LType) => Prompt::new("Enter new linetype name", Accept::TEXT).default("ByLayer"),
            Some(Prop::LtScale) => Prompt::new("Specify new linetype scale", Accept::NUMBER).default("1.0000"),
            Some(Prop::LWeight) => Prompt::new("Enter new lineweight", Accept::TEXT).default("ByLayer"),
            Some(Prop::Transparency) => Prompt::new("Enter transparency value (0-90) or [ByLayer/ByBlock]", Accept::TEXT).default("ByLayer"),
        }
    }
    fn input(&mut self, s: &mut Session, i: Input) -> Result<Step> {
        if matches!(i, Input::Cancel) {
            return Ok(Step::Done);
        }
        if !self.selected {
            return match self.sel.feed(s, &i)? {
                SelOutcome::More => Ok(Step::Continue),
                SelOutcome::Empty => Ok(Step::Done),
                SelOutcome::Done(hs) => {
                    self.hs = hs;
                    self.selected = true;
                    Ok(Step::Continue)
                }
            };
        }
        let t = match &i {
            Input::Text(t) | Input::Keyword(t) => t.trim().to_string(),
            Input::Enter => String::new(),
            _ => return Ok(Step::Continue),
        };
        match self.asking.take() {
            None => {
                if t.is_empty() {
                    s.set_selection(Vec::new());
                    return Ok(Step::Done);
                }
                self.asking = match t.to_ascii_lowercase().as_str() {
                    "c" | "color" => Some(Prop::Color),
                    "la" | "layer" => Some(Prop::Layer),
                    "lt" | "ltype" => Some(Prop::LType),
                    "s" | "ltscale" => Some(Prop::LtScale),
                    "lw" | "lweight" => Some(Prop::LWeight),
                    "tr" | "transparency" => Some(Prop::Transparency),
                    _ => {
                        s.echo("Invalid option keyword.");
                        None
                    }
                };
            }
            Some(prop) => {
                let v = if t.is_empty() {
                    match prop {
                        Prop::Color | Prop::LType | Prop::LWeight | Prop::Transparency => "ByLayer".to_string(),
                        Prop::LtScale => "1".into(),
                        Prop::Layer => return Ok(Step::Continue),
                    }
                } else {
                    t
                };
                match prop {
                    Prop::Color => {
                        let val = v.parse::<i64>().map(|n| json!(n)).unwrap_or(json!(v));
                        self.set(s, "color", val);
                    }
                    Prop::Layer => {
                        if s.doc()?.layer(&v).is_none() {
                            s.echo(format!("Cannot find layer \"{v}\"."));
                        } else {
                            self.set(s, "layer", json!(v));
                        }
                    }
                    Prop::LType => self.set(s, "linetype", json!(v)),
                    Prop::LtScale => match number(&v).filter(|x| *x > 0.0) {
                        Some(x) => self.set(s, "ltscale", json!(x)),
                        None => s.echo("Requires a positive number."),
                    },
                    Prop::LWeight => {
                        let val = number(&v).map(|x| json!(x)).unwrap_or(json!(v));
                        self.set(s, "lineweight", val);
                    }
                    Prop::Transparency => self.set(s, "transparency", json!(v)),
                }
            }
        }
        Ok(Step::Continue)
    }
}

fn run_style_line(s: &mut Session, p: &Value) -> Result<Value> {
    let f = find_command("style").ok_or_else(|| bad("-style", "style is missing"))?;
    let mut p = p.clone();
    p["current"] = json!(true);
    (f.run)(s, &p)
}

/// -STYLE: name, font, height, width factor, oblique angle, backwards, upside-down.
struct StyleM {
    step: usize,
    v: Value,
}

impl StyleM {
    fn new(s: &Session) -> Self {
        let cur = s.doc().map(|d| d.header.str("TEXTSTYLE", "Standard")).unwrap_or_else(|_| "Standard".into());
        StyleM { step: 0, v: json!({ "name": cur }) }
    }
    fn existing(&self, s: &Session) -> Option<cadcraft_doc::TextStyle> {
        let n = self.v["name"].as_str()?;
        s.doc().ok()?.text_style(n).cloned()
    }
}

impl Interactive for StyleM {
    fn name(&self) -> &'static str {
        "-STYLE"
    }
    fn prompt(&self, s: &Session) -> Prompt {
        let st = self.existing(s);
        let yn = |b: bool| if b { "Yes" } else { "No" };
        match self.step {
            0 => Prompt::new("Enter name of text style", Accept::TEXT).default(self.v["name"].as_str().unwrap_or("Standard").to_string()),
            1 => Prompt::new("Specify full font name or font filename (TTF or SHX)", Accept::TEXT).default(st.map_or("txt".into(), |t| t.font)),
            2 => Prompt::new("Specify height of text", Accept::NUMBER).default(format!("{:.4}", st.map_or(0.0, |t| t.height))),
            3 => Prompt::new("Specify width factor", Accept::NUMBER).default(format!("{:.4}", st.map_or(1.0, |t| t.width_factor))),
            4 => Prompt::new("Specify obliquing angle", Accept::NUMBER).default(format!("{}", st.map_or(0.0, |t| t.oblique.to_degrees()))),
            5 => Prompt::new("Display text backwards?", KW).kw(&["Yes", "No"]).default(yn(st.is_some_and(|t| t.backwards))),
            _ => Prompt::new("Display text upside-down?", KW).kw(&["Yes", "No"]).default(yn(st.is_some_and(|t| t.upside_down))),
        }
    }
    fn input(&mut self, s: &mut Session, i: Input) -> Result<Step> {
        let t = match i {
            Input::Text(t) | Input::Keyword(t) => t.trim().to_string(),
            Input::Enter => String::new(),
            Input::Cancel => return Ok(Step::Done),
            _ => return Ok(Step::Continue),
        };
        let st = self.existing(s);
        match self.step {
            0 => {
                if !t.is_empty() {
                    self.v["name"] = json!(t);
                }
            }
            1 => self.v["font"] = json!(if t.is_empty() { st.map_or("txt".into(), |x| x.font) } else { t }),
            2 => self.v["height"] = json!(number(&t).filter(|x| *x >= 0.0).unwrap_or(st.map_or(0.0, |x| x.height))),
            3 => self.v["widthFactor"] = json!(number(&t).filter(|x| *x > 0.0).unwrap_or(st.map_or(1.0, |x| x.width_factor))),
            4 => self.v["oblique"] = json!(number(&t).filter(|x| x.abs() < 85.0).unwrap_or(st.map_or(0.0, |x| x.oblique.to_degrees()))),
            5 => self.v["backwards"] = json!(if t.is_empty() { st.is_some_and(|x| x.backwards) } else { t.to_ascii_lowercase().starts_with('y') }),
            _ => {
                self.v["upsideDown"] = json!(if t.is_empty() { st.is_some_and(|x| x.upside_down) } else { t.to_ascii_lowercase().starts_with('y') });
                match run_style_line(s, &self.v) {
                    Ok(_) => s.echo(format!("\"{}\" is now the current text style.", self.v["name"].as_str().unwrap_or(""))),
                    Err(e) => s.echo(e.to_string()),
                }
                return Ok(Step::Done);
            }
        }
        self.step += 1;
        Ok(Step::Continue)
    }
}

fn run_dimstyle_line(s: &mut Session, p: &Value) -> Result<Value> {
    let opt = str_param(p, "option").unwrap_or("restore").to_ascii_lowercase();
    let f = find_command("dimstyle").ok_or_else(|| bad("-dimstyle", "dimstyle is missing"))?;
    match opt.as_str() {
        "restore" | "r" | "save" | "s" => {
            let name = str_param(p, "name").ok_or_else(|| bad("-dimstyle", "`name` is required"))?;
            let save = opt.starts_with('s');
            let exists = s.doc()?.dim_style(name).is_some();
            if !save && !exists {
                return Err(bad("-dimstyle", format!("Dimension style \"{name}\" not found.")));
            }
            if save && !exists {
                // A new style takes the current one's settings.
                let cur = s.doc()?.header.str("DIMSTYLE", "Standard");
                let base = s.doc()?.dim_style(&cur).cloned().unwrap_or_default();
                let mut v = serde_json::to_value(&base).unwrap_or(json!({}));
                v["name"] = json!(name);
                v["current"] = json!(true);
                return (f.run)(s, &v);
            }
            (f.run)(s, &json!({ "name": name, "current": true }))
        }
        _ => {
            let cur = s.doc()?.header.str("DIMSTYLE", "Standard");
            Ok(json!({ "current": cur }))
        }
    }
}

#[derive(Default)]
struct DimStyleM {
    /// The option chosen: 'R' restore, 'S' save.
    opt: Option<char>,
}

impl Interactive for DimStyleM {
    fn name(&self) -> &'static str {
        "-DIMSTYLE"
    }
    fn begin(&mut self, s: &mut Session) -> Result<Step> {
        let cur = s.doc()?.header.str("DIMSTYLE", "Standard");
        s.echo(format!("Current dimension style: {cur}  Annotative: No"));
        Ok(Step::Continue)
    }
    fn prompt(&self, _s: &Session) -> Prompt {
        match self.opt {
            None => Prompt::new("Enter a dimension style option", KW).kw(&["Save", "Restore", "STatus", "?"]).default("Restore"),
            Some('S') => Prompt::new("Enter name for new dimension style or [?]", Accept::TEXT),
            Some(_) => Prompt::new("Enter a dimension style name, [?]", Accept::TEXT),
        }
    }
    fn input(&mut self, s: &mut Session, i: Input) -> Result<Step> {
        let t = match i {
            Input::Text(t) | Input::Keyword(t) => t.trim().to_string(),
            Input::Enter => String::new(),
            Input::Cancel => return Ok(Step::Done),
            _ => return Ok(Step::Continue),
        };
        let list = |s: &mut Session| {
            let names: Vec<String> = s.doc().map(|d| d.dim_styles.iter().map(|d| d.name.clone()).collect()).unwrap_or_default();
            s.echo(format!("Named dimension styles: {}", names.join(", ")));
        };
        match self.opt {
            None => match t.to_ascii_lowercase().as_str() {
                "" | "r" | "restore" => self.opt = Some('R'),
                "s" | "save" => self.opt = Some('S'),
                "st" | "status" => {
                    let cur = s.doc()?.header.str("DIMSTYLE", "Standard");
                    s.echo(format!("DIMSTYLE  {cur}  Current dimension style"));
                    return Ok(Step::Done);
                }
                "?" => {
                    list(s);
                    return Ok(Step::Done);
                }
                _ => s.echo("Invalid option keyword."),
            },
            Some(o) => {
                if t == "?" {
                    list(s);
                    return Ok(Step::Continue);
                }
                if t.is_empty() {
                    return Ok(Step::Done);
                }
                let opt = if o == 'S' { "save" } else { "restore" };
                match run_dimstyle_line(s, &json!({ "option": opt, "name": t })) {
                    Ok(_) => s.echo(format!("Current dimension style: {t}")),
                    Err(e) => {
                        s.echo(e.to_string());
                        return Ok(Step::Continue);
                    }
                }
                return Ok(Step::Done);
            }
        }
        Ok(Step::Continue)
    }
}

fn run_array_line(s: &mut Session, p: &Value) -> Result<Value> {
    let polar = str_param(p, "type").is_some_and(|t| t.to_ascii_lowercase().starts_with('p'));
    let mut q = p.clone();
    q["associative"] = json!(false);
    let f = find_command(if polar { "arraypolar" } else { "arrayrect" }).ok_or_else(|| bad("-array", "array is missing"))?;
    (f.run)(s, &q)
}

/// The classic -ARRAY: rectangular (rows, columns, distances) or polar (centre, count, angle,
/// rotate), never associative.
#[derive(Default)]
struct ArrayM {
    sel: SelectPhase,
    hs: Vec<Handle>,
    selected: bool,
    polar: Option<bool>,
    step: usize,
    v: Vec<f64>,
    center: Option<Vec2>,
}

impl ArrayM {
    fn go(&self, s: &mut Session, p: Value) -> Result<Step> {
        if let Err(e) = run_array_line(s, &p) {
            s.echo(e.to_string());
        }
        s.set_selection(Vec::new());
        Ok(Step::Done)
    }
    fn handles(&self) -> Vec<String> {
        self.hs.iter().map(|h| h.hex()).collect()
    }
}

impl Interactive for ArrayM {
    fn name(&self) -> &'static str {
        "-ARRAY"
    }
    fn begin(&mut self, s: &mut Session) -> Result<Step> {
        self.sel = SelectPhase::begin(s);
        if self.sel.done {
            self.hs = self.sel.picked.clone();
            self.selected = !self.hs.is_empty();
        }
        Ok(Step::Continue)
    }
    fn prompt(&self, _s: &Session) -> Prompt {
        if !self.selected {
            return self.sel.prompt();
        }
        match (self.polar, self.step) {
            (None, _) => Prompt::new("Enter the type of array", KW).kw(&["Rectangular", "Polar"]).default("R"),
            (Some(false), 0) => Prompt::new("Enter the number of rows (---)", Accept::NUMBER).default("1"),
            (Some(false), 1) => Prompt::new("Enter the number of columns (|||)", Accept::NUMBER).default("1"),
            (Some(false), 2) => Prompt::new("Enter the distance between rows or specify unit cell (---)", Accept::NUMBER),
            (Some(false), _) => Prompt::new("Specify the distance between columns (|||)", Accept::NUMBER),
            (Some(true), 0) => Prompt::new("Specify center point of array", Accept::POINT),
            (Some(true), 1) => Prompt::new("Enter the number of items in the array", Accept::NUMBER),
            (Some(true), 2) => Prompt::new("Specify the angle to fill (+=ccw, -=cw)", Accept::NUMBER).default("360"),
            (Some(true), _) => Prompt::new("Rotate arrayed objects?", KW).kw(&["Yes", "No"]).default("Y"),
        }
    }
    fn input(&mut self, s: &mut Session, i: Input) -> Result<Step> {
        if matches!(i, Input::Cancel) {
            return Ok(Step::Done);
        }
        if !self.selected {
            return match self.sel.feed(s, &i)? {
                SelOutcome::More => Ok(Step::Continue),
                SelOutcome::Empty => Ok(Step::Done),
                SelOutcome::Done(hs) => {
                    self.hs = hs;
                    self.selected = true;
                    Ok(Step::Continue)
                }
            };
        }
        let t = match &i {
            Input::Text(t) | Input::Keyword(t) => t.trim().to_ascii_lowercase(),
            Input::Enter => String::new(),
            Input::Point(_) => String::new(),
            _ => return Ok(Step::Continue),
        };
        let Some(polar) = self.polar else {
            match t.as_str() {
                "" | "r" | "rectangular" => self.polar = Some(false),
                "p" | "polar" => self.polar = Some(true),
                _ => s.echo("Invalid option keyword."),
            }
            return Ok(Step::Continue);
        };
        if polar {
            match self.step {
                0 => match i {
                    Input::Point(p) => self.center = Some(p),
                    _ => return Ok(Step::Continue),
                },
                1 => match number(&t).filter(|n| *n >= 2.0 && *n <= 100_000.0) {
                    Some(n) => self.v.push(n),
                    None => {
                        s.echo("Requires an integer between 2 and 100000.");
                        return Ok(Step::Continue);
                    }
                },
                2 => self.v.push(if t.is_empty() { 360.0 } else { number(&t).unwrap_or(360.0) }),
                _ => {
                    let rotate = !t.starts_with('n');
                    let c = self.center.unwrap_or_default();
                    let p = json!({ "handles": self.handles(), "type": "polar", "center": [c.x, c.y],
                        "count": self.v.first().copied().unwrap_or(2.0).round() as i64, "angle": self.v.get(1).copied().unwrap_or(360.0), "rotate": rotate });
                    return self.go(s, p);
                }
            }
        } else {
            let n = if t.is_empty() && self.step < 2 { Some(1.0) } else { number(&t) };
            let Some(n) = n.filter(|n| n.is_finite()) else {
                s.echo("Requires a number.");
                return Ok(Step::Continue);
            };
            if self.step < 2 && !(1.0..=100_000.0).contains(&n) {
                s.echo("Requires an integer between 1 and 100000.");
                return Ok(Step::Continue);
            }
            self.v.push(n);
            let rows = self.v.first().copied().unwrap_or(1.0).round() as i64;
            let cols = self.v.get(1).copied().unwrap_or(1.0).round() as i64;
            // Distances only for the directions with more than one item.
            let done = match self.step {
                1 if rows == 1 && cols == 1 => {
                    s.echo("One-element array, nothing to do.");
                    return Ok(Step::Done);
                }
                2 if cols == 1 => true,
                3 => true,
                _ => false,
            };
            if self.step == 1 && rows == 1 {
                // No row distance to ask.
                self.v.push(0.0);
                self.step += 1;
            }
            if done || (self.step == 2 && cols == 1) {
                let p = json!({ "handles": self.handles(), "type": "rectangular", "rows": rows, "cols": cols,
                    "rowSpacing": self.v.get(2).copied().unwrap_or(0.0), "colSpacing": self.v.get(3).copied().unwrap_or(0.0) });
                return self.go(s, p);
            }
        }
        self.step += 1;
        Ok(Step::Continue)
    }
}
