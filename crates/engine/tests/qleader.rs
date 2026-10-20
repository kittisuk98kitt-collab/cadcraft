//! QLEADER and its Leader Settings.

use cadcraft_engine::Session;
use cadcraft_engine::doc::EntityKind;
use serde_json::json;

fn typed(s: &mut Session, lines: &[&str]) {
    for l in lines {
        s.cmdline(l).unwrap();
    }
}

fn kinds(s: &Session) -> Vec<EntityKind> {
    s.doc().unwrap().model.iter().map(|e| e.kind.clone()).collect()
}

#[test]
fn settings_option_asks_for_the_dialog() {
    let mut s = Session::new();
    typed(&mut s, &["qleader", "s"]);
    assert!(s.qleader_dialog);
    assert!(s.running.is_some());
}

#[test]
fn three_points_then_width_and_lines() {
    let mut s = Session::new();
    typed(&mut s, &["qleader", "0,0", "10,10", "20,10", "", "HELLO", "WORLD", ""]);
    assert!(s.running.is_none());
    let k = kinds(&s);
    let leader = k.iter().find_map(|k| if let EntityKind::Leader(l) = k { Some(l.clone()) } else { None }).unwrap();
    assert_eq!(leader.vertices.len(), 3);
    let mt = k.iter().find_map(|k| if let EntityKind::MText(t) = k { Some(t.clone()) } else { None }).unwrap();
    assert_eq!(mt.contents, "HELLO\\PWORLD");
    // Text on the right, middle of top line: attached top left.
    assert_eq!(mt.attach, 1);
}

#[test]
fn settings_drive_the_leader() {
    let mut s = Session::new();
    s.execute("qleader.settings", &json!({ "annotation": "none", "spline": true, "arrow": "Dot", "maxPoints": 0, "angle1": -1, "underline": true }))
        .unwrap();
    typed(&mut s, &["qleader", "0,0", "10,7", "20,20", "30,25", ""]);
    let k = kinds(&s);
    assert_eq!(k.len(), 1, "None: no annotation");
    let EntityKind::Leader(l) = &k[0] else { panic!("not a leader") };
    assert!(l.spline);
    assert_eq!(l.vertices.len(), 4);
    // Horizontal first segment.
    assert!((l.vertices[1].y - 0.0).abs() < 1e-9);
    assert_eq!(l.arrow_block.as_deref(), Some("Dot"));
}

#[test]
fn text_on_the_left_uses_the_left_attachment() {
    let mut s = Session::new();
    s.execute("qleader.settings", &json!({ "promptWidth": false, "attachLeft": 2, "frame": true })).unwrap();
    typed(&mut s, &["qleader", "0,0", "-10,10", "", "A", ""]);
    let k = kinds(&s);
    let mt = k.iter().find_map(|k| if let EntityKind::MText(t) = k { Some(t.clone()) } else { None }).unwrap();
    assert_eq!(mt.attach, 6, "middle right");
    assert!(k.iter().any(|k| matches!(k, EntityKind::LwPolyline(_))), "frame");
}

#[test]
fn reuse_next_then_current() {
    let mut s = Session::new();
    s.execute("qleader.settings", &json!({ "promptWidth": false, "reuse": "next" })).unwrap();
    typed(&mut s, &["qleader", "0,0", "10,10", "", "R1", ""]);
    assert_eq!(s.qleader.reuse, cadcraft_engine::QReuse::Current);
    typed(&mut s, &["qleader", "0,20", "10,30", ""]);
    let n = kinds(&s).iter().filter(|k| matches!(k, EntityKind::MText(t) if t.contents == "R1")).count();
    assert_eq!(n, 2);
}

#[test]
fn associative_hatch_pickstyle_selects_the_boundary() {
    let mut s = Session::new();
    s.viewport_px = (1200.0, 800.0);
    let c = s.execute("circle", &json!({ "center": [0, 0], "radius": 10 })).unwrap();
    let ch = c["handle"].as_str().or_else(|| c["handles"][0].as_str()).unwrap().to_string();
    s.execute("hatch", &json!({ "handles": [ch], "pattern": "SOLID", "associative": true })).unwrap();
    let hatch = s.doc().unwrap().model.iter().find(|e| matches!(e.kind, EntityKind::Hatch(_))).map(|e| e.handle).unwrap();
    s.execute("zoom", &json!({ "mode": "extents" })).unwrap();
    s.settings.pickstyle = 2;
    // Pick inside the solid fill, away from the circle.
    s.idle_click(cadcraft_engine::geom::Vec2::new(0.0, 0.0), false).unwrap();
    let sel = s.selection();
    assert!(sel.contains(&hatch));
    assert_eq!(sel.len(), 2, "the hatch and its boundary circle");
}

#[test]
fn action_recorder_records_and_plays() {
    let mut s = Session::new();
    typed(&mut s, &["actrecord", "line", "0,0", "10,0", "", "actstop", "TWO"]);
    assert!(s.actions.contains_key("TWO"));
    let lines = |s: &Session| s.doc().unwrap().model.iter().filter(|e| matches!(e.kind, EntityKind::Line(_))).count();
    assert_eq!(lines(&s), 1);
    typed(&mut s, &["play", "TWO"]);
    assert_eq!(lines(&s), 2);
    s.execute("play", &json!({ "name": "two" })).unwrap();
    assert_eq!(lines(&s), 3);
}

#[test]
fn blockreplace_txt2mtxt_and_laytrans() {
    let mut s = Session::new();
    let h = |v: serde_json::Value| v["handle"].as_str().or_else(|| v["handles"][0].as_str()).unwrap().to_string();
    let a = h(s.execute("line", &json!({ "points": [[0, 0], [1, 0]] })).unwrap());
    s.execute("block", &json!({ "name": "A", "base": [0, 0], "handles": [a], "keep": "delete" })).unwrap();
    let b = h(s.execute("circle", &json!({ "center": [0, 0], "radius": 1 })).unwrap());
    s.execute("block", &json!({ "name": "B", "base": [0, 0], "handles": [b], "keep": "delete" })).unwrap();
    s.execute("insert", &json!({ "name": "A", "at": [5, 5] })).unwrap();
    s.execute("insert", &json!({ "name": "A", "at": [9, 5] })).unwrap();
    let r = s.execute("blockreplace", &json!({ "old": "a", "new": "B" })).unwrap();
    assert_eq!(r["replaced"], 2);
    assert!(s.doc().unwrap().model.iter().all(|e| !matches!(&e.kind, EntityKind::Insert(i) if i.block == "A")));

    let t1 = h(s.execute("text", &json!({ "at": [0, 10], "height": 2.5, "text": "FIRST" })).unwrap());
    let t2 = h(s.execute("text", &json!({ "at": [0, 5], "height": 2.5, "text": "SECOND" })).unwrap());
    s.execute("txt2mtxt", &json!({ "handles": [t2, t1] })).unwrap();
    let mt: Vec<String> =
        s.doc().unwrap().model.iter().filter_map(|e| if let EntityKind::MText(m) = &e.kind { Some(m.contents.clone()) } else { None }).collect();
    assert_eq!(mt, vec!["FIRST\\PSECOND".to_string()]);

    s.execute("layer.new", &json!({ "name": "OLD", "color": 3 })).unwrap();
    s.execute("laytrans", &json!({ "map": { "OLD": "NEW" } })).unwrap();
    let d = s.doc().unwrap();
    assert!(d.layer("OLD").is_none() && d.layer("NEW").is_some());
}

#[test]
fn spring_draws_a_block_and_reports_its_rate() {
    for view in ["full", "section", "simplified"] {
        let mut s = Session::new();
        let r = s.execute("spring", &json!({ "at": [0, 0], "wire": 2, "od": 20, "length": 50, "coils": 6, "view": view })).unwrap();
        // k = G d^4 / (8 D^3 n) = 79300 * 16 / (8 * 5832 * 6)
        assert!((r["rate"].as_f64().unwrap() - 79_300.0 * 16.0 / (8.0 * 5832.0 * 6.0)).abs() < 1e-9);
        let name = r["block"].as_str().unwrap().to_string();
        let d = s.doc().unwrap();
        assert!(d.block(&name).is_some_and(|b| b.entities.len() > 3), "{view}");
        assert!(d.model.iter().any(|e| matches!(&e.kind, EntityKind::Insert(i) if i.block == name)));
    }
    let mut s = Session::new();
    assert!(s.execute("spring", &json!({ "at": [0, 0], "wire": 5, "od": 20, "length": 20, "coils": 6 })).is_err(), "solid");
}

#[test]
fn spring_typed() {
    let mut s = Session::new();
    typed(&mut s, &["spring", "0,0", "60,0", "25", "2.5", "7", "C", "S"]);
    assert!(s.running.is_none());
    assert!(s.doc().unwrap().blocks.keys().any(|k| k.starts_with("SPRING_2_5x25x60")));
}

#[test]
fn tapped_holes_from_the_jis_tables() {
    let mut s = Session::new();
    let r = s.execute("taphole", &json!({ "at": [[0, 0], [30, 0]], "kind": "M", "size": "M8" })).unwrap();
    assert_eq!(r["drill"], 6.8);
    assert_eq!(r["handles"].as_array().unwrap().len(), 2);
    let r = s.execute("taphole", &json!({ "at": [0, 50], "kind": "G", "size": "1/4", "view": "side" })).unwrap();
    assert_eq!(r["drill"], 11.8);
    assert!(r["block"].as_str().unwrap().starts_with("HOLE_G1-4_SIDE"));
    let r = s
        .execute("taphole", &json!({ "at": [0, 90], "kind": "PT", "size": "Rc1/2", "view": "side", "through": true, "thickness": 25, "note": true }))
        .unwrap();
    assert_eq!(r["drill"], 18.3);
    let r = s.execute("taphole", &json!({ "at": [0, 130], "kind": "MF", "size": "M10x1.25" })).unwrap();
    assert_eq!(r["drill"], 8.8);
    assert!(s.execute("taphole", &json!({ "at": [0, 0], "kind": "M", "size": "M7" })).is_err());
    let st = s.execute("taphole.settings", &json!({ "kind": "pt" })).unwrap();
    assert_eq!(st["kind"], "PT");
    assert!(st["sizes"].as_array().unwrap().len() >= 10);
}

#[test]
fn taphole_typed_settings_and_points() {
    let mut s = Session::new();
    typed(&mut s, &["taphole", "s"]);
    assert!(s.hole_dialog);
    typed(&mut s, &["10,10", "20,10", ""]);
    assert!(s.running.is_none());
    let n = s.doc().unwrap().model.iter().filter(|e| matches!(&e.kind, EntityKind::Insert(i) if i.block.starts_with("HOLE_M8"))).count();
    assert_eq!(n, 2);
}

#[test]
fn underline_puts_the_text_on_the_landing() {
    let mut s = Session::new();
    s.execute("qleader.settings", &json!({ "promptWidth": false, "underline": true })).unwrap();
    typed(&mut s, &["qleader", "0,0", "10,10", "", "ABC", ""]);
    let k = kinds(&s);
    let l = k.iter().find_map(|k| if let EntityKind::Leader(l) = k { Some(l.clone()) } else { None }).unwrap();
    let mt = k.iter().find_map(|k| if let EntityKind::MText(t) = k { Some(t.clone()) } else { None }).unwrap();
    assert!(!mt.contents.contains("\\L"), "not an underline format code");
    assert_eq!(mt.attach, 7);
    let last = l.vertices.last().unwrap();
    assert!((last.y - 10.0).abs() < 1e-9 && last.x > mt.insert.x, "the landing runs on under the text");
    assert!(mt.insert.y > 10.0, "the text sits above the line");
}

#[test]
fn stretch_uses_the_pickfirst_crossing_window() {
    let mut s = Session::new();
    s.viewport_px = (1200.0, 800.0);
    s.execute("line", &json!({ "points": [[0, 0], [10, 0]] })).unwrap();
    s.execute("zoom", &json!({ "mode": "extents" })).unwrap();
    // Crossing window (right to left) around the right end only.
    s.idle_click(cadcraft_engine::geom::Vec2::new(12.0, 2.0), false).unwrap();
    s.idle_click(cadcraft_engine::geom::Vec2::new(8.0, -2.0), false).unwrap();
    assert_eq!(s.selection().len(), 1);
    typed(&mut s, &["stretch", "0,0", "5,0"]);
    let k = kinds(&s);
    let EntityKind::Line(l) = &k[0] else { panic!("line") };
    assert!((l.a.x - 0.0).abs() < 1e-9 && (l.b.x - 15.0).abs() < 1e-9, "stretched, not moved: {:?} {:?}", l.a, l.b);
}

#[test]
fn stretch_box_selection_stretches() {
    let mut s = Session::new();
    s.viewport_px = (1200.0, 800.0);
    s.execute("line", &json!({ "points": [[0, 0], [10, 0]] })).unwrap();
    s.execute("zoom", &json!({ "mode": "extents" })).unwrap();
    s.start("stretch").unwrap();
    // What the canvas sends for a press-and-drag from right to left.
    s.input(cadcraft_engine::Input::Keyword("BOX".into())).unwrap();
    s.input(cadcraft_engine::Input::Point(cadcraft_engine::geom::Vec2::new(12.0, 2.0))).unwrap();
    s.input(cadcraft_engine::Input::Point(cadcraft_engine::geom::Vec2::new(8.0, -2.0))).unwrap();
    typed(&mut s, &["", "0,0", "5,0"]);
    let k = kinds(&s);
    let EntityKind::Line(l) = &k[0] else { panic!("line") };
    assert!((l.a.x - 0.0).abs() < 1e-9 && (l.b.x - 15.0).abs() < 1e-9, "{:?} {:?}", l.a, l.b);
}

#[test]
fn pipe_runs_fittings_tees_and_edit() {
    let mut s = Session::new();
    s.viewport_px = (1200.0, 800.0);
    let r =
        s.execute("pipe", &json!({ "points": [[0, 0], [1000, 0], [1000, 800]], "size": "50A", "schedule": "SCH40", "flangeStart": true })).unwrap();
    let run = r["run"].as_str().unwrap().to_string();
    // The elbow's arcs: inside, outside and centre line.
    assert_eq!(kinds(&s).iter().filter(|k| matches!(k, EntityKind::Arc(_))).count(), 3);
    s.execute("zoom", &json!({ "mode": "extents" })).unwrap();
    // A branch starting on the run puts a tee on it.
    let b = s.execute("pipe", &json!({ "points": [[500, 0], [500, -600]] })).unwrap();
    assert_eq!(b["fromTee"], true);
    let first = s.doc().unwrap().groups.iter().find(|g| g.name == run).unwrap().members[0];
    let info = s.execute("pipe.run", &json!({ "handle": first.hex() })).unwrap();
    assert_eq!(info["tees"].as_array().unwrap().len(), 1);
    // PIPEEDIT redraws the run at another size.
    s.execute("pipeedit", &json!({ "handle": first.hex(), "size": "80A" })).unwrap();
    let first = s.doc().unwrap().groups.iter().find(|g| g.name == run).unwrap().members[0];
    let info = s.execute("pipe.run", &json!({ "handle": first.hex() })).unwrap();
    assert_eq!(info["size"], "80A");
    assert!(s.execute("pipe", &json!({ "points": [[0, 0], [10, 0], [10, 10]], "size": "100A" })).is_err(), "too short for its elbow");
    assert!(s.execute("pipe", &json!({ "points": [[0, 0], [10, 0]], "size": "13A" })).is_err());
}

#[test]
fn steel_members_from_jis_tables() {
    let mut s = Session::new();
    for (kind, size) in [("C", "150x75x6.5x10"), ("L", "50x50x6"), ("H", "200x200x8x12"), ("TUBE", "100x50x3.2"), ("FB", "50x6")] {
        let r = s.execute("steel", &json!({ "points": [[0, 0]], "kind": kind, "size": size })).unwrap();
        assert!(r["area"].as_f64().unwrap() > 0.0, "{kind}");
    }
    let r = s.execute("steel", &json!({ "points": [[0, -300], [800, -300]], "kind": "C", "size": "150x75x6.5x10", "view": "side" })).unwrap();
    let h = r["handles"][0].as_str().unwrap().to_string();
    let info = s.execute("steel.member", &json!({ "handle": h })).unwrap();
    assert!((info["length"].as_f64().unwrap() - 800.0).abs() < 1e-9);
    s.execute("steeledit", &json!({ "handle": h, "size": "200x80x7.5x11" })).unwrap();
    let info = s.execute("steel.member", &json!({ "handle": h })).unwrap();
    assert_eq!(info["size"], "200x80x7.5x11");
    assert!(s.execute("steel", &json!({ "points": [[0, 0]], "kind": "C", "size": "151x75x6.5x10" })).is_err());
    let st = s.execute("steel.settings", &json!({ "kind": "L" })).unwrap();
    assert!(st["sizes"].as_array().unwrap().len() > 30);
}

#[test]
fn pipe_sizes_in_inches() {
    use cadcraft_engine::cmd::pipe::{inch, size};
    assert_eq!(size("2\"").unwrap().name, "50A");
    assert_eq!(size("1-1/2").unwrap().name, "40A");
    assert_eq!(size("1 1/4\"").unwrap().name, "32A");
    assert_eq!(size("3B").unwrap().name, "80A");
    assert_eq!(size("50").unwrap().name, "50A");
    assert_eq!(size("15A").unwrap().name, "15A");
    assert_eq!(inch("100A"), "4");
    let mut s = Session::new();
    s.execute("pipe", &json!({ "points": [[0, 0], [500, 0]], "size": "2\"" })).unwrap();
    assert!(kinds(&s).iter().any(|k| matches!(k, EntityKind::MText(t) if t.contents == "50A (2\") SGP")));
}

#[test]
fn textedit_offers_the_in_place_editor_and_takes_stacked_text() {
    let mut s = Session::new();
    s.viewport_px = (1200.0, 800.0);
    let r = s.execute("dimlinear", &json!({ "p1": [0, 0], "p2": [25, 0], "at": [12.5, 10] })).unwrap();
    let h = cadcraft_engine::doc::Handle::parse_hex(r["handle"].as_str().unwrap()).unwrap();
    s.execute("zoom", &json!({ "mode": "extents" })).unwrap();
    let mid = match &s.doc().unwrap().entity(h).unwrap().kind {
        EntityKind::Dimension(d) => d.text_mid,
        _ => panic!("dimension"),
    };
    s.start("textedit").unwrap();
    s.input(cadcraft_engine::Input::Point(cadcraft_engine::geom::Vec2::new(mid.x, mid.y))).unwrap();
    let proxy = s.editing_mtext().expect("the editor opens");
    assert_eq!(proxy.contents, "<>");
    // What the editor hands back: the measurement, a fit and a stacked tolerance.
    let mut f = cadcraft_engine::mtext_edit::Formatted::from_contents("<>H8+0.033^0");
    assert!(f.stack(4..12));
    s.input(cadcraft_engine::Input::Text(f.to_contents())).unwrap();
    s.input(cadcraft_engine::Input::Enter).unwrap();
    assert!(s.running.is_none());
    match &s.doc().unwrap().entity(h).unwrap().kind {
        EntityKind::Dimension(d) => assert_eq!(d.text, "<>H8\\S+0.033^0;"),
        _ => panic!("dimension"),
    }
    // Single-line text keeps plain characters.
    let t = s.execute("text", &json!({ "at": [0, 50], "height": 2.5, "text": "ABC" })).unwrap();
    let th = cadcraft_engine::doc::Handle::parse_hex(t["handle"].as_str().unwrap()).unwrap();
    s.start("textedit").unwrap();
    s.input(cadcraft_engine::Input::Point(cadcraft_engine::geom::Vec2::new(1.0, 51.0))).unwrap();
    assert!(s.editing_mtext().is_some());
    s.input(cadcraft_engine::Input::Text("{\\fArial|b1;XYZ}".into())).unwrap();
    s.input(cadcraft_engine::Input::Enter).unwrap();
    match &s.doc().unwrap().entity(th).unwrap().kind {
        EntityKind::Text(t) => assert_eq!(t.value, "XYZ"),
        _ => panic!("text"),
    }
}

#[test]
fn mtext_strike_oblique_tracking_width_round_trip() {
    use cadcraft_engine::mtext_edit::{Flag, Formatted};
    let f = Formatted::from_contents("A\\K\\Q15;\\T1.5;\\W0.8;BC\\kD");
    assert!(f.fmt_at(1).strike);
    assert_eq!(f.fmt_at(1).oblique, Some(15.0));
    assert_eq!(f.fmt_at(1).track, Some(1.5));
    assert_eq!(f.fmt_at(1).width, Some(0.8));
    assert!(!f.fmt_at(3).strike);
    let back = Formatted::from_contents(&f.to_contents());
    assert_eq!(back.text(), "ABCD");
    assert_eq!(back.fmt_at(2), f.fmt_at(2));
    let mut g = Formatted::from_contents("XYZ");
    g.toggle(0..2, Flag::Strike);
    assert!(g.to_contents().contains("\\K"));
}

#[test]
fn xclip_clips_renders_and_round_trips_through_dxf() {
    let mut s = Session::new();
    // A block of two lines, 0..100 wide, inserted at the origin.
    s.execute("line", &json!({"points": [[0, 0], [100, 0]]})).unwrap();
    s.execute("line", &json!({"points": [[0, 50], [100, 50]]})).unwrap();
    let all: Vec<String> = s.doc().unwrap().model.iter().map(|e| e.handle.hex()).collect();
    s.execute("block", &json!({"name": "B", "base": [0, 0], "handles": all, "keep": "delete"})).unwrap();
    let r = s.execute("insert", &json!({"name": "B", "at": [10, 10]})).unwrap();
    let h = r["handle"].as_str().unwrap().to_string();
    s.execute("xclip", &json!({"handles": [h], "boundary": [[20, 0], [60, 100]]})).unwrap();
    let d = s.doc().unwrap();
    let EntityKind::Insert(i) = &d.model.iter().find(|e| e.handle.hex() == h).unwrap().kind else { panic!() };
    let c = i.clip.as_ref().unwrap();
    assert_eq!(c.boundary.len(), 4);
    assert!((c.boundary[0].x - 10.0).abs() < 1e-9);
    // Clipped: the drawn lines span 20..60 only.
    let list = cadcraft_render::build(d, &cadcraft_engine::doc::Space::Model, &Default::default());
    let xs: Vec<f64> = list.verts.iter().map(|v| v.x).collect();
    let (lo, hi) = (xs.iter().cloned().fold(f64::MAX, f64::min), xs.iter().cloned().fold(f64::MIN, f64::max));
    assert!((lo - 20.0).abs() < 1e-6 && (hi - 60.0).abs() < 1e-6, "{lo} {hi}");
    // DXF: CADCraft's xdata and AutoCAD's SPATIAL_FILTER.
    let text = cadcraft_io::write_dxf(d);
    assert!(text.contains("SPATIAL_FILTER") && text.contains("ACAD_FILTER"));
    let back = cadcraft_io::read_dxf(text.as_bytes()).unwrap();
    let ins = back.model.iter().find_map(|e| if let EntityKind::Insert(i) = &e.kind { Some(i.clone()) } else { None }).unwrap();
    assert_eq!(ins.clip.as_ref().unwrap().boundary.len(), 4);
    // Without CADCraft's xdata the SPATIAL_FILTER alone gives the boundary.
    let stripped: String = {
        let lines: Vec<&str> = text.lines().collect();
        let mut out = Vec::new();
        let mut k = 0;
        while k < lines.len() {
            if lines[k].trim() == "1001" && lines.get(k + 1).map(|l| l.trim()) == Some("CADCRAFT") {
                k += 2;
                while k + 1 < lines.len() && lines[k].trim().parse::<i32>().is_ok_and(|c| c >= 1000) && lines[k].trim() != "1001" {
                    k += 2;
                }
                continue;
            }
            out.push(lines[k]);
            k += 1;
        }
        out.join("\n")
    };
    let back = cadcraft_io::read_dxf(stripped.as_bytes()).unwrap();
    let ins = back.model.iter().find_map(|e| if let EntityKind::Insert(i) = &e.kind { Some(i.clone()) } else { None }).unwrap();
    let c = ins.clip.expect("clip from SPATIAL_FILTER");
    assert!(c.boundary.iter().any(|p| (p.x - 10.0).abs() < 1e-6 && (p.y + 10.0).abs() < 1e-6), "{:?}", c.boundary);
    // Off, then delete.
    s.execute("xclip", &json!({"handles": [h], "op": "off"})).unwrap();
    s.execute("xclip", &json!({"handles": [h], "op": "delete"})).unwrap();
    let d = s.doc().unwrap();
    let EntityKind::Insert(i) = &d.model.iter().find(|e| e.handle.hex() == h).unwrap().kind else { panic!() };
    assert!(i.clip.is_none());
}

#[test]
fn xclip_typed_rectangle_and_invert() {
    let mut s = Session::new();
    s.execute("circle", &json!({"center": [0, 0], "radius": 10})).unwrap();
    let all: Vec<String> = s.doc().unwrap().model.iter().map(|e| e.handle.hex()).collect();
    s.execute("block", &json!({"name": "C", "base": [0, 0], "handles": all, "keep": "delete"})).unwrap();
    let h = s.execute("insert", &json!({"name": "C", "at": [0, 0]})).unwrap()["handle"].as_str().unwrap().to_string();
    s.set_selection(vec![cadcraft_engine::doc::Handle::parse_hex(&h).unwrap()]);
    typed(&mut s, &["xclip", "", "i", "-5,-5", "5,5"]);
    assert!(s.running.is_none());
    let d = s.doc().unwrap();
    let EntityKind::Insert(i) = &d.model.iter().find(|e| e.handle.hex() == h).unwrap().kind else { panic!() };
    let c = i.clip.as_ref().unwrap();
    assert!(c.inverted && c.enabled);
    // Inverted: the circle shows (it lies outside the hole).
    let list = cadcraft_render::build(d, &cadcraft_engine::doc::Space::Model, &Default::default());
    assert!(list.verts.iter().any(|v| (v.len() - 10.0).abs() < 0.1));
    typed(&mut s, &["xclip", "p"]);
}
