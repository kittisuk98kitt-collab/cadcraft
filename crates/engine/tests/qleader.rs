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
