use easyacft_lang::expr::compile;
use std::path::Path;

#[test]
fn the_twin_jet_exports_as_a_standalone_crate() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/twin-jet");
    let out = std::env::temp_dir().join(format!("easyacft-export-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&out);
    let s = easyacft_export::export(&dir, &out, Some("Example Twin")).unwrap();
    assert_eq!(s.crate_name, "example-twin");
    assert_eq!((s.parts, s.tests), (184, 2));
    let lib = std::fs::read_to_string(out.join("src/lib.rs")).unwrap();
    assert!(lib.contains("pub fn build() -> Aircraft") && lib.contains("pub const DC1_VOLTAGE: usize"));
    assert!(lib.contains("delay(&mut st[0], b(s[v::GEN1_READY]), 1.0, dt)"), "expressions become Rust");
    for f in ["runtime/core/src/network.rs", "runtime/parts/src/electrical/machine.rs", "runtime/parts/src/actuation/ode.rs", "runtime/parts/src/aircon/mod.rs", "runtime/lang/src/expr.rs", "LICENSE-EXPORT.md", "tests/cold_start.rs"] {
        assert!(out.join(f).exists(), "{f}");
    }
    assert!(!out.join("runtime/lang/src/loader.rs").exists() && !out.join("runtime/parts/src/scenario.rs").exists());
    let manifest = std::fs::read_to_string(out.join("Cargo.toml")).unwrap() + &std::fs::read_to_string(out.join("runtime/lang/Cargo.toml")).unwrap();
    assert!(!manifest.contains("toml_edit"), "no outside dependencies");
    assert!(!std::fs::read_to_string(out.join("runtime/core/src/network.rs")).unwrap().contains("#[cfg(test)]"));
    assert!(easyacft_export::export(&dir, &out, None).is_err(), "never writes over a crate");
    std::fs::remove_dir_all(&out).unwrap();
}

#[test]
fn bytecode_and_generated_rust_agree() {
    struct N;
    impl easyacft_lang::expr::Names for N {
        fn lookup(&self, n: &str) -> Option<(usize, easyacft_lang::Type)> {
            ["a", "b"].iter().position(|x| *x == n).map(|i| (i, easyacft_lang::Type::Num(easyacft_core::Dim::NONE)))
        }
    }
    let p = compile("if(a > 2 && !(b < 1), min(a, b) * -3, abs(a - b) / 0)", &N).unwrap();
    let body = easyacft_export::rust_body(&p, &["A".into(), "B".into()]);
    assert_eq!(body.trim(), "if b(f(b(f(b(f(s[v::A] > 2.0)))) && b(f(b(f(!b(f(s[v::B] < 1.0)))))))) { (f64::min(s[v::A], s[v::B]) * -(3.0)) } else { div(f64::abs((s[v::A] - s[v::B])), 0.0) }");
    let guarded = compile("if(a > 2, latch(b > 1, false), false)", &N).unwrap();
    let body = easyacft_export::rust_body(&guarded, &["A".into(), "B".into()]);
    assert!(body.contains("latch(&mut st[0]") && body.contains("st[0..1].fill(0.0)"), "a skipped arm resets its latch:
{body}");
}
