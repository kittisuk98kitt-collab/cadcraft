//! The built-in Kitt default template: the company's layers and dimension and text styles.

use cadcraft_engine::cmd::file::{IoHooks, set_io, template};

#[test]
fn kitt_default_template_has_the_company_standard() {
    set_io(IoHooks {
        read: |b, name| cadcraft_io::read(b, name).map_err(|e| e.to_string()),
        write: |d, name| cadcraft_io::write(d, name).map_err(|e| e.to_string()),
        plot: None,
    });
    let d = template::builtin_drawing().expect("template reads");
    let mut layers: Vec<String> = d.layers.iter().map(|l| l.name.clone()).collect();
    layers.sort_by_key(|n| n.to_ascii_lowercase());
    assert_eq!(layers, ["0", "2-DIM", "CENTER", "Cutline", "Defpoints", "HIDDEN", "LEVEL001", "LEVEL002", "PHANTOM"]);
    // Style 5 is current at DIMSCALE 5, with no header overrides on top of it.
    assert_eq!(d.header.str("DIMSTYLE", ""), "5");
    let st = d.dim_style("5").expect("style 5");
    assert_eq!(st.scale, 5.0);
    assert_eq!(st.arrow_block, "_OPEN30");
    assert!(d.dim_overrides().is_empty(), "{:?}", d.dim_overrides());
    for n in ["5 base", "4", "3", "2", "0.5", "ALSYS", "AM_JIS"] {
        assert!(d.dim_style(n).is_some(), "{n}");
    }
    // Text in txt.shx with the Japanese big font; TXT.shx at width 0.8.
    let ts = d.text_style("STANDARD").expect("STANDARD");
    assert!(ts.font.eq_ignore_ascii_case("txt.shx") && ts.big_font.eq_ignore_ascii_case("bigfont.shx"));
    assert!((d.text_style("TXT.shx").expect("TXT.shx").width_factor - 0.8).abs() < 1e-9);
    // The earlier name of the template still means it.
    template::set_qnew_template("<ALSYS default>");
    assert_eq!(template::qnew_template(), template::BUILTIN);
    template::set_qnew_template("");
}
