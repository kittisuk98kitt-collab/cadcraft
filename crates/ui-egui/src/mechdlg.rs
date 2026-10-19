//! The Pipe and Structural Member dialogs: PIPE / STEEL's Settings option, and PIPEEDIT /
//! STEELEDIT on a picked run or member. OK applies through the commands' JSON forms.

use egui::{RichText, vec2};
use serde_json::{Value, json};

use crate::CadApp;
use crate::theme::Tokens;

#[derive(Clone)]
struct PipeDraft {
    v: Value,
    /// The run being edited (PIPEEDIT), or the settings.
    edit: Option<String>,
    sizes: Vec<String>,
}

#[derive(Clone)]
struct SteelDraft {
    v: Value,
    edit: Option<String>,
    sizes: Vec<String>,
}

fn names(v: &Value, key: &str, field: Option<&str>) -> Vec<String> {
    v.get(key)
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|x| field.map_or_else(|| x.as_str(), |f| x.get(f).and_then(Value::as_str)).map(str::to_string)).collect())
        .unwrap_or_default()
}

fn s_of(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}

fn b_of(v: &Value, k: &str) -> bool {
    v.get(k).and_then(Value::as_bool).unwrap_or(false)
}

/// A settings call that leaves the session's settings as they were (to read a table).
fn peek(app: &mut CadApp, cmd: &str, p: Value) -> Value {
    let (pipe, steel) = (app.session.pipe.clone(), app.session.steel.clone());
    let r = app.session.execute(cmd, &p).unwrap_or(Value::Null);
    app.session.pipe = pipe;
    app.session.steel = steel;
    r
}

pub fn show(app: &mut CadApp, ctx: &egui::Context) {
    pipe(app, ctx);
    steel(app, ctx);
}

fn buttons(ui: &mut egui::Ui, answer: &mut Option<bool>) {
    ui.add_space(8.0);
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if ui.button(crate::tl!("Cancel")).clicked() {
            *answer = Some(false);
        }
        if ui.button(crate::tl!("OK")).clicked() {
            *answer = Some(true);
        }
    });
}

fn pipe(app: &mut CadApp, ctx: &egui::Context) {
    let id = egui::Id::new("pipe_dialog_draft");
    if !app.session.pipe_dialog && app.session.pipe_edit.is_none() {
        ctx.data_mut(|d| {
            d.remove::<PipeDraft>(id);
        });
        return;
    }
    let mut draft = match ctx.data_mut(|d| d.get_temp::<PipeDraft>(id)) {
        Some(d) => d,
        None => {
            let table = peek(app, "pipe.settings", json!({}));
            let sizes = names(&table, "sizes", Some("name"));
            match app.session.pipe_edit {
                Some(h) => {
                    let v = app.session.execute("pipe.run", &json!({ "handle": h.hex() })).unwrap_or(Value::Null);
                    PipeDraft { edit: Some(h.hex()), v, sizes }
                }
                None => PipeDraft { v: table, edit: None, sizes },
            }
        }
    };
    let t = Tokens::get();
    let mut answer = None;
    let mut keep = true;
    let title = if draft.edit.is_some() { crate::tl!("Edit Pipe") } else { crate::tl!("Pipe") };
    egui::Window::new(title).id(egui::Id::new("Pipe Dialog")).open(&mut keep).collapsible(false).resizable(false).show(ctx, |ui| {
        let v = &mut draft.v;
        egui::Grid::new("pipe_grid").num_columns(2).spacing(vec2(10.0, 6.0)).show(ui, |ui| {
            ui.label(crate::tl!("Pipe"));
            let mut sch = s_of(v, "schedule");
            egui::ComboBox::from_id_salt("pipe_sch").selected_text(sch.clone()).show_ui(ui, |ui| {
                for (k, label) in [("SGP", "SGP (JIS G3452)"), ("SCH40", "SCH40 (JIS G3454)"), ("SCH60", "SCH60 (JIS G3454)")] {
                    ui.selectable_value(&mut sch, k.to_string(), label);
                }
            });
            v["schedule"] = json!(sch);
            ui.end_row();
            ui.label(crate::tl!("Size"));
            let mut size = s_of(v, "size");
            egui::ComboBox::from_id_salt("pipe_size").selected_text(size.clone()).height(320.0).show_ui(ui, |ui| {
                for n in &draft.sizes {
                    ui.selectable_value(&mut size, n.clone(), n);
                }
            });
            v["size"] = json!(size);
            ui.end_row();
        });
        ui.add_space(4.0);
        for (k, label) in [
            ("flangeStart", crate::tl!("Flange at start (JIS 10K)")),
            ("flangeEnd", crate::tl!("Flange at end (JIS 10K)")),
            ("label", crate::tl!("Size label")),
        ] {
            let mut on = b_of(v, k);
            if ui.checkbox(&mut on, label).changed() {
                v[k] = json!(on);
            }
        }
        ui.label(RichText::new(crate::tl!("Elbows and tees are fitted where the run bends and branches.")).small().color(t.text_dim));
        buttons(ui, &mut answer);
    });
    if !keep {
        answer = Some(false);
    }
    match answer {
        Some(ok) => {
            if ok {
                let mut p = json!({
                    "schedule": draft.v["schedule"], "size": draft.v["size"],
                    "flangeStart": b_of(&draft.v, "flangeStart"), "flangeEnd": b_of(&draft.v, "flangeEnd"), "label": b_of(&draft.v, "label"),
                });
                match &draft.edit {
                    Some(h) => {
                        p["handle"] = json!(h);
                        let _ = app.run("pipeedit", p);
                    }
                    None => {
                        let _ = app.run("pipe.settings", p);
                    }
                }
            }
            app.session.pipe_dialog = false;
            app.session.pipe_edit = None;
            ctx.data_mut(|d| {
                d.remove::<PipeDraft>(id);
            });
        }
        None => ctx.data_mut(|d| {
            d.insert_temp(id, draft);
        }),
    }
}

const STEEL_KINDS: &[(&str, &str)] = &[
    ("C", "Channel (JIS G3192)"),
    ("L", "Angle (JIS G3192)"),
    ("H", "H section (JIS G3192)"),
    ("TUBE", "Square / rectangular tube (JIS G3466)"),
    ("FB", "Flat bar"),
];

fn steel(app: &mut CadApp, ctx: &egui::Context) {
    let id = egui::Id::new("steel_dialog_draft");
    if !app.session.steel_dialog && app.session.steel_edit.is_none() {
        ctx.data_mut(|d| {
            d.remove::<SteelDraft>(id);
        });
        return;
    }
    let mut draft = match ctx.data_mut(|d| d.get_temp::<SteelDraft>(id)) {
        Some(d) => d,
        None => match app.session.steel_edit {
            Some(h) => {
                let v = app.session.execute("steel.member", &json!({ "handle": h.hex() })).unwrap_or(Value::Null);
                let table = peek(app, "steel.settings", json!({ "kind": s_of(&v, "kind") }));
                SteelDraft { sizes: names(&table, "sizes", None), v, edit: Some(h.hex()) }
            }
            None => {
                let v = peek(app, "steel.settings", json!({}));
                SteelDraft { sizes: names(&v, "sizes", None), v, edit: None }
            }
        },
    };
    let t = Tokens::get();
    let mut answer = None;
    let mut keep = true;
    let mut kind_changed = None;
    let editing = draft.edit.is_some();
    let title = if editing { crate::tl!("Edit Structural Member") } else { crate::tl!("Structural Member") };
    egui::Window::new(title).id(egui::Id::new("Steel Dialog")).open(&mut keep).collapsible(false).resizable(false).show(ctx, |ui| {
        let v = &mut draft.v;
        ui.columns(2, |c| {
            c[0].group(|ui| {
                ui.set_min_width(220.0);
                ui.label(RichText::new(crate::tl!("Profile")).strong());
                let kind = s_of(v, "kind");
                for (k, label) in STEEL_KINDS {
                    if ui.radio(kind == *k, crate::i18n::t(label)).clicked() && kind != *k {
                        kind_changed = Some((*k).to_string());
                    }
                }
            });
            c[1].group(|ui| {
                ui.set_min_width(220.0);
                ui.label(RichText::new(crate::tl!("Size")).strong());
                let mut size = s_of(v, "size");
                if s_of(v, "kind") == "FB" {
                    ui.add(egui::TextEdit::singleline(&mut size).hint_text("50x6"));
                    ui.label(RichText::new(crate::tl!("Width x thickness")).small().color(t.text_dim));
                } else {
                    egui::ComboBox::from_id_salt("steel_size").width(200.0).selected_text(size.clone()).height(340.0).show_ui(ui, |ui| {
                        for n in &draft.sizes {
                            ui.selectable_value(&mut size, n.clone(), n);
                        }
                    });
                }
                v["size"] = json!(size);
            });
            c[1].add_space(6.0);
            c[1].group(|ui| {
                ui.add_enabled_ui(!editing, |ui| {
                    ui.label(RichText::new(crate::tl!("View")).strong());
                    let mut view = s_of(v, "view");
                    ui.horizontal(|ui| {
                        ui.radio_value(&mut view, "section".to_string(), crate::tl!("Section"));
                        ui.radio_value(&mut view, "side".to_string(), crate::tl!("Side"));
                        ui.radio_value(&mut view, "top".to_string(), crate::tl!("Top"));
                    });
                    v["view"] = json!(view.clone());
                    ui.horizontal(|ui| {
                        ui.label(crate::tl!("Insertion point"));
                        let mut a = s_of(v, "anchor");
                        let options: &[(&str, &str)] = if view == "section" {
                            &[
                                ("MC", "Centre"),
                                ("BL", "Bottom left"),
                                ("BC", "Bottom centre"),
                                ("BR", "Bottom right"),
                                ("ML", "Middle left"),
                                ("MR", "Middle right"),
                                ("TL", "Top left"),
                                ("TC", "Top centre"),
                                ("TR", "Top right"),
                            ]
                        } else {
                            &[("MC", "Centre line"), ("BL", "Bottom edge"), ("TL", "Top edge")]
                        };
                        let shown = options.iter().find(|(k, _)| *k == a).map_or(a.clone(), |(_, l)| crate::i18n::t(l).to_string());
                        egui::ComboBox::from_id_salt("steel_anchor").selected_text(shown).show_ui(ui, |ui| {
                            for (k, l) in options {
                                ui.selectable_value(&mut a, (*k).to_string(), crate::i18n::t(l));
                            }
                        });
                        v["anchor"] = json!(a);
                    });
                    if view == "section" {
                        ui.horizontal(|ui| {
                            ui.label(crate::tl!("Angle"));
                            let mut ang = v.get("angle").and_then(Value::as_f64).unwrap_or(0.0);
                            if ui.add(egui::DragValue::new(&mut ang).range(-360.0..=360.0).suffix("°")).changed() {
                                v["angle"] = json!(ang);
                            }
                        });
                    }
                });
                let mut mirror = b_of(v, "mirror");
                if ui.checkbox(&mut mirror, crate::tl!("Mirror")).changed() {
                    v["mirror"] = json!(mirror);
                }
                let mut hatch = b_of(v, "hatch");
                if ui.checkbox(&mut hatch, crate::tl!("Hatch the section")).changed() {
                    v["hatch"] = json!(hatch);
                }
            });
        });
        buttons(ui, &mut answer);
    });
    if let Some(k) = kind_changed {
        let table = peek(app, "steel.settings", json!({ "kind": k }));
        draft.sizes = names(&table, "sizes", None);
        draft.v["kind"] = json!(k);
        draft.v["size"] = table.get("size").cloned().unwrap_or(json!(""));
    }
    if !keep {
        answer = Some(false);
    }
    match answer {
        Some(ok) => {
            if ok {
                let v = &draft.v;
                let mut p =
                    json!({ "kind": v["kind"], "size": v["size"], "mirror": b_of(v, "mirror"), "hatch": b_of(v, "hatch"), "anchor": v["anchor"] });
                match &draft.edit {
                    Some(h) => {
                        p["handle"] = json!(h);
                        let _ = app.run("steeledit", p);
                    }
                    None => {
                        p["view"] = v["view"].clone();
                        p["angle"] = v.get("angle").cloned().unwrap_or(json!(0.0));
                        let _ = app.run("steel.settings", p);
                    }
                }
            }
            app.session.steel_dialog = false;
            app.session.steel_edit = None;
            ctx.data_mut(|d| {
                d.remove::<SteelDraft>(id);
            });
        }
        None => ctx.data_mut(|d| {
            d.insert_temp(id, draft);
        }),
    }
}
