//! The Options dialog. It reads the engine's settings and the interface state, and changes them
//! only through the OPTIONS JSON form (`app.run("options", …)`), so the dialog, scripts and the
//! control channel share one path. Changes apply at once.
//!
//! The open tab is part of the dialog name (`"options:selection"`), so `ui.set` can show any tab.

use egui::{Color32, RichText, Sense, Stroke, vec2};
use serde_json::{Map, Value, json};

use crate::CadApp;
use crate::animation::AnimationPref;
use crate::theme::{ThemePref, Tokens};

/// The tabs, in AutoCAD's order: (id in the dialog name, label).
pub(super) const TABS: [(&str, &str); 10] = [
    ("files", "Files"),
    ("display", "Display"),
    ("opensave", "Open and Save"),
    ("plot", "Plot and Publish"),
    ("system", "System"),
    ("preferences", "User Preferences"),
    ("drafting", "Drafting"),
    ("modeling", "3D Modeling"),
    ("selection", "Selection"),
    ("profiles", "Profiles"),
];

const WIDTH: f32 = 760.0;
const MIN_TAB_HEIGHT: f32 = 380.0;

/// What a tab asks for besides OPTIONS changes: drawing variables (SETVAR), interface toggles
/// (`ui.toggle.*`, which flip) and a command to start.
#[derive(Default)]
struct Extra {
    vars: Vec<(&'static str, Value)>,
    toggles: Vec<&'static str>,
    start: Option<&'static str>,
}

/// Show the Options dialog (`name` is `"options"` or `"options:<tab>"`).
pub fn dialog(app: &mut CadApp, ctx: &egui::Context, name: &str, open: &mut bool) {
    let tab = name.strip_prefix("options:").and_then(|t| TABS.iter().find(|(id, _)| *id == t)).map_or("display", |(id, _)| *id);
    let mut changes = Map::new();
    let mut extra = Extra::default();
    let mut next_tab = None;
    let mut close = false;
    let center = ctx.content_rect().center();
    let drawing = app.session.state().map(|st| st.title.clone()).unwrap_or_default();
    egui::Window::new(crate::tl!("Options"))
        .id(egui::Id::new("cc_options"))
        .open(open)
        .resizable(false)
        .collapsible(false)
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(center)
        .show(ctx, |ui| {
            ui.set_width(WIDTH);
            ui.horizontal(|ui| {
                ui.label(RichText::new(crate::tl!("Current profile:")).color(Tokens::get().text_dim));
                ui.label("<<CADCraft>>");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(&drawing);
                    ui.label(RichText::new(crate::tl!("Current drawing:")).color(Tokens::get().text_dim));
                });
            });
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for (id, label) in TABS {
                    if ui.selectable_label(tab == id, crate::i18n::t(label)).clicked() {
                        next_tab = Some(id);
                    }
                }
            });
            ui.separator();
            ui.add_space(4.0);
            let top = ui.cursor().top();
            match tab {
                "files" => files(app, ui, &mut changes, &mut extra),
                "opensave" => open_save(app, ui, &mut changes),
                "plot" => plot(ui, &mut extra),
                "system" => system(app, ui, &mut changes),
                "drafting" => drafting(app, ui, &mut changes, &mut extra),
                "modeling" => modeling(app, ui, &mut extra),
                "selection" => selection(app, ui, &mut changes),
                "preferences" => preferences(app, ui, &mut changes, &mut extra),
                "profiles" => profiles(app, ui, &mut changes),
                _ => display(app, ui, &mut changes, &mut extra),
            }
            // The same height on every tab, so switching tabs doesn't move the buttons.
            ui.add_space((MIN_TAB_HEIGHT - (ui.cursor().top() - top)).max(8.0));
            ui.separator();
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(crate::tl!("OK")).clicked() {
                        close = true;
                    }
                    ui.label(RichText::new(crate::tl!("Changes apply at once and are kept for next time.")).small().color(Tokens::get().text_faint));
                });
            });
        });
    if close {
        *open = false;
    }
    if let Some(t) = next_tab {
        app.ui.dialog = Some(format!("options:{t}"));
    }
    if !changes.is_empty() {
        let _ = app.run("options", Value::Object(changes));
    }
    for (name, value) in extra.vars {
        let _ = app.run("setvar", json!({ "name": name, "value": value }));
    }
    for id in extra.toggles {
        let _ = app.run(id, Value::Null);
    }
    if let Some(id) = extra.start {
        app.start(id);
    }
}

/// A titled group box, as the Options tabs lay out their settings.
fn group(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new(title).strong());
        ui.add_space(2.0);
        add(ui);
    });
    ui.add_space(6.0);
}

/// A checkbox for an interface toggle (`ui.toggle.*` flips it).
fn ui_toggle(ui: &mut egui::Ui, on: bool, label: &str, id: &'static str, extra: &mut Extra) {
    let mut v = on;
    if ui.checkbox(&mut v, label).changed() {
        extra.toggles.push(id);
    }
}

fn grid(ui: &mut egui::Ui, id: &str, add: impl FnOnce(&mut egui::Ui)) {
    egui::Grid::new(id).num_columns(2).spacing(vec2(14.0, 8.0)).show(ui, add);
}

fn hint(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).small().color(Tokens::get().text_dim));
}

fn theme_label(t: ThemePref) -> &'static str {
    match t {
        ThemePref::System => crate::tl!("Use System Setting"),
        ThemePref::Light => crate::tl!("Light"),
        ThemePref::Dark => crate::tl!("Dark"),
    }
}

fn animations_label(a: AnimationPref) -> &'static str {
    match a {
        AnimationPref::System => crate::tl!("Use System Setting"),
        AnimationPref::On => crate::tl!("On"),
        AnimationPref::Off => crate::tl!("Off"),
    }
}

/// A colour button, its hex value and a Default button; a change goes into `changes` under `key`.
fn color_row(ui: &mut egui::Ui, current: Option<Color32>, default: Color32, key: &str, changes: &mut Map<String, Value>) {
    ui.horizontal(|ui| {
        let shown = current.unwrap_or(default);
        let mut rgb = [shown.r(), shown.g(), shown.b()];
        if ui.color_edit_button_srgb(&mut rgb).changed() {
            changes.insert(key.into(), json!(super::hex(Color32::from_rgb(rgb[0], rgb[1], rgb[2]))));
        }
        ui.monospace(super::hex(shown));
        if ui.add_enabled(current.is_some(), egui::Button::new(crate::tl!("Default"))).clicked() {
            changes.insert(key.into(), Value::Null);
        }
    });
}

/// A square of `side` points in `color` on the drawing-area background (marker and pickbox
/// previews).
fn preview_square(ui: &mut egui::Ui, side: f32, color: Color32) {
    let t = Tokens::get();
    let (rect, _) = ui.allocate_exact_size(vec2(36.0, 36.0), Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, 3.0, t.canvas);
    let side = side.clamp(1.0, 32.0);
    p.rect_stroke(egui::Rect::from_center_size(rect.center(), vec2(side, side)), 0.0, Stroke::new(2.0, color), egui::StrokeKind::Middle);
}

fn display(app: &CadApp, ui: &mut egui::Ui, changes: &mut Map<String, Value>, extra: &mut Extra) {
    ui.columns(2, |c| {
        let [left, right] = c else { return };
        group(left, crate::tl!("Window Elements"), |ui| {
            grid(ui, "options_display_window", |ui| {
                ui.label(crate::tl!("Color theme:"));
                let mut theme = app.ui.theme;
                egui::ComboBox::from_id_salt("options_theme").selected_text(theme_label(theme)).show_ui(ui, |ui| {
                    for t in ThemePref::ALL {
                        ui.selectable_value(&mut theme, t, theme_label(t));
                    }
                });
                if theme != app.ui.theme {
                    changes.insert("theme".into(), json!(theme.as_str()));
                }
                ui.end_row();
                ui.label(crate::tl!("Animation effects:"));
                let mut animations = app.ui.animations;
                egui::ComboBox::from_id_salt("options_animations").selected_text(animations_label(animations)).show_ui(ui, |ui| {
                    for a in AnimationPref::ALL {
                        ui.selectable_value(&mut animations, a, animations_label(a));
                    }
                });
                if animations != app.ui.animations {
                    changes.insert("animations".into(), json!(animations.as_str()));
                }
                ui.end_row();
            });
            ui_toggle(ui, app.ui.show_file_tabs, crate::tl!("Display File Tabs"), "ui.toggle.filetabs", extra);
            ui_toggle(ui, app.ui.show_status_bar, crate::tl!("Display status bar"), "ui.toggle.statusbar", extra);
            ui_toggle(ui, app.ui.in_window_menu, crate::tl!("Display menu bar"), "ui.toggle.menubar", extra);
            ui_toggle(ui, app.ui.show_toolbar, crate::tl!("Display toolbar"), "ui.toggle.toolbar", extra);
            ui_toggle(ui, app.ui.show_command_line, crate::tl!("Display command line"), "ui.toggle.cmdline", extra);
            let mut system_cursor = app.ui.system_cursor;
            if ui.checkbox(&mut system_cursor, crate::tl!("Keep the system cursor visible over the drawing area")).changed() {
                changes.insert("systemCursor".into(), json!(system_cursor));
            }
            ui.horizontal(|ui| {
                let mut lines = app.ui.history_lines;
                if ui.add(egui::DragValue::new(&mut lines).range(0..=crate::cmdline::MAX_LINES)).changed() {
                    changes.insert("historyLines".into(), json!(lines));
                }
                ui.label(crate::tl!("Command line history lines"));
            });
        });
        group(left, crate::tl!("Colors"), |ui| {
            grid(ui, "options_display_colors", |ui| {
                ui.label(crate::tl!("Model background"));
                color_row(ui, app.options.model_background, Tokens::DARK.canvas, "modelBackground", changes);
                ui.end_row();
                ui.label(crate::tl!("Layout paper"));
                color_row(ui, app.options.paper_background, Tokens::DARK.paper, "paperBackground", changes);
                ui.end_row();
            });
        });
        group(right, crate::tl!("Display resolution"), |ui| {
            let segs = app.session.doc().ok().map_or(8, |d| d.header.i64("SPLINESEGS", 8));
            ui.horizontal(|ui| {
                let mut v = segs;
                if ui.add(egui::DragValue::new(&mut v).range(1..=32_767)).changed() {
                    extra.vars.push(("SPLINESEGS", json!(v)));
                }
                ui.label(crate::tl!("Segments in a polyline curve"));
            });
        });
        group(right, crate::tl!("Display performance"), |ui| {
            let mut lw = app.session.settings.lwdisplay;
            if ui.checkbox(&mut lw, crate::tl!("Display lineweight")).changed() {
                changes.insert("LWDISPLAY".into(), json!(i32::from(lw)));
            }
        });
        group(right, crate::tl!("Crosshair size"), |ui| {
            let mut size = app.session.settings.cursorsize;
            if ui.add(egui::Slider::new(&mut size, 1.0..=100.0).integer()).changed() {
                changes.insert("CURSORSIZE".into(), json!(size));
            }
        });
    });
}

/// Files: where automatic saves go and the template QNEW starts from.
fn files(_app: &CadApp, ui: &mut egui::Ui, changes: &mut Map<String, Value>, extra: &mut Extra) {
    ui.label(crate::tl!("Search paths, file names, and file locations:"));
    ui.add_space(4.0);
    group(ui, crate::tl!("Automatic Save File Location"), |ui| {
        // Edited as text; the folder changes when the field loses focus.
        let id = egui::Id::new("cc_options_savefilepath");
        let current = cadcraft_engine::autosave::savefilepath();
        let mut path = ui.data(|d| d.get_temp::<String>(id)).unwrap_or_else(|| current.clone());
        let r = ui.add(egui::TextEdit::singleline(&mut path).desired_width(f32::INFINITY));
        if r.changed() {
            ui.data_mut(|d| d.insert_temp(id, path.clone()));
        }
        if r.lost_focus() {
            ui.data_mut(|d| d.remove::<String>(id));
            if path.trim() != current {
                changes.insert("SAVEFILEPATH".into(), json!(path.trim()));
            }
        }
    });
    group(ui, crate::tl!("Default Template File Name for QNEW"), |ui| {
        let id = egui::Id::new("cc_options_qnew");
        let current = cadcraft_engine::cmd::file::qnew_template();
        let mut path = ui.data(|d| d.get_temp::<String>(id)).unwrap_or_else(|| current.clone());
        let r = ui.add(egui::TextEdit::singleline(&mut path).desired_width(f32::INFINITY).hint_text(crate::tl!("None")));
        if r.changed() {
            ui.data_mut(|d| d.insert_temp(id, path.clone()));
        }
        if r.lost_focus() {
            ui.data_mut(|d| d.remove::<String>(id));
            if path.trim() != current {
                extra.vars.push(("QNEWTEMPLATE", json!(path.trim())));
            }
        }
    });
}

/// The Save As choices: (format, version, label).
fn save_as_choices() -> Vec<(crate::SaveFormat, crate::SaveVersion, String)> {
    let mut v = Vec::new();
    for f in [crate::SaveFormat::Dwg, crate::SaveFormat::Dxf] {
        for ver in crate::SaveVersion::ALL.iter().rev() {
            let kind = match f {
                crate::SaveFormat::Dwg => "Drawing (*.dwg)",
                crate::SaveFormat::Dxf => "DXF (*.dxf)",
            };
            v.push((f, *ver, format!("AutoCAD {} {kind}", ver.year())));
        }
    }
    v
}

fn open_save(app: &CadApp, ui: &mut egui::Ui, changes: &mut Map<String, Value>) {
    ui.columns(2, |c| {
        let [left, right] = c else { return };
        group(left, crate::tl!("File Save"), |ui| {
            ui.label(crate::tl!("Save as:"));
            let version = crate::SaveVersion::parse(&app.ui.save_version).unwrap_or_default();
            let choices = save_as_choices();
            let shown = choices.iter().find(|(f, v, _)| *f == app.ui.save_format && *v == version).map_or_else(String::new, |c| c.2.clone());
            egui::ComboBox::from_id_salt("options_save_as").width(260.0).selected_text(shown).show_ui(ui, |ui| {
                for (f, v, label) in &choices {
                    if ui.selectable_label(*f == app.ui.save_format && *v == version, label).clicked() {
                        changes.insert("saveFormat".into(), json!(f.as_str()));
                        changes.insert("saveVersion".into(), json!(v.year()));
                    }
                }
            });
            hint(ui, crate::tl!("A drawing opened from a file keeps that file's format."));
        });
        group(left, crate::tl!("File Safety Precautions"), |ui| {
            let minutes = cadcraft_engine::autosave::savetime();
            let mut on = minutes > 0;
            if ui.checkbox(&mut on, crate::tl!("Automatic save")).changed() {
                changes.insert("SAVETIME".into(), json!(if on { 10 } else { 0 }));
            }
            ui.add_enabled_ui(on, |ui| {
                ui.horizontal(|ui| {
                    ui.add_space(20.0);
                    let mut m = minutes.max(1);
                    let max = cadcraft_engine::autosave::MAX_SAVETIME;
                    if ui.add(egui::DragValue::new(&mut m).range(1..=max)).changed() {
                        changes.insert("SAVETIME".into(), json!(m));
                    }
                    ui.label(crate::tl!("Minutes between saves"));
                });
            });
        });
        group(right, crate::tl!("File Open"), |ui| {
            ui.label(format!("{} {}", crate::RECENT_FILES, crate::tl!("Number of recently-used files")));
        });
    });
}

fn plot(ui: &mut egui::Ui, extra: &mut Extra) {
    ui.columns(2, |c| {
        let [left, right] = c else { return };
        group(left, crate::tl!("Default plot settings for new drawings"), |ui| {
            if ui.button(crate::tl!("Page Setup Manager...")).clicked() {
                extra.start = Some("pagesetup");
            }
        });
        group(right, crate::tl!("General plot options"), |ui| {
            if ui.button(crate::tl!("Plot Style Table Settings...")).clicked() {
                extra.start = Some("stylesmanager");
            }
        });
    });
}

fn system(app: &CadApp, ui: &mut egui::Ui, changes: &mut Map<String, Value>) {
    ui.columns(2, |c| {
        let [left, right] = c else { return };
        group(left, crate::tl!("General Options"), |ui| {
            grid(ui, "options_system", |ui| {
                ui.label(crate::tl!("Interface language"));
                let mut lang = app.ui.interface_language;
                egui::ComboBox::from_id_salt("options_language").selected_text(lang.name()).show_ui(ui, |ui| {
                    for p in crate::i18n::Preference::ALL {
                        ui.selectable_value(&mut lang, p, p.name());
                    }
                });
                if lang != app.ui.interface_language {
                    changes.insert("interfaceLanguage".into(), json!(lang.code()));
                }
                ui.end_row();
            });
        });
        group(right, crate::tl!("Arrays"), |ui| {
            ui.horizontal(|ui| {
                let mut max = app.session.settings.maxarray;
                if ui.add(egui::DragValue::new(&mut max).range(100..=10_000_000).speed(1000.0)).changed() {
                    changes.insert("MAXARRAY".into(), json!(max));
                }
                ui.label(crate::tl!("Largest array (objects)"));
            });
            hint(ui, crate::tl!("An array that would create more objects than this is refused."));
        });
    });
}

fn drafting(app: &CadApp, ui: &mut egui::Ui, changes: &mut Map<String, Value>, extra: &mut Extra) {
    let t = Tokens::get();
    let st = &app.session.settings;
    ui.columns(2, |c| {
        let [left, right] = c else { return };
        group(left, crate::tl!("AutoSnap Settings"), |ui| {
            let mut marker = app.options.autosnap_marker;
            if ui.checkbox(&mut marker, crate::tl!("Marker")).changed() {
                changes.insert("autosnapMarker".into(), json!(marker));
            }
            let mut apbox = app.options.aperture_box;
            if ui.checkbox(&mut apbox, crate::tl!("Display AutoSnap aperture box")).changed() {
                changes.insert("apertureBox".into(), json!(apbox));
            }
        });
        group(left, crate::tl!("AutoSnap Marker Size"), |ui| {
            ui.horizontal(|ui| {
                preview_square(ui, app.options.snap_marker_size, t.snap);
                let mut size = app.options.snap_marker_size;
                if ui.add(egui::Slider::new(&mut size, super::SNAP_MARKER_SIZES).integer()).changed() {
                    changes.insert("snapMarkerSize".into(), json!(size));
                }
            });
        });
        group(left, crate::tl!("Object Snap Options"), |ui| {
            let mut ignore = !st.osnaphatch;
            if ui.checkbox(&mut ignore, crate::tl!("Ignore hatch objects")).changed() {
                changes.insert("OSNAPHATCH".into(), json!(i32::from(!ignore)));
            }
        });
        group(right, crate::tl!("Aperture Size"), |ui| {
            ui.horizontal(|ui| {
                preview_square(ui, st.aperture as f32, t.crosshair);
                let mut aperture = st.aperture;
                if ui.add(egui::Slider::new(&mut aperture, 1.0..=50.0).integer()).changed() {
                    changes.insert("APERTURE".into(), json!(aperture));
                }
            });
            hint(ui, crate::tl!("The aperture is how near the cursor must come to a point for an object snap to find it."));
        });
        right.add_space(4.0);
        if right.button(crate::tl!("Drafting Settings...")).clicked() {
            extra.start = Some("dsettings");
        }
    });
}

fn modeling(app: &CadApp, ui: &mut egui::Ui, extra: &mut Extra) {
    ui.columns(2, |c| {
        let [left, _] = c else { return };
        group(left, crate::tl!("Display ViewCube or UCS Icon"), |ui| {
            ui_toggle(ui, app.ui.show_viewcube, crate::tl!("Display the ViewCube"), "ui.toggle.viewcube", extra);
            ui_toggle(ui, app.ui.show_ucs_icon, crate::tl!("Display the UCS Icon"), "ui.toggle.ucsicon", extra);
        });
    });
}

fn selection(app: &CadApp, ui: &mut egui::Ui, changes: &mut Map<String, Value>) {
    let t = Tokens::get();
    let st = &app.session.settings;
    let mut effects = ui.ctx().data(|d| d.get_temp::<bool>(egui::Id::new(VISUAL_EFFECTS)).unwrap_or(false));
    ui.columns(2, |cols| {
        let [left, right] = cols else { return };
        // Left: how objects are picked.
        heading(left, crate::tl!("Pickbox size"));
        left.horizontal(|ui| {
            let mut pickbox = st.pickbox;
            if ui.add(egui::Slider::new(&mut pickbox, 0.0..=50.0).integer().suffix(" px")).changed() {
                changes.insert("PICKBOX".into(), json!(pickbox));
            }
            preview_square(ui, (pickbox * 3.0) as f32, t.crosshair);
        });
        left.add_space(8.0);
        heading(left, crate::tl!("Selection modes"));
        let mut pickfirst = st.pickfirst;
        if left.checkbox(&mut pickfirst, crate::tl!("Noun/verb selection")).changed() {
            changes.insert("PICKFIRST".into(), json!(i32::from(pickfirst)));
        }
        let mut shift_adds = !st.pickadd;
        if left.checkbox(&mut shift_adds, crate::tl!("Use Shift to add to selection")).changed() {
            changes.insert("PICKADD".into(), json!(i32::from(!shift_adds)));
        }
        let mut grouping = st.pickstyle & 1 != 0;
        if left.checkbox(&mut grouping, crate::tl!("Object grouping")).changed() {
            changes.insert("PICKSTYLE".into(), json!((st.pickstyle & !1) | u8::from(grouping)));
        }
        let mut assoc_hatch = st.pickstyle & 2 != 0;
        if left.checkbox(&mut assoc_hatch, crate::tl!("Associative Hatch")).changed() {
            changes.insert("PICKSTYLE".into(), json!((st.pickstyle & !2) | (u8::from(assoc_hatch) << 1)));
        }
        let mut qp = st.qpmode;
        if left.checkbox(&mut qp, crate::tl!("Show Quick Properties for selected objects")).changed() {
            changes.insert("QPMODE".into(), json!(i32::from(qp)));
        }
        left.add_space(4.0);
        left.label(crate::tl!("Selection effect color"));
        color_row(left, app.options.selection_color, crate::theme::Tokens::of(crate::theme::active()).selection, "selectionColor", changes);

        // Right: grips and the preview under the cursor.
        heading(right, crate::tl!("Grip size"));
        right.horizontal(|ui| {
            let mut grip = st.gripsize;
            if ui.add(egui::Slider::new(&mut grip, 1.0..=20.0).integer().suffix(" px")).changed() {
                changes.insert("GRIPSIZE".into(), json!(grip));
            }
            preview_square(ui, (grip * 2.0) as f32, t.grip);
        });
        right.add_space(8.0);
        heading(right, crate::tl!("Grips"));
        let mut grips = st.grips > 0;
        if right.checkbox(&mut grips, crate::tl!("Show grips")).changed() {
            changes.insert("GRIPS".into(), json!(u8::from(grips)));
        }
        right.horizontal(|ui| {
            let mut limit = st.gripobjlimit;
            if ui.add_enabled(st.grips > 0, egui::DragValue::new(&mut limit).range(0..=32_767)).changed() {
                changes.insert("GRIPOBJLIMIT".into(), json!(limit));
            }
            ui.label(crate::tl!("Object selection limit for display of grips"));
        });
        right.add_space(8.0);
        heading(right, crate::tl!("Preview"));
        right.label(crate::tl!("Selection preview"));
        let (mut asking, mut idle) = (st.selectionpreview & 2 != 0, st.selectionpreview & 1 != 0);
        let a = right.checkbox(&mut asking, crate::tl!("When a command is active")).changed();
        let b = right.checkbox(&mut idle, crate::tl!("When no command is active")).changed();
        if a || b {
            changes.insert("SELECTIONPREVIEW".into(), json!(u8::from(idle) | (u8::from(asking) << 1)));
        }
        if right.button(crate::tl!("Visual Effect Settings...")).clicked() {
            effects = true;
        }
    });
    if effects {
        visual_effects(ui.ctx(), st, &mut effects, changes);
    }
    ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(VISUAL_EFFECTS), effects));
}

/// Whether the Visual Effect Settings window is open (egui temp data).
const VISUAL_EFFECTS: &str = "cc_options_visual_effects";

fn heading(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).strong());
}

/// Visual Effect Settings: the selection area (window and crossing fills) and the objects the
/// cursor never highlights (PREVIEWFILTER). Changes apply at once, as on the Options tabs.
fn visual_effects(ctx: &egui::Context, st: &cadcraft_engine::Settings, open: &mut bool, changes: &mut Map<String, Value>) {
    let mut close = false;
    egui::Window::new(crate::tl!("Visual Effect Settings"))
        .id(egui::Id::new("cc_visual_effects"))
        .open(open)
        .resizable(false)
        .collapsible(false)
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center())
        .show(ctx, |ui| {
            ui.columns(2, |cols| {
                let [left, right] = cols else { return };
                heading(left, crate::tl!("Selection Area Effect"));
                let mut area = st.selectionarea;
                if left.checkbox(&mut area, crate::tl!("Indicate selection area")).changed() {
                    changes.insert("SELECTIONAREA".into(), json!(i32::from(area)));
                }
                left.add_enabled_ui(st.selectionarea, |ui| {
                    ui.label(crate::tl!("Window selection color:"));
                    aci_row(ui, st.windowareacolor, "WINDOWAREACOLOR", changes);
                    ui.label(crate::tl!("Crossing selection color:"));
                    aci_row(ui, st.crossingareacolor, "CROSSINGAREACOLOR", changes);
                    ui.label(crate::tl!("Selection area opacity:"));
                    let mut opacity = st.selectionareaopacity;
                    if ui.add(egui::Slider::new(&mut opacity, 0..=100)).changed() {
                        changes.insert("SELECTIONAREAOPACITY".into(), json!(opacity));
                    }
                });

                heading(right, crate::tl!("Selection Preview Filter"));
                right.label(crate::tl!("Exclude:"));
                for (bit, label) in [
                    (1u8, crate::tl!("Objects on locked layers")),
                    (2, crate::tl!("Xrefs")),
                    (4, crate::tl!("Tables")),
                    (32, crate::tl!("Groups")),
                    (8, crate::tl!("Multiline text")),
                    (16, crate::tl!("Hatches")),
                ] {
                    let mut on = st.previewfilter & bit != 0;
                    if right.checkbox(&mut on, label).changed() {
                        changes.insert("PREVIEWFILTER".into(), json!(if on { st.previewfilter | bit } else { st.previewfilter & !bit }));
                    }
                }
            });
            ui.separator();
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(crate::tl!("Close")).clicked() {
                        close = true;
                    }
                });
            });
        });
    if close {
        *open = false;
    }
}

/// A colour by its index (ACI 1-255): a swatch and the number; a change goes into `changes`.
fn aci_row(ui: &mut egui::Ui, current: u8, key: &str, changes: &mut Map<String, Value>) {
    ui.horizontal(|ui| {
        let c = cadcraft_color::aci_rgb(current);
        let (r, _) = ui.allocate_exact_size(vec2(16.0, 16.0), egui::Sense::hover());
        ui.painter().rect_filled(r, 2.0, Color32::from_rgb(c.0, c.1, c.2));
        let mut v = current;
        if ui.add(egui::DragValue::new(&mut v).range(1..=255).prefix(format!("{} ", crate::tl!("Color")))).changed() {
            changes.insert(key.into(), json!(v));
        }
    });
}

fn preferences(app: &CadApp, ui: &mut egui::Ui, changes: &mut Map<String, Value>, extra: &mut Extra) {
    let rc_id = egui::Id::new("cc_options_right_click");
    let mut rc_open = ui.ctx().data(|d| d.get_temp::<bool>(rc_id).unwrap_or(false));
    ui.columns(2, |c| {
        let [left, right] = c else { return };
        group(left, crate::tl!("Windows Standard Behavior"), |ui| {
            let mut dbl = app.options.double_click_edit;
            if ui.checkbox(&mut dbl, crate::tl!("Double click editing")).changed() {
                changes.insert("doubleClickEdit".into(), json!(dbl));
            }
            if ui.button(crate::tl!("Right-click Customization...")).clicked() {
                rc_open = true;
            }
        });
        group(left, crate::tl!("Mouse wheel"), |ui| {
            let reverse = app.session.settings.zoomwheel;
            let label = |r: bool| if r { crate::tl!("Reversed: wheel forward zooms out") } else { crate::tl!("Normal: wheel forward zooms in") };
            let mut picked = reverse;
            egui::ComboBox::from_id_salt("options_zoomwheel").width(240.0).selected_text(label(reverse)).show_ui(ui, |ui| {
                for r in [false, true] {
                    ui.selectable_value(&mut picked, r, label(r));
                }
            });
            if picked != reverse {
                changes.insert("ZOOMWHEEL".into(), json!(i32::from(picked)));
            }
            ui.label(crate::tl!("Mouse wheel zoom speed"));
            let mut factor = app.session.settings.zoomfactor;
            if ui.add(egui::Slider::new(&mut factor, 3..=100)).changed() {
                changes.insert("ZOOMFACTOR".into(), json!(factor));
            }
        });
        group(right, crate::tl!("Associative Dimensioning"), |ui| {
            let assoc = app.session.doc().ok().is_some_and(|d| d.header.i64("DIMASSOC", 2) != 0);
            let mut on = assoc;
            if ui.checkbox(&mut on, crate::tl!("Make new dimensions associative")).changed() {
                extra.vars.push(("DIMASSOC", json!(if on { 2 } else { 0 })));
            }
        });
    });
    if rc_open {
        right_click_window(app, ui.ctx(), &mut rc_open, changes);
    }
    ui.ctx().data_mut(|d| d.insert_temp(rc_id, rc_open));
}

/// Right-Click Customization: what a right-click on the drawing area does.
fn right_click_window(app: &CadApp, ctx: &egui::Context, open: &mut bool, changes: &mut Map<String, Value>) {
    let mut close = false;
    egui::Window::new(crate::tl!("Right-Click Customization"))
        .id(egui::Id::new("cc_right_click"))
        .open(open)
        .resizable(false)
        .collapsible(false)
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center())
        .show(ctx, |ui| {
            let o = &app.options;
            let mut ts = o.time_sensitive_right_click;
            if ui.checkbox(&mut ts, crate::tl!("Turn on time-sensitive right-click")).changed() {
                changes.insert("timeSensitiveRightClick".into(), json!(ts));
                if !ts {
                    changes.insert("rightClickRepeat".into(), json!(false));
                }
            }
            hint(ui, crate::tl!("Quick click for Enter; longer click to display the shortcut menu."));
            ui.add_space(6.0);
            group(ui, crate::tl!("Default Mode"), |ui| {
                ui.label(crate::tl!("If no objects are selected, right-click means"));
                ui.add_enabled_ui(o.time_sensitive_right_click, |ui| {
                    if ui.radio(o.right_click_repeat, crate::tl!("Repeat Last Command")).clicked() {
                        changes.insert("rightClickRepeat".into(), json!(true));
                    }
                });
                if ui.radio(!o.right_click_repeat, crate::tl!("Shortcut Menu")).clicked() {
                    changes.insert("rightClickRepeat".into(), json!(false));
                }
            });
            group(ui, crate::tl!("Command Mode"), |ui| {
                ui.label(crate::tl!("If a command is in progress, right-click means"));
                if ui.radio(o.time_sensitive_right_click, crate::tl!("ENTER")).clicked() {
                    changes.insert("timeSensitiveRightClick".into(), json!(true));
                }
                if ui.radio(!o.time_sensitive_right_click, crate::tl!("Shortcut Menu")).clicked() {
                    changes.insert("timeSensitiveRightClick".into(), json!(false));
                    changes.insert("rightClickRepeat".into(), json!(false));
                }
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(crate::tl!("Apply & Close")).clicked() {
                    close = true;
                }
            });
        });
    if close {
        *open = false;
    }
}

fn profiles(app: &CadApp, ui: &mut egui::Ui, changes: &mut Map<String, Value>) {
    ui.label(crate::tl!(
        "CADCraft keeps these preferences, object snap modes and drafting toggles for your user account and restores them when it starts."
    ));
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        if ui.button(crate::tl!("Reset to Defaults")).clicked() {
            changes.insert("reset".into(), json!(true));
        }
        if ui.button(crate::tl!("Copy as JSON")).clicked() {
            ui.ctx().copy_text(serde_json::to_string_pretty(&super::report(app)).unwrap_or_default());
        }
    });
    hint(ui, crate::tl!("Reset keeps the interface language."));
}
