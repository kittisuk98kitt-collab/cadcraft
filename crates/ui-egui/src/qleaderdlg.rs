//! QLEADER's Leader Settings dialog: the Annotation, Leader Line & Arrow and Attachment tabs.
//! It opens from QLEADER's Settings option (or the `qleaderset` UI command) and applies its
//! values through `qleader.settings` on OK.

use cadcraft_engine::{QAnnotation, QLeaderSettings, QReuse};
use egui::vec2;

use crate::CadApp;

/// Arrowheads offered (display name, DIMLDRBLK name).
const ARROWS: &[(&str, &str)] = &[
    ("Closed filled", "_CLOSEDFILLED"),
    ("Closed blank", "_CLOSEDBLANK"),
    ("Closed", "_CLOSED"),
    ("Dot", "_DOT"),
    ("Dot small", "_DOTSMALL"),
    ("Dot blank", "_DOTBLANK"),
    ("Architectural tick", "_ARCHTICK"),
    ("Oblique", "_OBLIQUE"),
    ("Open", "_OPEN"),
    ("Origin indicator", "_ORIGIN"),
    ("Right angle", "_OPEN90"),
    ("Open 30", "_OPEN30"),
    ("Datum triangle", "_DATUMBLANK"),
    ("Datum triangle filled", "_DATUMFILLED"),
    ("Integral", "_INTEGRAL"),
    ("None", "None"),
];

/// Angle constraints (settings value, label).
const ANGLES: &[(i32, &str)] = &[(0, "Any angle"), (-1, "Horizontal"), (90, "90°"), (45, "45°"), (30, "30°"), (15, "15°")];

const ATTACH: &[&str] = &["Top of top line", "Middle of top line", "Middle of multi-line text", "Middle of bottom line", "Bottom of bottom line"];

#[derive(Clone)]
struct Draft {
    st: QLeaderSettings,
    tab: u8,
}

fn norm(n: &str) -> String {
    n.trim().trim_start_matches('_').chars().filter(char::is_ascii_alphanumeric).collect::<String>().to_ascii_lowercase()
}

/// The arrowhead's display name ("" = closed filled, as DIMLDRBLK).
fn arrow_label(name: &str) -> &'static str {
    let n = norm(name);
    ARROWS.iter().find(|(_, v)| norm(v) == n || (n.is_empty() && *v == "_CLOSEDFILLED")).map_or("Closed filled", |(l, _)| l)
}

/// Whether the dialog is up: asked by QLEADER's Settings option, or opened on its own.
pub fn wanted(app: &mut CadApp) -> bool {
    if app.session.qleader_dialog && !app.session.running.as_ref().is_some_and(|r| r.id == "qleader") {
        app.session.qleader_dialog = false;
    }
    app.session.qleader_dialog || app.ui.dialog.as_deref() == Some("qleader")
}

pub fn show(app: &mut CadApp, ctx: &egui::Context) {
    let id = egui::Id::new("qleader_settings_draft");
    if !wanted(app) {
        ctx.data_mut(|d| {
            d.remove::<Draft>(id);
        });
        return;
    }
    let mut draft = ctx.data_mut(|d| d.get_temp::<Draft>(id)).unwrap_or_else(|| {
        let mut st = app.session.qleader.clone();
        // The arrowhead shown is the dimension style's until one is chosen here.
        if st.arrow.is_empty() {
            st.arrow = app
                .session
                .doc()
                .ok()
                .and_then(|d| d.dim_style(&d.header.str("DIMSTYLE", "Standard")).map(|s| s.leader_arrow_block.clone()))
                .unwrap_or_default();
        }
        Draft { st, tab: 0 }
    });
    let can_reuse = app.session.qleader_reuse.is_some();
    let mut answer: Option<bool> = None;
    let mut keep = true;
    egui::Window::new(crate::tl!("Leader Settings"))
        .id(egui::Id::new("Leader Settings"))
        .open(&mut keep)
        .collapsible(false)
        .resizable(false)
        .default_width(460.0)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                for (k, name) in [crate::tl!("Annotation"), crate::tl!("Leader Line & Arrow"), crate::tl!("Attachment")].into_iter().enumerate() {
                    ui.selectable_value(&mut draft.tab, k as u8, name);
                }
            });
            ui.separator();
            ui.set_min_height(180.0);
            let st = &mut draft.st;
            match draft.tab {
                0 => annotation_tab(ui, st, can_reuse),
                1 => line_tab(ui, st),
                _ => attachment_tab(ui, st),
            }
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
    if !keep {
        answer = Some(false);
    }
    match answer {
        Some(ok) => {
            if ok {
                let v = serde_json::to_value(&draft.st).unwrap_or_default();
                let _ = app.run("qleader.settings", v);
            }
            app.session.qleader_dialog = false;
            if app.ui.dialog.as_deref() == Some("qleader") {
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

fn annotation_tab(ui: &mut egui::Ui, st: &mut QLeaderSettings, can_reuse: bool) {
    ui.columns(2, |c| {
        c[0].group(|ui| {
            ui.set_min_width(170.0);
            ui.label(crate::tl!("Annotation Type"));
            for (v, l) in [
                (QAnnotation::Mtext, crate::tl!("MText")),
                (QAnnotation::Copy, crate::tl!("Copy an Object")),
                (QAnnotation::Tolerance, crate::tl!("Tolerance")),
                (QAnnotation::Block, crate::tl!("Block Reference")),
                (QAnnotation::None, crate::tl!("None")),
            ] {
                ui.radio_value(&mut st.annotation, v, l);
            }
        });
        let mtext = st.annotation == QAnnotation::Mtext;
        c[1].group(|ui| {
            ui.label(crate::tl!("MText options:"));
            ui.add_enabled_ui(mtext, |ui| {
                ui.checkbox(&mut st.prompt_width, crate::tl!("Prompt for width"));
                ui.checkbox(&mut st.left_justify, crate::tl!("Always left justify"));
                ui.checkbox(&mut st.frame, crate::tl!("Frame text"));
            });
        });
        c[1].add_space(6.0);
        c[1].group(|ui| {
            ui.label(crate::tl!("Annotation Reuse"));
            ui.radio_value(&mut st.reuse, QReuse::None, crate::tl!("None"));
            ui.radio_value(&mut st.reuse, QReuse::Next, crate::tl!("Reuse Next"));
            ui.add_enabled_ui(can_reuse, |ui| {
                ui.radio_value(&mut st.reuse, QReuse::Current, crate::tl!("Reuse Current"));
            });
        });
    });
}

fn line_tab(ui: &mut egui::Ui, st: &mut QLeaderSettings) {
    ui.columns(2, |c| {
        c[0].group(|ui| {
            ui.set_min_width(170.0);
            ui.label(crate::tl!("Leader Line"));
            ui.radio_value(&mut st.spline, false, crate::tl!("Straight"));
            ui.radio_value(&mut st.spline, true, crate::tl!("Spline"));
        });
        c[0].add_space(6.0);
        c[0].group(|ui| {
            ui.label(crate::tl!("Number of Points"));
            let mut unlimited = st.max_points == 0;
            if ui.checkbox(&mut unlimited, crate::tl!("No Limit")).changed() {
                st.max_points = if unlimited { 0 } else { 3 };
            }
            ui.add_enabled_ui(!unlimited, |ui| {
                ui.horizontal(|ui| {
                    let mut n = st.max_points.max(2);
                    if ui.add(egui::DragValue::new(&mut n).range(2..=999)).changed() && !unlimited {
                        st.max_points = n;
                    }
                    ui.label(crate::tl!("Maximum"));
                });
            });
        });
        c[1].group(|ui| {
            ui.label(crate::tl!("Arrowhead"));
            egui::ComboBox::from_id_salt("qleader_arrow").width(180.0).selected_text(crate::i18n::t(arrow_label(&st.arrow))).show_ui(ui, |ui| {
                for (l, v) in ARROWS {
                    if ui.selectable_label(norm(&st.arrow) == norm(v), crate::i18n::t(l)).clicked() {
                        st.arrow = (*v).to_string();
                    }
                }
            });
        });
        c[1].add_space(6.0);
        c[1].group(|ui| {
            ui.label(crate::tl!("Angle Constraints"));
            egui::Grid::new("qleader_angles").num_columns(2).spacing(vec2(8.0, 4.0)).show(ui, |ui| {
                for (k, label) in [crate::tl!("First Segment:"), crate::tl!("Second Segment:")].into_iter().enumerate() {
                    ui.label(label);
                    let v = if k == 0 { &mut st.angle1 } else { &mut st.angle2 };
                    let cur = ANGLES.iter().find(|(a, _)| a == v).map_or("Any angle", |(_, l)| l);
                    egui::ComboBox::from_id_salt(("qleader_angle", k)).width(110.0).selected_text(crate::i18n::t(cur)).show_ui(ui, |ui| {
                        for (a, l) in ANGLES {
                            ui.selectable_value(v, *a, crate::i18n::t(l));
                        }
                    });
                    ui.end_row();
                }
            });
        });
    });
}

fn attachment_tab(ui: &mut egui::Ui, st: &mut QLeaderSettings) {
    ui.vertical_centered(|ui| {
        ui.group(|ui| {
            ui.label(crate::tl!("Multi-line Text Attachment"));
            ui.add_enabled_ui(!st.underline, |ui| {
                egui::Grid::new("qleader_attach").num_columns(3).min_col_width(110.0).spacing(vec2(24.0, 6.0)).show(ui, |ui| {
                    ui.label(crate::tl!("Text on left side"));
                    ui.label("");
                    ui.label(crate::tl!("Text on right side"));
                    ui.end_row();
                    for (k, label) in ATTACH.iter().enumerate() {
                        let k = k as u8;
                        ui.vertical_centered(|ui| ui.radio_value(&mut st.attach_left, k, ""));
                        ui.vertical_centered(|ui| ui.add(egui::Label::new(crate::i18n::t(label)).extend()));
                        ui.vertical_centered(|ui| ui.radio_value(&mut st.attach_right, k, ""));
                        ui.end_row();
                    }
                });
            });
        });
        ui.add_space(6.0);
        ui.checkbox(&mut st.underline, crate::tl!("Underline bottom line"));
    });
}
