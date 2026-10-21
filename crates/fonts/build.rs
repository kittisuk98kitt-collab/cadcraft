//! Embeds the fonts of the optional craft-fonts build input (`CRAFT_FONTS_DIR`) as `CRAFT_FONTS`.
//! The recipe is craft-fonts' `docs/integration.md` (as in the other Crafting Apps); unset or
//! empty, `CRAFT_FONTS` is empty and the build is unchanged. It only reads the local checkout
//! (no network). A bad checkout is a warning, or an error with `CRAFT_FONTS_REQUIRED=1`.
//!
//! On wasm32 it embeds only BIZ UDPGothic Regular and the `Arab` faces (about 5.5 MB), to keep the
//! web build within hosting limits (Cloudflare Pages: 25 MiB per file). The web build has no
//! system fonts, so these are its only fallback for Japanese and Arabic-script text.

use std::fmt::Write as _;
use std::path::PathBuf;

fn main() {
    println!("cargo::rerun-if-env-changed=CRAFT_FONTS_DIR");
    println!("cargo::rerun-if-env-changed=CRAFT_FONTS_REQUIRED");
    let mut src = String::from("pub static CRAFT_FONTS: &[CraftFont] = &[\n");
    if let Some(dir) = std::env::var_os("CRAFT_FONTS_DIR").filter(|d| !d.is_empty()).map(PathBuf::from) {
        match craft_fonts(&dir) {
            Ok(entries) => src.push_str(&entries),
            Err(e) if std::env::var_os("CRAFT_FONTS_REQUIRED").is_some() => {
                println!("cargo::error=CRAFT_FONTS_DIR={}: {e}", dir.display());
            }
            Err(e) => println!("cargo::warning=building without craft-fonts: CRAFT_FONTS_DIR={}: {e}", dir.display()),
        }
    }
    src.push_str("];\n");
    let out = PathBuf::from(std::env::var_os("OUT_DIR").unwrap_or_default()).join("craft_fonts.rs");
    if let Err(e) = std::fs::write(&out, src) {
        println!("cargo::error=writing {}: {e}", out.display());
    }
    shx_fonts();
}

/// Embeds the `.shx` files of `CRAFT_SHX_DIR` (the user's own shape fonts, for a personal
/// build; never part of the source) as `SHX_FONTS`. Unset, it is empty.
fn shx_fonts() {
    println!("cargo::rerun-if-env-changed=CRAFT_SHX_DIR");
    let mut src = String::from("/// Shape fonts built in (`CRAFT_SHX_DIR`): file name and contents.\npub static SHX_FONTS: &[(&str, &[u8])] = &[\n");
    if let Some(dir) = std::env::var_os("CRAFT_SHX_DIR").filter(|d| !d.is_empty()).map(PathBuf::from) {
        println!("cargo::rerun-if-changed={}", dir.display());
        match std::fs::read_dir(&dir) {
            Ok(rd) => {
                let mut files: Vec<PathBuf> =
                    rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("shx"))).collect();
                files.sort();
                for p in files {
                    let Ok(abs) = p.canonicalize() else { continue };
                    let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                    let _ = writeln!(src, "    ({name:?}, include_bytes!({:?})),", abs.display().to_string());
                }
            }
            Err(e) => println!("cargo::warning=CRAFT_SHX_DIR={}: {e}", dir.display()),
        }
    }
    src.push_str("];\n");
    let out = PathBuf::from(std::env::var_os("OUT_DIR").unwrap_or_default()).join("shx_fonts.rs");
    if let Err(e) = std::fs::write(&out, src) {
        println!("cargo::error=writing {}: {e}", out.display());
    }
}

/// Family-name prefixes AGENTS.md bars (no Adobe fonts, ever): Adobe's Source families and Noto
/// CJK, which is Source Han under another name. `src/craft_fonts.rs` tests the same list.
const BARRED_FAMILIES: [&str; 5] = ["Source Han", "Source Serif", "Source Sans", "Noto Sans CJK", "Noto Serif CJK"];

/// One `CraftFont { .. }` initialiser per manifest line, in manifest order.
fn craft_fonts(dir: &std::path::Path) -> Result<String, String> {
    let manifest = dir.join("fonts/manifest.txt");
    println!("cargo::rerun-if-changed={}", manifest.display());
    let text = std::fs::read_to_string(&manifest).map_err(|e| format!("{}: {e}", manifest.display()))?;
    let wasm = std::env::var("CARGO_CFG_TARGET_ARCH").is_ok_and(|a| a == "wasm32");
    let mut out = String::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
        let f: Vec<&str> = line.split(" | ").map(str::trim).collect();
        let [family, style, file, scripts, ..] = f.as_slice() else {
            return Err(format!("malformed manifest line: {line}"));
        };
        if BARRED_FAMILIES.iter().any(|barred| family.starts_with(barred)) {
            println!("cargo::warning=craft-fonts: not embedding {family} {style} (AGENTS.md: no Adobe fonts)");
            continue;
        }
        let arabic = scripts.split(',').any(|s| s.trim() == "Arab");
        // Thai too (Kitt Edition): drawings and the interface in Thai on the web.
        let thai = scripts.split(',').any(|s| s.trim() == "Thai");
        let web_face = (*family == "BIZ UDPGothic" && *style == "Regular") || arabic || thai;
        if wasm && !web_face {
            continue;
        }
        let path = dir.join(file).canonicalize().map_err(|e| format!("{file}: {e}"))?;
        println!("cargo::rerun-if-changed={}", path.display());
        let scripts: Vec<String> = scripts.split(',').map(|s| format!("{:?}", s.trim())).collect();
        let _ = writeln!(
            out,
            "    CraftFont {{ family: {family:?}, style: {style:?}, scripts: &[{}], bytes: include_bytes!({:?}) }},",
            scripts.join(", "),
            path.display().to_string(),
        );
    }
    Ok(out)
}
