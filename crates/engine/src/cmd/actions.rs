//! The Action Recorder: ACTRECORD starts recording the commands and inputs given, ACTSTOP saves
//! them as a named action macro and PLAY replays one. Macros live in the session (the interface
//! keeps them with its preferences).

use cadcraft_doc::Handle;
use cadcraft_geom::Vec2;
use serde_json::{Value, json};

use super::*;
use crate::{Accept, ActStep, Input, Interactive, Prompt, Result, Session, Step};

/// How deep PLAY may nest (a macro that plays a macro…).
const MAX_PLAY_DEPTH: usize = 8;

thread_local! {
    static DEPTH: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("actrecord", "Record", run_actrecord).menu(&["Tools", "Action Recorder", "Record"]).alias(&["arr"]).noundo(),
        CommandSpec::new("actstop", "Stop", run_actstop)
            .menu(&["Tools", "Action Recorder", "Stop"])
            .alias(&["ars"])
            .params("{name}")
            .noundo()
            .interactive(|s| Ok(Box::new(ActStopM::new(s)))),
        CommandSpec::new("play", "Play", run_play)
            .menu(&["Tools", "Action Recorder", "Play"])
            .alias(&["-play"])
            .params("{name} | {} (lists the action macros)")
            .noundo()
            .interactive(|_| Ok(Box::new(PlayM))),
        CommandSpec::new("actdelete", "Delete Action Macro", run_actdelete).params("{name}").noundo(),
    ]
}

/// The Action Recorder's own commands are not recorded.
pub fn not_recorded(name: &str) -> bool {
    let n = name.trim().trim_start_matches(['_', '.', '-', '\'']).to_ascii_lowercase();
    matches!(n.as_str(), "actrecord" | "arr" | "actstop" | "ars" | "play" | "actdelete")
}

fn run_actrecord(s: &mut Session, _p: &Value) -> Result<Value> {
    if s.recording.is_some() {
        return Err(bad("actrecord", "the Action Recorder is already recording; ACTSTOP ends it"));
    }
    s.recording = Some(Vec::new());
    s.echo("Action Recorder: recording. ACTSTOP saves the action macro.");
    Ok(json!({ "recording": true }))
}

fn clean_name(n: &str) -> Option<String> {
    let n = n.trim();
    (!n.is_empty() && n.chars().count() <= 64 && n.chars().all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | ' '))).then(|| n.to_string())
}

fn save(s: &mut Session, name: &str, steps: Vec<ActStep>) -> Result<Value> {
    let name = clean_name(name).ok_or_else(|| bad("actstop", "a macro name is letters, digits, spaces, - and _ (up to 64)"))?;
    if steps.is_empty() {
        return Err(bad("actstop", "nothing was recorded"));
    }
    let n = steps.len();
    s.actions.insert(name.clone(), steps);
    s.echo(format!("Action macro \"{name}\" saved ({n} steps). PLAY replays it."));
    Ok(json!({ "name": name, "steps": n }))
}

fn run_actstop(s: &mut Session, p: &Value) -> Result<Value> {
    let steps = s.recording.take().ok_or_else(|| bad("actstop", "the Action Recorder is not recording"))?;
    let name = str_param(p, "name").map(str::to_string).unwrap_or_else(|| next_name(s));
    save(s, &name, steps)
}

fn next_name(s: &Session) -> String {
    (1..).map(|i| format!("ActMacro{i:03}")).find(|n| !s.actions.contains_key(n)).unwrap_or_else(|| "ActMacro".into())
}

fn run_actdelete(s: &mut Session, p: &Value) -> Result<Value> {
    let name = str_param(p, "name").ok_or_else(|| bad("actdelete", "`name` is required"))?;
    let key =
        s.actions.keys().find(|k| k.eq_ignore_ascii_case(name)).cloned().ok_or_else(|| bad("actdelete", format!("no action macro \"{name}\"")))?;
    s.actions.remove(&key);
    Ok(json!({ "deleted": key }))
}

fn run_play(s: &mut Session, p: &Value) -> Result<Value> {
    let Some(name) = str_param(p, "name") else { return Ok(json!({ "actions": s.actions.keys().collect::<Vec<_>>() })) };
    play(s, name)?;
    Ok(Value::Null)
}

/// Replay action macro `name`: its commands and inputs, as recorded.
pub fn play(s: &mut Session, name: &str) -> Result<()> {
    let (key, steps) = s
        .actions
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name.trim()))
        .map(|(k, v)| (k.clone(), v.clone()))
        .ok_or_else(|| bad("play", format!("no action macro \"{}\"", name.trim())))?;
    let depth = DEPTH.with(std::cell::Cell::get);
    if depth >= MAX_PLAY_DEPTH {
        return Err(bad("play", "action macros nest too deeply"));
    }
    DEPTH.with(|d| d.set(depth + 1));
    // A macro played while recording is not recorded again step by step.
    let recording = s.recording.take();
    let r = replay(s, &steps);
    s.recording = recording;
    DEPTH.with(|d| d.set(depth));
    r?;
    s.echo(format!("Action macro \"{key}\" played."));
    Ok(())
}

fn replay(s: &mut Session, steps: &[ActStep]) -> Result<()> {
    for st in steps {
        match st {
            ActStep::Start(n) => s.start(n)?,
            ActStep::Point([x, y]) => s.input(Input::Point(Vec2::new(*x, *y)))?,
            ActStep::Keyword(k) => s.input(Input::Keyword(k.clone()))?,
            ActStep::Text(t) => s.input(Input::Text(t.clone()))?,
            ActStep::Pick(hs) => {
                let d = s.doc()?;
                let live: Vec<Handle> = hs.iter().filter_map(|h| Handle::parse_hex(h)).filter(|h| d.entity(*h).is_some()).collect();
                s.input(Input::Pick(live))?;
            }
            ActStep::Enter => s.input(Input::Enter)?,
            ActStep::Cancel => s.input(Input::Cancel)?,
        }
    }
    Ok(())
}

struct ActStopM {
    steps: Option<Vec<ActStep>>,
    default: String,
}

impl ActStopM {
    fn new(s: &Session) -> Self {
        ActStopM { steps: None, default: next_name(s) }
    }
}

impl Interactive for ActStopM {
    fn name(&self) -> &'static str {
        "ACTSTOP"
    }
    fn begin(&mut self, s: &mut Session) -> Result<Step> {
        match s.recording.take() {
            Some(st) => {
                self.steps = Some(st);
                Ok(Step::Continue)
            }
            None => {
                s.echo("The Action Recorder is not recording.");
                Ok(Step::Done)
            }
        }
    }
    fn prompt(&self, _s: &Session) -> Prompt {
        Prompt::new("Enter action macro name", Accept::TEXT).default(self.default.clone())
    }
    fn input(&mut self, s: &mut Session, i: Input) -> Result<Step> {
        let name = match i {
            Input::Text(t) | Input::Keyword(t) if !t.trim().is_empty() => t,
            Input::Text(_) | Input::Keyword(_) | Input::Enter => self.default.clone(),
            Input::Cancel => {
                s.echo("Recording discarded.");
                return Ok(Step::Done);
            }
            _ => return Ok(Step::Continue),
        };
        let steps = self.steps.take().unwrap_or_default();
        match save(s, &name, steps.clone()) {
            Ok(_) => Ok(Step::Done),
            Err(e) => {
                // Ask again for a usable name.
                self.steps = Some(steps);
                s.echo(e.to_string());
                Ok(Step::Continue)
            }
        }
    }
}

struct PlayM;

impl Interactive for PlayM {
    fn name(&self) -> &'static str {
        "PLAY"
    }
    fn begin(&mut self, s: &mut Session) -> Result<Step> {
        if s.actions.is_empty() {
            s.echo("No action macros. ACTRECORD records one.");
            return Ok(Step::Done);
        }
        Ok(Step::Continue)
    }
    fn prompt(&self, s: &Session) -> Prompt {
        let names: Vec<&str> = s.actions.keys().map(String::as_str).filter(|n| !n.contains(' ')).collect();
        let p = Prompt::new("Enter action macro name", Accept::TEXT);
        let p = if names.len() <= 12 { p.kw(&names) } else { p };
        match s.actions.keys().next() {
            Some(first) => p.default(first.clone()),
            None => p,
        }
    }
    fn input(&mut self, s: &mut Session, i: Input) -> Result<Step> {
        let name = match i {
            Input::Text(t) | Input::Keyword(t) if !t.trim().is_empty() => t,
            Input::Text(_) | Input::Keyword(_) | Input::Enter => match s.actions.keys().next() {
                Some(n) => n.clone(),
                None => return Ok(Step::Done),
            },
            Input::Cancel => return Ok(Step::Done),
            _ => return Ok(Step::Continue),
        };
        match play(s, &name) {
            Ok(()) => Ok(Step::Done),
            Err(e) => {
                s.echo(e.to_string());
                Ok(Step::Done)
            }
        }
    }
}
