//! The Hole dialog (TAPHOLE's Settings): thread standard and size from the JIS tables, view,
//! depths and callout. OK applies them through `taphole.settings`.

use cadcraft_engine::HoleSettings;
use egui::{RichText, vec2};
use serde_json::{Value, json};

use crate::CadApp;
use crate::theme::Tokens;

const KINDS: &[(&str, &str)] = &[
    ("M", "Metric coarse M (JIS B 0205)"),
    ("MF", "Metric fine M (JIS B 0205)"),
    ("G", "Parallel pipe G (JIS B 0202)"),
    ("PT", "Taper pipe Rc / PT (JIS B 0203)"),
    ("DRILL", "Drilled hole"),
];

#[derive(Clone)]
struct Draft {
    st: HoleSettings,
    sizes: Vec<Value>,
}

pub fn wanted(app: &mut CadApp) -> bool {
    // The Settings option's dialog goes with its command (Escape ends both).
    if app.session.hole_dialog && !app.session.running.as_ref().is_some_and(|r| r.id.starts_with("taphole")) {
        app.session.hole_dialog = false;
    }
    app.session.hole_dialog || app.ui.dialog.as_deref() == Some("hole")
}

/// The sizes of `kind` (the engine's table).
fn sizes(app: &mut CadApp, st: &HoleSettings) -> Vec<Value> {
    let saved = app.session.hole.clone();
    let r = app.session.execute("taphole.settings", &json!({ "kind": st.kind }));
    app.session.hole = saved;
    r.ok().and_then(|v| v.get("sizes").and_then(Value::as_array).cloned()).unwrap_or_default()
}

pub fn show(app: &mut CadApp, ctx: &egui::Context) {
    let id = egui::Id::new("hole_settings_draft");
    if !wanted(app) {
        ctx.data_mut(|d| {
            d.remove::<Draft>(id);
        });
        return;
    }
    let mut draft = match ctx.data_mut(|d| d.get_temp::<Draft>(id)) {
        Some(d) => d,
        None => {
            let st = app.session.hole.clone();
            let sizes = sizes(app, &st);
            Draft { st, sizes }
        }
    };
    let t = Tokens::get();
    let mut answer: Option<bool> = None;
    let mut keep = true;
    let mut kind_changed = false;
    egui::Window::new(crate::tl!("Hole"))
        .id(egui::Id::new("Hole Settings"))
        .open(&mut keep)
        .collapsible(false)
        .resizable(false)
        .default_width(520.0)
        .show(ctx, |ui| {
            let st = &mut draft.st;
            ui.columns(2, |c| {
                c[0].group(|ui| {
                    ui.set_min_width(230.0);
                    ui.label(RichText::new(crate::tl!("Thread")).strong());
                    for (k, label) in KINDS {
                        if ui.radio(st.kind == *k, crate::i18n::t(label)).clicked() && st.kind != *k {
                            st.kind = (*k).to_string();
                            kind_changed = true;
                        }
                    }
                });
                c[0].add_space(6.0);
                c[0].group(|ui| {
                    ui.label(RichText::new(crate::tl!("View")).strong());
                    ui.horizontal(|ui| {
                        ui.radio_value(&mut st.view, "top".to_string(), crate::tl!("Top"));
                        ui.radio_value(&mut st.view, "side".to_string(), crate::tl!("Side (section)"));
                    });
                    ui.add_enabled_ui(st.view == "side", |ui| {
                        ui.horizontal(|ui| {
                            ui.label(crate::tl!("Angle"));
                            ui.add(egui::DragValue::new(&mut st.angle).range(-360.0..=360.0).suffix("°"));
                        });
                    });
                });
                c[1].group(|ui| {
                    ui.set_min_width(230.0);
                    ui.label(RichText::new(crate::tl!("Size")).strong());
                    if st.kind == "DRILL" {
                        ui.horizontal(|ui| {
                            ui.label(crate::tl!("Diameter"));
                            ui.add(egui::DragValue::new(&mut st.drill).range(0.1..=1000.0).speed(0.1));
                        });
                    } else {
                        let name_of = |v: &Value| v.get("name").and_then(Value::as_str).unwrap_or("").to_string();
                        egui::ComboBox::from_id_salt("hole_size").width(200.0).selected_text(st.size.clone()).height(320.0).show_ui(ui, |ui| {
                            for v in &draft.sizes {
                                let n = name_of(v);
                                let drill = v.get("drill").and_then(Value::as_f64).unwrap_or(0.0);
                                if ui.selectable_label(st.size == n, format!("{n}    Ø{drill}")).clicked() {
                                    st.size = n;
                                }
                            }
                        });
                        if let Some(v) = draft.sizes.iter().find(|v| name_of(v) == st.size) {
                            let f = |k: &str| v.get(k).and_then(Value::as_f64).unwrap_or(0.0);
                            ui.label(
                                RichText::new(format!(
                                    "{} {:.3}   {} {:.3}\n{} Ø{}   {} {}",
                                    crate::tl!("Major"),
                                    f("major"),
                                    crate::tl!("Pitch"),
                                    f("pitch"),
                                    crate::tl!("Tap drill"),
                                    f("drill"),
                                    crate::tl!("Thread depth"),
                                    f("depth")
                                ))
                                .small()
                                .color(t.text_dim),
                            );
                        }
                    }
                });
                c[1].add_space(6.0);
                c[1].group(|ui| {
                    ui.label(RichText::new(crate::tl!("Depth")).strong());
                    ui.checkbox(&mut st.through, crate::tl!("Through"));
                    egui::Grid::new("hole_depths").num_columns(2).spacing(vec2(8.0, 4.0)).show(ui, |ui| {
                        if st.through {
                            ui.label(crate::tl!("Thickness"));
                            ui.add(egui::DragValue::new(&mut st.thickness).range(0.0..=100_000.0).speed(0.5));
                            ui.end_row();
                        } else {
                            if st.kind != "DRILL" {
                                ui.label(crate::tl!("Thread depth"));
                                ui.add(egui::DragValue::new(&mut st.thread_depth).range(0.0..=100_000.0).speed(0.5));
                                ui.end_row();
                            }
                            ui.label(crate::tl!("Drill depth"));
                            ui.add(egui::DragValue::new(&mut st.drill_depth).range(0.0..=100_000.0).speed(0.5));
                            ui.end_row();
                        }
                    });
                    ui.label(RichText::new(crate::tl!("0 = from the JIS table")).small().color(t.text_faint));
                });
                c[1].add_space(6.0);
                c[1].checkbox(&mut st.note, crate::tl!("Place the size note"));
            });
            ui.add_space(8.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(crate::tl!("Cancel")).clicked() {
                    answer = Some(false);
                }
                if ui.button(crate::tl!("OK")).clicked() {
                    answer = Some(true);
                }
            });
        });
    if kind_changed {
        draft.st.size.clear();
        draft.sizes = sizes(app, &draft.st);
        // The first common size of the new kind, as the engine picks it.
        let saved = app.session.hole.clone();
        if let Ok(v) = app.session.execute("taphole.settings", &json!({ "kind": draft.st.kind })) {
            draft.st.size = v.get("size").and_then(Value::as_str).unwrap_or("").to_string();
        }
        app.session.hole = saved;
    }
    if !keep {
        answer = Some(false);
    }
    match answer {
        Some(ok) => {
            if ok {
                let v = serde_json::to_value(&draft.st).unwrap_or_default();
                let _ = app.run("taphole.settings", v);
            }
            app.session.hole_dialog = false;
            if app.ui.dialog.as_deref() == Some("hole") {
                app.ui.dialog = None;
            }
            ctx.data_mut(|d| {
                d.remove::<Draft>(id);
            });
        }
        None => ctx.data_mut(|d| {
            d.insert_temp(id, draft);
        }),
    }
}
