//! A DWG keeps the drawing's current layer, text style and dimension style (the DWG header holds
//! them as handles; the bridge maps them to and from the names).

use cadcraft_doc::{DimStyle, Drawing, Layer, TextStyle};

#[test]
fn current_styles_survive_a_dwg_round_trip() {
    let mut d = Drawing::new_metric();
    d.layers.push(Layer { name: "2-DIM".into(), ..Layer::default() });
    d.text_styles.push(TextStyle { name: "TXT08".into(), width_factor: 0.8, ..TextStyle::default() });
    d.dim_styles.push(DimStyle { name: "5".into(), scale: 5.0, ..DimStyle::default() });
    d.header.set_str("CLAYER", "2-DIM");
    d.header.set_str("TEXTSTYLE", "TXT08");
    d.header.set_str("DIMSTYLE", "5");
    let bytes = cadcraft_io::write(&d, "t.dwg").expect("dwg written");
    let back = cadcraft_io::read(&bytes, "t.dwg").expect("dwg read");
    assert_eq!(back.header.str("DIMSTYLE", ""), "5");
    assert_eq!(back.header.str("CLAYER", ""), "2-DIM");
    assert_eq!(back.header.str("TEXTSTYLE", ""), "TXT08");
}
