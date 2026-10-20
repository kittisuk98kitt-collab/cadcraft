//! Middle-centred attributes (balloons) keep their vertical alignment through DXF and DWG:
//! in ATTRIB/ATTDEF it is group 74, group 73 being the field length.

use cadcraft_doc::*;
use cadcraft_geom::Vec3;
use std::sync::Arc;

fn attrib(tag: &str, y: f64) -> Attrib {
    let text = Text {
        insert: Vec3::new(-1.0, y - 0.5, 0.0),
        align_pt: Some(Vec3::new(0.0, y, 0.0)),
        height: 1.0,
        value: tag.into(),
        rotation: 0.0,
        width_factor: 1.0,
        oblique: 0.0,
        style: "Standard".into(),
        halign: HAlign::Center,
        valign: VAlign::Middle,
    };
    Attrib {
        tag: tag.into(),
        text,
        invisible: false,
        constant: false,
        prompt: String::new(),
        props: Default::default(),
        multiline: None,
        modes: Default::default(),
    }
}

#[test]
fn centred_attribs_through_dwg() {
    let mut d = Drawing::new_metric();
    let mut b = Block::new("BAL");
    for a in [attrib("A", 1.0), attrib("B", -1.0)] {
        let h = d.new_handle();
        b.entities.push(Entity::new(h, EntityKind::AttDef(a)));
    }
    d.blocks.insert("BAL".into(), Arc::new(b));
    let ins = Insert {
        block: "BAL".into(),
        insert: Vec3::new(10.0, 10.0, 0.0),
        scale: Vec3::new(6.0, 6.0, 6.0),
        rotation: 0.0,
        attribs: vec![attrib("A", 1.0), attrib("B", -1.0)],
        cols: 1,
        rows: 1,
        col_spacing: 0.0,
        row_spacing: 0.0,
        clip: None,
    };
    d.add(&Space::Model, Common::default(), EntityKind::Insert(ins)).unwrap();
    for name in ["x.dxf", "x.dwg"] {
        let bytes = cadcraft_io::write(&d, name).unwrap();
        let back = cadcraft_io::read(&bytes, name).unwrap();
        let i = back.model.iter().find_map(|e| if let EntityKind::Insert(i) = &e.kind { Some(i.clone()) } else { None }).unwrap();
        for a in &i.attribs {
            assert_eq!((a.text.halign, a.text.valign), (HAlign::Center, VAlign::Middle), "{name} attrib {}", a.tag);
        }
        let blk = back.block("BAL").unwrap();
        for e in blk.entities.iter() {
            if let EntityKind::AttDef(a) = &e.kind {
                assert_eq!((a.text.halign, a.text.valign), (HAlign::Center, VAlign::Middle), "{name} definition {}", a.tag);
            }
        }
    }
}
