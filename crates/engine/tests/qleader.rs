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
