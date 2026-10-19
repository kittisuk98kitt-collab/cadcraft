//! Manage and Insert tab tools: BLOCKREPLACE (every reference of one block becomes another),
//! TXT2MTXT (texts combined into one multiline text) and LAYTRANS (layers translated to others,
//! their objects moved and the old layers removed).

use cadcraft_doc::{EntityKind, EntityStore, Handle, MText};
use serde_json::{Map, Value, json};

use super::machines::SelectRun;
use super::*;
use crate::{Accept, Input, Interactive, Prompt, Result, Session, Step};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("blockreplace", "Replace Block", run_blockreplace)
            .menu(&["Insert", "Replace Block"])
            .params("{old: block name, new: block name}")
            .interactive(|_| Ok(Box::new(BlockReplaceM::default()))),
        CommandSpec::new("txt2mtxt", "Combine Text", run_txt2mtxt)
            .menu(&["Modify", "Object", "Text", "Combine Text"])
            .alias(&["t2mt"])
            .params("{handles?: [hex] (texts and multiline texts; default the selection)}")
            .interactive(|_| Ok(Box::new(SelectRun::new("txt2mtxt", "TXT2MTXT")))),
        CommandSpec::new("laytrans", "Layer Translator", run_laytrans)
            .menu(&["Tools", "CAD Standards", "Layer Translator"])
            .params("{map: {from layer: to layer}} (a missing `to` layer is created with the `from` layer's properties)")
            .interactive(|_| Ok(Box::new(LayTransM::default()))),
    ]
}

fn block_name(s: &Session, cmd: &str, n: &str) -> Result<String> {
    let d = s.doc()?;
    d.blocks.keys().find(|k| k.eq_ignore_ascii_case(n.trim())).cloned().ok_or_else(|| bad(cmd, format!("no block \"{}\"", n.trim())))
}

/// References to `old` in `store` become references to `new`; attributes without a definition
/// in `new` are dropped. The count changed.
fn replace_in(store: &mut EntityStore, old: &str, new: &str, tags: &[String]) -> usize {
    let hs: Vec<Handle> =
        store.iter().filter(|e| matches!(&e.kind, EntityKind::Insert(i) if i.block.eq_ignore_ascii_case(old))).map(|e| e.handle).collect();
    for h in &hs {
        store.modify(*h, |e| {
            if let EntityKind::Insert(i) = &mut e.kind {
                i.block = new.to_string();
                i.attribs.retain(|a| tags.iter().any(|t| t.eq_ignore_ascii_case(&a.tag)));
            }
        });
    }
    hs.len()
}

fn run_blockreplace(s: &mut Session, p: &Value) -> Result<Value> {
    let old = block_name(s, "blockreplace", str_param(p, "old").ok_or_else(|| bad("blockreplace", "`old` is required"))?)?;
    let new = block_name(s, "blockreplace", str_param(p, "new").ok_or_else(|| bad("blockreplace", "`new` is required"))?)?;
    if old == new {
        return Err(bad("blockreplace", "the blocks are the same"));
    }
    let d = s.doc_mut()?;
    let tags: Vec<String> = d
        .block(&new)
        .map(|b| b.entities.iter().filter_map(|e| if let EntityKind::AttDef(a) = &e.kind { Some(a.tag.clone()) } else { None }).collect())
        .unwrap_or_default();
    let mut n = replace_in(&mut d.model, &old, &new, &tags);
    for l in &mut d.layouts {
        n += replace_in(&mut l.entities, &old, &new, &tags);
    }
    // References nested in other blocks (not in `new` itself: no block may contain itself).
    let names: Vec<String> = d.blocks.keys().filter(|k| !k.eq_ignore_ascii_case(&new)).cloned().collect();
    for name in names {
        let Some(b) = d.blocks.get(&name) else { continue };
        if !b.entities.iter().any(|e| matches!(&e.kind, EntityKind::Insert(i) if i.block.eq_ignore_ascii_case(&old))) {
            continue;
        }
        let mut nb = (**b).clone();
        n += replace_in(&mut nb.entities, &old, &new, &tags);
        d.blocks.insert(name, std::sync::Arc::new(nb));
    }
    let msg = format!("{n} reference(s) of \"{old}\" replaced with \"{new}\".");
    s.echo(msg.clone());
    Ok(json!({ "replaced": n }))
}

#[derive(Default)]
struct BlockReplaceM {
    old: Option<String>,
}

impl Interactive for BlockReplaceM {
    fn name(&self) -> &'static str {
        "BLOCKREPLACE"
    }
    fn prompt(&self, _s: &Session) -> Prompt {
        match &self.old {
            None => Prompt::new("Enter name of block to be replaced", Accept::TEXT),
            Some(o) => Prompt::new(format!("Enter name of block to replace \"{o}\" with"), Accept::TEXT),
        }
    }
    fn input(&mut self, s: &mut Session, i: Input) -> Result<Step> {
        let t = match i {
            Input::Text(t) | Input::Keyword(t) if !t.trim().is_empty() => t.trim().to_string(),
            Input::Enter | Input::Cancel => return Ok(Step::Done),
            _ => return Ok(Step::Continue),
        };
        let name = match block_name(s, "blockreplace", &t) {
            Ok(n) => n,
            Err(e) => {
                s.echo(e.to_string());
                return Ok(Step::Continue);
            }
        };
        match self.old.take() {
            None => {
                self.old = Some(name);
                Ok(Step::Continue)
            }
            Some(old) => {
                run_blockreplace(s, &json!({ "old": old, "new": name }))?;
                Ok(Step::Done)
            }
        }
    }
}

fn run_txt2mtxt(s: &mut Session, p: &Value) -> Result<Value> {
    let hs = targets(s, p)?;
    let d = s.doc()?;
    // (top, left, height, contents, entity) of each text.
    let mut parts = Vec::new();
    for h in &hs {
        let Some(e) = d.entity(*h) else { continue };
        match &e.kind {
            EntityKind::Text(t) => parts.push((t.insert.y + t.height, t.insert.x, t.height, t.style.clone(), t.value.clone(), e.clone())),
            EntityKind::MText(t) => parts.push((t.insert.y, t.insert.x, t.height, t.style.clone(), t.contents.clone(), e.clone())),
            _ => {}
        }
    }
    if parts.len() < 2 {
        return Err(bad("txt2mtxt", "select two or more texts"));
    }
    // Top to bottom, then left to right.
    parts.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.total_cmp(&b.1)));
    let top = parts.iter().map(|p| p.0).fold(f64::NEG_INFINITY, f64::max);
    let left = parts.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
    let first = &parts[0];
    let contents = parts.iter().map(|p| p.4.as_str()).collect::<Vec<_>>().join("\\P");
    let common = first.5.common.clone();
    let kind = EntityKind::MText(MText {
        insert: cadcraft_geom::Vec3::new(left, top, 0.0),
        height: first.2,
        width: 0.0,
        attach: 1,
        rotation: 0.0,
        style: first.3.clone(),
        contents,
        line_spacing: 1.0,
        line_spacing_exact: false,
        columns: None,
    });
    let gone: Vec<Handle> = parts.iter().map(|p| p.5.handle).collect();
    let d = s.doc_mut()?;
    for h in &gone {
        d.remove_entity(*h);
    }
    let n = s.add_entity(kind)?;
    s.doc_mut()?.modify_entity(n, |e| e.common = common).map_err(|e| bad("txt2mtxt", e.to_string()))?;
    s.set_selection(Vec::new());
    Ok(json!({ "handle": n.hex(), "combined": gone.len(), "message": format!("{} texts combined into one multiline text.", gone.len()) }))
}

fn run_laytrans(s: &mut Session, p: &Value) -> Result<Value> {
    let map: Map<String, Value> = p.get("map").and_then(Value::as_object).cloned().ok_or_else(|| bad("laytrans", "`map` {from: to} is required"))?;
    if map.is_empty() {
        return Err(bad("laytrans", "`map` is empty"));
    }
    let merge = find_command("laymrg").ok_or_else(|| bad("laytrans", "LAYMRG is not available"))?;
    let mut done = Vec::new();
    for (from, to) in &map {
        let to =
            to.as_str().map(str::trim).filter(|t| !t.is_empty()).ok_or_else(|| bad("laytrans", format!("`{from}`: the new layer name is text")))?;
        let d = s.doc()?;
        let src = d.layer(from).cloned().ok_or_else(|| bad("laytrans", format!("no layer \"{from}\"")))?;
        if src.name.eq_ignore_ascii_case(to) {
            continue;
        }
        if d.layer(to).is_none() {
            // A new layer takes the translated layer's properties.
            let mut l = src.clone();
            l.name = to.to_string();
            s.doc_mut()?.layers.push(l);
        }
        (merge.run)(s, &json!({ "from": [src.name], "to": to }))?;
        done.push(json!([src.name, to]));
    }
    let n = done.len();
    s.echo(format!("{n} layer(s) translated."));
    Ok(json!({ "translated": done }))
}

#[derive(Default)]
struct LayTransM {
    pairs: Vec<(String, String)>,
    from: Option<String>,
}

impl Interactive for LayTransM {
    fn name(&self) -> &'static str {
        "LAYTRANS"
    }
    fn prompt(&self, _s: &Session) -> Prompt {
        match &self.from {
            None => Prompt::new("Enter layer to translate (Enter to finish)", Accept::TEXT),
            Some(f) => Prompt::new(format!("Translate \"{f}\" to layer"), Accept::TEXT),
        }
    }
    fn input(&mut self, s: &mut Session, i: Input) -> Result<Step> {
        let t = match i {
            Input::Text(t) | Input::Keyword(t) => t.trim().to_string(),
            Input::Enter => String::new(),
            Input::Cancel => return Ok(Step::Done),
            _ => return Ok(Step::Continue),
        };
        match self.from.take() {
            None if t.is_empty() => {
                if !self.pairs.is_empty() {
                    let map: Map<String, Value> = self.pairs.iter().map(|(a, b)| (a.clone(), json!(b))).collect();
                    run_laytrans(s, &json!({ "map": map }))?;
                }
                Ok(Step::Done)
            }
            None => {
                if s.doc()?.layer(&t).is_none() {
                    s.echo(format!("No layer \"{t}\"."));
                } else {
                    self.from = Some(t);
                }
                Ok(Step::Continue)
            }
            Some(f) => {
                if !t.is_empty() {
                    self.pairs.push((f, t));
                }
                Ok(Step::Continue)
            }
        }
    }
}
