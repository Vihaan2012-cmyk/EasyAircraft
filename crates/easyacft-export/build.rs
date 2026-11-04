//! Embeds the runtime sources the export copies.

use std::fmt::Write;
use std::path::Path;

const CRATES: [(&str, &str, &[&str]); 3] = [("core", "easyacft-core", &[]), ("lang", "easyacft-lang", &["loader.rs"]), ("parts", "easyacft-parts", &["files.rs", "scenario.rs"])];

fn main() {
    let root = Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("..");
    let mut out = String::from("pub static FILES: &[(&str, &str)] = &[\n");
    for (short, dir, skip) in CRATES {
        let src = root.join(dir).join("src");
        println!("cargo:rerun-if-changed={}", src.display());
        let mut names: Vec<_> = std::fs::read_dir(&src).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.ends_with(".rs") && !skip.contains(&n.as_str())).collect();
        names.sort();
        for n in names {
            let p = src.join(&n);
            writeln!(out, "    (\"{short}/src/{n}\", include_str!(r\"{}\")),", p.display()).unwrap();
        }
    }
    out.push_str("];\n");
    let dest = Path::new(&std::env::var("OUT_DIR").unwrap()).join("runtime_files.rs");
    std::fs::write(dest, out).unwrap();
}
