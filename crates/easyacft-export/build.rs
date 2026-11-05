//! Embeds the runtime sources the export copies, module folders included.

use std::fmt::Write;
use std::path::Path;

const CRATES: [(&str, &str, &[&str]); 3] = [("core", "easyacft-core", &[]), ("lang", "easyacft-lang", &["loader.rs"]), ("parts", "easyacft-parts", &["files.rs", "scenario.rs"])];

/// Every `.rs` file under `dir`, as paths relative to `base`, sorted so the output is stable.
fn sources(base: &Path, dir: &Path, skip: &[&str], out: &mut Vec<String>) {
    println!("cargo:rerun-if-changed={}", dir.display());
    let mut entries: Vec<_> = std::fs::read_dir(dir).unwrap().flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let p = e.path();
        if p.is_dir() {
            sources(base, &p, skip, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            let rel = p.strip_prefix(base).unwrap().to_string_lossy().replace('\\', "/");
            if !skip.contains(&rel.as_str()) {
                out.push(rel);
            }
        }
    }
}

fn main() {
    let root = Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("..");
    let mut out = String::from("pub static FILES: &[(&str, &str)] = &[\n");
    for (short, dir, skip) in CRATES {
        let src = root.join(dir).join("src");
        let mut names = Vec::new();
        sources(&src, &src, skip, &mut names);
        for n in names {
            let p = src.join(&n);
            writeln!(out, "    (\"{short}/src/{n}\", include_str!(r\"{}\")),", p.display()).unwrap();
        }
    }
    out.push_str("];\n");
    let dest = Path::new(&std::env::var("OUT_DIR").unwrap()).join("runtime_files.rs");
    std::fs::write(dest, out).unwrap();
}
