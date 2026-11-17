//! Generates `brand.rs` from the workspace's `brand.toml`: one constant for each name, as
//! `SECTION_KEY`, so code takes the product's names from one file.

use std::fmt::Write as _;
use std::path::Path;

fn main() {
    let file = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../brand.toml");
    println!("cargo:rerun-if-changed={}", file.display());
    let text = std::fs::read_to_string(&file).expect("brand.toml at the workspace's root");
    let table: toml::Table = toml::from_str(&text).expect("brand.toml is TOML");
    let mut out = String::from("// Generated from brand.toml by build.rs. Edit brand.toml, not this file.\n");
    for (section, keys) in &table {
        let keys = keys.as_table().unwrap_or_else(|| panic!("brand.toml: [{section}] is a table"));
        for (key, value) in keys {
            let value = value.as_str().unwrap_or_else(|| panic!("brand.toml: {section}.{key} is text"));
            let name = format!("{section}_{key}").to_ascii_uppercase();
            writeln!(out, "pub const {name}: &str = {value:?};").unwrap();
        }
    }
    let to = Path::new(&std::env::var("OUT_DIR").expect("cargo sets OUT_DIR")).join("brand.rs");
    std::fs::write(to, out).expect("OUT_DIR is writable");
}
