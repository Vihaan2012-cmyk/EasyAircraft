//! `easyacft export`: an aircraft written out as a standalone Rust crate with no dependency on EasyAcft.

use easyacft_lang::expr::{Op, Program, Table, Type};
use easyacft_lang::schema::Value;
use easyacft_lang::{check, Checked, Model};
use std::collections::HashMap;
use std::fmt::Write;
use std::path::{Path, PathBuf};

include!(concat!(env!("OUT_DIR"), "/runtime_files.rs"));

const EXCEPTION: &str = include_str!("../../../LICENSE-EXPORT.md");

pub struct Summary {
    pub out: PathBuf,
    pub crate_name: String,
    pub parts: usize,
    pub values: usize,
    pub expressions: usize,
    pub tests: usize,
}

fn lit(x: f64) -> String {
    if x.is_nan() {
        "f64::NAN".into()
    } else if x.is_infinite() {
        if x > 0.0 { "f64::INFINITY".into() } else { "f64::NEG_INFINITY".into() }
    } else {
        let s = format!("{x:?}");
        s
    }
}

fn ty_lit(t: Type) -> String {
    match t {
        Type::Bool => "Type::Bool".into(),
        Type::Num(d) => format!("Type::Num(Dim({:?}))", d.0),
    }
}

fn table_lit(t: &Table) -> String {
    let v = |xs: &[f64]| format!("vec![{}]", xs.iter().map(|x| lit(*x)).collect::<Vec<_>>().join(", "));
    format!("Table {{ xs: {}, cols: {}, values: vec![{}] }}", v(&t.xs), v(&t.cols), t.values.iter().map(|r| v(r)).collect::<Vec<_>>().join(", "))
}

/// `dc1.voltage` to `DC1_VOLTAGE`, unique.
fn const_names(names: &[(String, Type)]) -> Vec<String> {
    let mut seen: HashMap<String, usize> = HashMap::new();
    names
        .iter()
        .map(|(n, _)| {
            let mut c: String = n.chars().map(|ch| if ch.is_ascii_alphanumeric() { ch.to_ascii_uppercase() } else { '_' }).collect();
            if c.starts_with(|ch: char| ch.is_ascii_digit()) {
                c.insert(0, '_');
            }
            let k = seen.entry(c.clone()).or_default();
            *k += 1;
            if *k > 1 {
                c = format!("{c}_{k}");
            }
            c
        })
        .collect()
}

/// The bytecode as a Rust function body; side effects keep their order, and `&&`, `||` and `if` arms become
/// Rust blocks that run only when taken, resetting their timers when skipped, as in the interpreter.
pub fn rust_body(p: &Program, consts: &[String]) -> String {
    let mut tmp = 0;
    let (body, mut st) = region(&p.ops, 0, p.ops.len(), consts, &mut tmp, "    ");
    let ret = st.pop().unwrap_or_else(|| "0.0".into());
    format!("{body}    {ret}\n")
}

fn reset(k0: usize, k1: usize) -> String {
    if k0 < k1 { format!("st[{k0}..{k1}].fill(0.0); ") } else { String::new() }
}

fn region(ops: &[Op], lo: usize, hi: usize, consts: &[String], tmp: &mut usize, ind: &str) -> (String, Vec<String>) {
    let mut body = String::new();
    let mut st: Vec<String> = Vec::new();
    fn pop(st: &mut Vec<String>) -> String {
        st.pop().unwrap_or_else(|| "0.0".into())
    }
    let name = |tmp: &mut usize| {
        let n = format!("t{tmp}");
        *tmp += 1;
        n
    };
    let inner = format!("{ind}    ");
    let mut pc = lo;
    while pc < hi {
        let op = ops[pc];
        pc += 1;
        let e = match op {
            Op::Push(v) => lit(v),
            Op::Load(i) => format!("s[{}]", consts.get(i).map_or(i.to_string(), |c| format!("v::{c}"))),
            Op::Not => format!("f(!b({}))", pop(&mut st)),
            Op::Truth => format!("f(b({}))", pop(&mut st)),
            Op::Neg => format!("-({})", pop(&mut st)),
            Op::Abs => format!("f64::abs({})", pop(&mut st)),
            Op::AndThen(to, k0, k1) | Op::OrElse(to, k0, k1) => {
                let l = pop(&mut st);
                let (rb, mut rs) = region(ops, pc, to, consts, tmp, &inner);
                let r = pop(&mut rs);
                let and = matches!(op, Op::AndThen(..));
                pc = to;
                if rb.is_empty() && k0 == k1 {
                    format!("f(b({l}) {} b({r}))", if and { "&&" } else { "||" })
                } else {
                    let n = name(tmp);
                    let (taken, skipped) = if and { ("b", "0.0") } else { ("!b", "1.0") };
                    writeln!(body, "{ind}let {n}: f64 = if {taken}({l}) {{\n{rb}{inner}{r}\n{ind}}} else {{ {}{skipped} }};", reset(k0, k1)).unwrap();
                    n
                }
            }
            Op::IfNot(to, k0, k1) => {
                let c = pop(&mut st);
                let Op::Jump(end, e0, e1) = ops[to - 1] else { unreachable!("an if's then arm ends in a jump") };
                let (tb, mut ts) = region(ops, pc, to - 1, consts, tmp, &inner);
                let (eb, mut es) = region(ops, to, end, consts, tmp, &inner);
                let (t, e) = (pop(&mut ts), pop(&mut es));
                pc = end;
                if tb.is_empty() && eb.is_empty() && k0 == k1 && e0 == e1 {
                    format!("if b({c}) {{ {t} }} else {{ {e} }}")
                } else {
                    let n = name(tmp);
                    let line = |r: String| if r.is_empty() { r } else { format!("{inner}{r}\n") };
                    writeln!(body, "{ind}let {n}: f64 = if b({c}) {{\n{}{tb}{inner}{t}\n{ind}}} else {{\n{}{eb}{inner}{e}\n{ind}}};", line(reset(e0, e1)), line(reset(k0, k1))).unwrap();
                    n
                }
            }
            Op::Jump(..) => unreachable!("jumps are taken with their if"),
            Op::Delay(k) | Op::OffDelay(k) => {
                let t = pop(&mut st);
                let c = pop(&mut st);
                let n = name(tmp);
                let f = if matches!(op, Op::Delay(_)) { "delay" } else { "off_delay" };
                writeln!(body, "{ind}let {n} = {f}(&mut st[{k}], b({c}), {t}, dt);").unwrap();
                n
            }
            Op::Latch(k) => {
                let reset = pop(&mut st);
                let set = pop(&mut st);
                let n = name(tmp);
                writeln!(body, "{ind}let {n} = latch(&mut st[{k}], b({set}), b({reset}));").unwrap();
                n
            }
            Op::Rising(k) => {
                let c = pop(&mut st);
                let n = name(tmp);
                writeln!(body, "{ind}let {n} = rising(&mut st[{k}], b({c}));").unwrap();
                n
            }
            Op::Table(i, _) => {
                let y = pop(&mut st);
                let x = pop(&mut st);
                format!("tables[{i}].lookup({x}, {y})")
            }
            _ => {
                let r = pop(&mut st);
                let l = pop(&mut st);
                match op {
                    Op::Add => format!("({l} + {r})"),
                    Op::Sub => format!("({l} - {r})"),
                    Op::Mul => format!("({l} * {r})"),
                    Op::Div => format!("div({l}, {r})"),
                    Op::Lt => format!("f({l} < {r})"),
                    Op::Le => format!("f({l} <= {r})"),
                    Op::Gt => format!("f({l} > {r})"),
                    Op::Ge => format!("f({l} >= {r})"),
                    Op::Eq => format!("f({l} == {r})"),
                    Op::Ne => format!("f({l} != {r})"),
                    Op::Min => format!("f64::min({l}, {r})"),
                    Op::Max => format!("f64::max({l}, {r})"),
                    _ => "0.0".into(),
                }
            }
        };
        st.push(e);
    }
    (body, st)
}

struct Exprs {
    code: String,
    n: usize,
    consts: Vec<String>,
}

impl Exprs {
    /// Writes `p` as a native function and returns the `Program` literal that uses it.
    fn add(&mut self, p: &Program, doc: &str) -> String {
        let name = format!("e{}", self.n);
        self.n += 1;
        let uses = |s: &str| p.ops.iter().any(|o| match (o, s) {
            (Op::Delay(_) | Op::OffDelay(_) | Op::Latch(_) | Op::Rising(_), "st") => true,
            (Op::Table(..), "tables") => true,
            _ => false,
        });
        let dt_used = p.ops.iter().any(|o| matches!(o, Op::Delay(_) | Op::OffDelay(_)));
        let s_used = p.ops.iter().any(|o| matches!(o, Op::Load(_)));
        let args = format!("{}s: &[f64], {}st: &mut [f64], {}dt: f64, {}tables: &[Table]", if s_used { "" } else { "_" }, if uses("st") { "" } else { "_" }, if dt_used { "" } else { "_" }, if uses("tables") { "" } else { "_" });
        writeln!(self.code, "/// {doc}\npub fn {name}({args}) -> f64 {{\n{}}}\n", rust_body(p, &self.consts)).unwrap();
        let tables = p.tables.iter().map(table_lit).collect::<Vec<_>>().join(", ");
        format!("Program {{ ops: vec![], states: {}, ty: {}, tables: vec![{tables}], native: Some({name}) }}", p.states, ty_lit(p.ty))
    }
}

fn crate_name(out: &Path, name: Option<&str>) -> String {
    let raw = name.map(str::to_owned).unwrap_or_else(|| out.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "aircraft".into()));
    let mut s: String = raw.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect();
    while s.contains("--") {
        s = s.replace("--", "-");
    }
    let s = s.trim_matches('-').to_owned();
    if s.is_empty() || s.starts_with(|c: char| c.is_ascii_digit()) { format!("aircraft-{s}").trim_end_matches('-').to_owned() } else { s }
}

fn runtime_manifest(name: &str, deps: &[&str]) -> String {
    let d: String = deps.iter().map(|x| format!("easyacft-{x} = {{ path = \"../{x}\" }}\n")).collect();
    format!("[package]\nname = \"easyacft-{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\nrust-version = \"1.82\"\nlicense-file = \"../../LICENSE-EXPORT.md\"\npublish = false\n\n[dependencies]\n{d}")
}

/// Runtime sources without their tests or file loading.
fn runtime_source(path: &str, text: &str) -> String {
    let mut s = match text.find("\n#[cfg(test)]\nmod tests") {
        Some(i) => text[..i + 1].to_owned(),
        None => text.to_owned(),
    };
    if path == "lang/src/lib.rs" {
        s = s.replace("pub mod loader;\n", "").replace("pub use loader::{load, load_str};\n", "");
    }
    if path == "parts/src/lib.rs" {
        s = s.replace("mod files;\n", "").replace("pub mod scenario;\n", "");
    }
    s
}

fn checked_of(dir: &Path) -> Result<(Model, Checked), String> {
    let (model, mut diags) = easyacft_lang::load(dir);
    let own = easyacft_parts::custom::specs(&model.kinds, &mut diags);
    easyacft_parts::catalog::register(&own);
    let (checked, more) = check(&model, &easyacft_parts::catalog::With::new(&own));
    diags.extend(more);
    if diags.iter().any(|d| d.severity == easyacft_lang::Severity::Error) {
        return Err(diags.iter().map(|d| model.sources.render(d)).collect::<Vec<_>>().join("\n"));
    }
    Ok((model, checked))
}

fn field_lit(f: &easyacft_lang::model::KindField) -> String {
    format!("KindField {{ key: {:?}.into(), ty: {:?}.into(), default: {}, help: {:?}.into(), span: 0..0 }}", f.key, f.ty, f.default.as_ref().map_or("None".into(), |d| format!("Some({d:?}.into())")), f.help)
}

fn aircraft_rs(model: &Model, c: &Checked, ex: &mut Exprs) -> String {
    let mut o = String::new();
    let consts = ex.consts.clone();
    writeln!(o, "//! {}, written by `easyacft export`. Plain Rust you own: see LICENSE-EXPORT.md.\n", model.name).unwrap();
    o += "#![allow(clippy::all, unused_imports, unused_parens)]\n\nuse easyacft_core::Dim;\nuse easyacft_lang::expr::{Program, Table, Type};\nuse easyacft_lang::model::{Binding, KindDecl, KindField};\nuse easyacft_lang::schema::{Checked, CheckedPart, Slots, Value};\nuse easyacft_parts::Aircraft;\nuse std::path::Path;\npub use ops::*;\n\n";
    writeln!(o, "pub const NAME: &str = {:?};\n", model.name).unwrap();
    o += "/// Every value's index in `Aircraft::slots`.\npub mod v {\n";
    for (i, (n, _)) in c.slots.names.iter().enumerate() {
        writeln!(o, "    /// `{n}`\n    pub const {}: usize = {i};", consts[i]).unwrap();
    }
    o += "}\n\n/// Helpers the expression functions use.\npub mod ops {\n    pub fn b(x: f64) -> bool {\n        x != 0.0\n    }\n    pub fn f(x: bool) -> f64 {\n        x as u8 as f64\n    }\n    pub fn div(l: f64, r: f64) -> f64 {\n        if r == 0.0 { 0.0 } else { l / r }\n    }\n    pub fn delay(timer: &mut f64, c: bool, t: f64, dt: f64) -> f64 {\n        *timer = if c { *timer + dt } else { 0.0 };\n        f(c && *timer >= t - 1e-9)\n    }\n    pub fn off_delay(left: &mut f64, c: bool, t: f64, dt: f64) -> f64 {\n        *left = if c { t } else { (*left - dt).max(0.0) };\n        f(c || *left > 1e-9)\n    }\n    pub fn latch(state: &mut f64, set: bool, reset: bool) -> f64 {\n        *state = f(!reset && (set || b(*state)));\n        *state\n    }\n    pub fn rising(state: &mut f64, c: bool) -> f64 {\n        let out = c && !b(*state);\n        *state = f(c);\n        f(out)\n    }\n}\n\n";
    o += "/// Builds the aircraft, reading engine map files from this crate's `data` folder.\npub fn build() -> Aircraft {\n    build_with_data(Path::new(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/data\")))\n}\n\n";
    o += "/// Builds the aircraft with its data files in `data`; use this in a shipped plugin.\npub fn build_with_data(data: &Path) -> Aircraft {\n    easyacft_parts::catalog::register(&easyacft_parts::custom::specs(&kinds(), &mut Vec::new()));\n";
    let rates: Vec<String> = model.rates.iter().map(|(n, p)| format!("({n:?}.into(), {})", lit(*p))).collect();
    writeln!(o, "    let rates: Vec<(String, f64)> = vec![{}];", rates.join(", ")).unwrap();
    o += "    Aircraft::from_checked(checked(), NAME, &rates, bindings(), Vec::new(), data)\n}\n\n";
    o += "/// The X-Plane datarefs the files bound: dataref, value, X-Plane writes it, SI = raw * factor + offset.\npub fn bindings() -> Vec<Binding> {\n    vec![\n";
    for b in &model.bindings {
        writeln!(o, "        Binding {{ dataref: {:?}.into(), path: {:?}.into(), write: {}, factor: {}, offset: {}, file: 0, span: 0..0 }},", b.dataref, b.path, b.write, lit(b.factor), lit(b.offset)).unwrap();
    }
    o += "    ]\n}\n\n/// This aircraft's own part kinds.\npub fn kinds() -> Vec<KindDecl> {\n    vec![\n";
    for k in &model.kinds {
        let l = |v: &[easyacft_lang::model::KindField]| v.iter().map(field_lit).collect::<Vec<_>>().join(", ");
        writeln!(o, "        KindDecl {{ name: {:?}.into(), help: {:?}.into(), settings: vec![{}], values: vec![{}], ports: vec![{}], links: vec![{}], file: 0, span: 0..0 }},", k.name, k.help, l(&k.settings), l(&k.values), l(&k.ports), l(&k.links)).unwrap();
    }
    o += "    ]\n}\n\nfn part(domain: &'static str, kind: &'static str, id: &str, slot: usize, values: Vec<(&'static str, Value)>, exprs: &[(&'static str, &str)]) -> CheckedPart {\n    CheckedPart { domain, kind, id: id.into(), values, slot, exprs: exprs.iter().map(|(k, s)| (*k, s.to_string())).collect(), file: 0 }\n}\n\n";
    o += "/// Every part with its settings, as the checker left them.\npub fn checked() -> Checked {\n    let mut slots = Slots::default();\n";
    for (n, t) in &c.slots.names {
        writeln!(o, "    slots.add({n:?}.into(), {});", ty_lit(*t)).unwrap();
    }
    o += "    let parts = vec![\n";
    for p in &c.parts {
        let mut vals = Vec::new();
        for (k, v) in &p.values {
            let s = match v {
                Value::Num(x) => format!("Value::Num({})", lit(*x)),
                Value::Bool(b) => format!("Value::Bool({b})"),
                Value::Text(t) => format!("Value::Text({t:?}.into())"),
                Value::Ref(i) => format!("Value::Ref({i})"),
                Value::Refs(r) => format!("Value::Refs(vec!{r:?})"),
                Value::Expr(prog) => {
                    let src = p.exprs.iter().find(|(e, _)| e == k).map_or(String::new(), |x| x.1.clone());
                    format!("Value::Expr({})", ex.add(prog, &format!("`{}.{k}` = `{}`", p.id, src.replace('`', "'"))))
                }
            };
            vals.push(format!("({k:?}, {s})"));
        }
        let exprs: Vec<String> = p.exprs.iter().map(|(k, s)| format!("({k:?}, {s:?})")).collect();
        writeln!(o, "        part({:?}, {:?}, {:?}, {}, vec![{}], &[{}]),", p.domain, p.kind, p.id, p.slot, vals.join(", "), exprs.join(", ")).unwrap();
    }
    o += "    ];\n    Checked { parts, slots }\n}\n\n// Expressions, compiled from the files.\n\n";
    o
}

fn scenario_rs(text: &str, file: &str, c: &Checked, ex: &mut Exprs, krate: &str) -> Result<String, String> {
    let doc: toml_edit::DocumentMut = text.parse().map_err(|e: toml_edit::TomlError| format!("{file}: {e}"))?;
    let time = |i: Option<&toml_edit::Item>| -> Result<f64, String> {
        let s = i.and_then(|x| x.as_str()).ok_or(format!("{file}: each step and expect needs `at`"))?;
        easyacft_core::parse_quantity(s).ok().filter(|q| q.dim == easyacft_core::Dim::TIME).map(|q| q.si).ok_or(format!("{file}: `{s}` is not a time"))
    };
    let mut steps = Vec::new();
    if let Some(arr) = doc.get("step").and_then(|x| x.as_array_of_tables()) {
        for t in arr.iter() {
            let at = time(t.get("at"))?;
            let set = t.get("set").and_then(|x| x.as_table_like()).ok_or(format!("{file}: a step needs `set`"))?;
            for (k, v) in set.iter() {
                let x = v.as_float().or_else(|| v.as_integer().map(|i| i as f64)).or_else(|| v.as_bool().map(|b| b as u8 as f64)).ok_or(format!("{file}: `{k}` needs a number"))?;
                let slot = c.slots.index.get(k).ok_or(format!("{file}: nothing called `{k}`"))?;
                steps.push(format!("        ({}, v::{}, {}),", lit(at), ex.consts[*slot], lit(x)));
            }
        }
    }
    let mut checks = Vec::new();
    if let Some(arr) = doc.get("expect").and_then(|x| x.as_array_of_tables()) {
        for t in arr.iter() {
            let at = time(t.get("at"))?;
            let that = t.get("that").and_then(|x| x.as_str()).ok_or(format!("{file}: an expect needs `that`"))?;
            let p = easyacft_lang::expr::compile(that, &c.slots).map_err(|e| format!("{file}: `{that}`: {}", e.message))?;
            let prog = ex.add(&p, &format!("expect `{}`", that.replace('`', "'")));
            checks.push(format!("        ({}, {that:?}, {prog}),", lit(at)));
        }
    }
    let lib = krate.replace('-', "_");
    Ok(format!(
        "//! `{file}`, written by `easyacft export`.\n#![allow(unused_imports)]\n\nuse {lib}::*;\nuse easyacft_lang::expr::{{Program, Table, Type}};\nuse easyacft_core::Dim;\n\n#[test]\nfn scenario() {{\n    let mut a = build();\n    let mut steps = vec![\n{}\n    ];\n    steps.sort_by(|x: &(f64, usize, f64), y| x.0.total_cmp(&y.0));\n    let mut checks: Vec<(f64, &str, Program)> = vec![\n{}\n    ];\n    checks.sort_by(|x, y| x.0.total_cmp(&y.0));\n    let end = steps.iter().map(|s| s.0).chain(checks.iter().map(|c| c.0)).fold(0.0, f64::max);\n    let (start, mut si, mut ci, mut failed) = (a.time, 0, 0, Vec::new());\n    loop {{\n        let t = a.time - start;\n        while si < steps.len() && steps[si].0 <= t + 1e-9 {{\n            a.slots[steps[si].1] = steps[si].2;\n            si += 1;\n        }}\n        while ci < checks.len() && checks[ci].0 <= t + 1e-9 {{\n            let (at, that, p) = &checks[ci];\n            let mut st = vec![0.0; p.states];\n            if p.eval(&a.slots, &mut st, 0.0) == 0.0 {{\n                failed.push(format!(\"at {{at}} s: {{that}}\"));\n            }}\n            ci += 1;\n        }}\n        if t > end + 1e-9 || (si >= steps.len() && ci >= checks.len()) {{\n            break;\n        }}\n        a.step(1.0 / 60.0);\n    }}\n    assert!(failed.is_empty(), \"{{failed:#?}}\");\n}}\n\n{}",
        steps.join("\n"),
        checks.join("\n"),
        std::mem::take(&mut ex.code)
    ))
}

fn copy_data(from: &Path, to: &Path) -> std::io::Result<usize> {
    let mut n = 0;
    for e in std::fs::read_dir(from)?.flatten() {
        let p = e.path();
        if p.is_dir() {
            n += copy_data(&p, &to.join(e.file_name()))?;
        } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("csv")) {
            std::fs::create_dir_all(to)?;
            std::fs::copy(&p, to.join(e.file_name()))?;
            n += 1;
        }
    }
    Ok(n)
}

/// Writes the aircraft in `dir` as a crate in `out`.
pub fn export(dir: &Path, out: &Path, name: Option<&str>) -> Result<Summary, String> {
    if out.join("Cargo.toml").exists() {
        return Err(format!("{} already has a Cargo.toml; export into a new folder", out.display()));
    }
    let (model, c) = checked_of(dir)?;
    let krate = crate_name(out, name);
    let io = |e: std::io::Error| e.to_string();
    for (path, text) in FILES {
        let p = out.join("runtime").join(path);
        std::fs::create_dir_all(p.parent().unwrap()).map_err(io)?;
        std::fs::write(&p, runtime_source(path, text)).map_err(io)?;
    }
    for (n, deps) in [("core", &[][..]), ("lang", &["core"][..]), ("parts", &["core", "lang"][..])] {
        std::fs::write(out.join("runtime").join(n).join("Cargo.toml"), runtime_manifest(n, deps)).map_err(io)?;
    }
    std::fs::write(out.join("LICENSE-EXPORT.md"), EXCEPTION).map_err(io)?;
    std::fs::write(
        out.join("Cargo.toml"),
        format!("[package]\nname = \"{krate}\"\nversion = \"0.1.0\"\nedition = \"2021\"\nrust-version = \"1.82\"\nlicense-file = \"LICENSE-EXPORT.md\"\npublish = false\n\n[dependencies]\neasyacft-core = {{ path = \"runtime/core\" }}\neasyacft-lang = {{ path = \"runtime/lang\" }}\neasyacft-parts = {{ path = \"runtime/parts\" }}\n\n[workspace]\nmembers = [\"runtime/core\", \"runtime/lang\", \"runtime/parts\"]\n"),
    )
    .map_err(io)?;
    let mut ex = Exprs { code: String::new(), n: 0, consts: const_names(&c.slots.names) };
    std::fs::create_dir_all(out.join("src")).map_err(io)?;
    let mut src = aircraft_rs(&model, &c, &mut ex);
    src += &std::mem::take(&mut ex.code);
    std::fs::write(out.join("src/lib.rs"), src).map_err(io)?;
    let expressions = ex.n;
    let mut tests = 0;
    if let Ok(rd) = std::fs::read_dir(dir.join("tests")) {
        let mut files: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "toml")).collect();
        files.sort();
        std::fs::create_dir_all(out.join("tests")).map_err(io)?;
        for f in files {
            let text = std::fs::read_to_string(&f).map_err(io)?;
            let stem = f.file_stem().unwrap().to_string_lossy().replace(|c: char| !c.is_ascii_alphanumeric(), "_");
            let code = scenario_rs(&text, &f.file_name().unwrap().to_string_lossy(), &c, &mut ex, &krate)?;
            std::fs::write(out.join("tests").join(format!("{stem}.rs")), code).map_err(io)?;
            tests += 1;
        }
    }
    copy_data(dir, &out.join("data")).map_err(io)?;
    let lib = krate.replace('-', "_");
    let pick = |end: &str| c.slots.names.iter().position(|(n, _)| n.ends_with(end)).map_or(ex.consts[0].clone(), |i| ex.consts[i].clone());
    let (set_c, get_c) = (pick(".on"), pick(".voltage"));
    std::fs::write(
        out.join("README.md"),
        format!(
            "# {}\n\nWritten by `easyacft export`. Everything is here: the aircraft in `src/lib.rs`, the solver and part models in `runtime/`. Nothing else is needed, and LICENSE-EXPORT.md lets you use it under any terms.\n\n```rust\nlet mut a = {lib}::build();\na.slots[{lib}::v::{set_c}] = 1.0;\na.step(1.0 / 60.0);\nlet value = a.slots[{lib}::v::{get_c}];\n```\n\n`cargo test` runs the aircraft's scenario tests. In a shipped plugin call `build_with_data(folder)` with the folder holding `data/`.\n",
            model.name
        ),
    )
    .map_err(io)?;
    Ok(Summary { out: out.to_path_buf(), crate_name: krate, parts: c.parts.len(), values: c.slots.names.len(), expressions, tests })
}
