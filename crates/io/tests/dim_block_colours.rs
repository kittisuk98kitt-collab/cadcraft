//! The block a dimension is written with (what AutoCAD and other readers draw) carries the
//! style's colours: DIMCLRD for the dimension line and arrows, DIMCLRE for the extension lines,
//! DIMCLRT for the text.

use cadcraft_color::Color;
use cadcraft_doc::{DimKind, DimMarks, DimStyle, Dimension, Drawing, Entity, EntityKind};
use cadcraft_geom::Vec3;
use cadcraft_io::{read_dxf, write_dxf};

#[test]
fn dimension_block_takes_the_style_colours() {
    let mut d = Drawing::new_metric();
    d.dim_styles.push(DimStyle {
        name: "C".into(),
        dim_line_color: Color::Index(2),
        ext_line_color: Color::Index(3),
        text_color: Color::Index(7),
        ..DimStyle::default()
    });
    let k = EntityKind::Dimension(Dimension {
        kind: DimKind::Linear { rotation: 0.0 },
        defpt: Vec3::new(0.0, 5.0, 0.0),
        text_mid: Vec3::ZERO,
        p13: Vec3::ZERO,
        p14: Vec3::new(40.0, 0.0, 0.0),
        p15: Vec3::ZERO,
        p16: Vec3::ZERO,
        text: String::new(),
        style: "C".into(),
        measurement: 0.0,
        text_rotation: 0.0,
        user_text_pos: false,
        block: None,
        overrides: Default::default(),
        assoc: Vec::new(),
        marks: DimMarks::default(),
    });
    let h = d.new_handle();
    d.model.push(Entity { handle: h, common: Default::default(), kind: k });
    let back = read_dxf(write_dxf(&d).as_bytes()).expect("reads");
    let blk = back
        .model
        .iter()
        .find_map(|e| match &e.kind {
            EntityKind::Dimension(dm) => dm.block.clone(),
            _ => None,
        })
        .and_then(|n| back.block(&n).cloned())
        .expect("dimension block");
    let colours = |want: fn(&EntityKind) -> bool| blk.entities.iter().filter(|e| want(&e.kind)).map(|e| e.common.color).collect::<Vec<_>>();
    assert!(colours(|k| matches!(k, EntityKind::MText(_))).iter().all(|c| *c == Color::Index(7)));
    let lines = colours(|k| matches!(k, EntityKind::Line(_)));
    assert!(lines.contains(&Color::Index(2)) && lines.contains(&Color::Index(3)), "{lines:?}");
}
