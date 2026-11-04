//! Aircraft files loaded into part declarations with source spans.

use crate::diag::{Diagnostic, Sources};
use std::ops::Range;
use std::path::{Path, PathBuf};
use toml_edit::{ImDocument, Item, Table, Value};

#[derive(Clone, Debug, PartialEq)]
pub enum Field {
    Str(String),
    Num(f64),
    Bool(bool),
    List(Vec<String>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct FieldDecl {
    pub key: String,
    pub value: Field,
    pub file: usize,
    pub span: Range<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PartDecl {
    pub domain: String,
    pub kind: String,
    pub id: String,
    pub file: usize,
    pub span: Range<usize>,
    pub fields: Vec<FieldDecl>,
}

impl PartDecl {
    pub fn get(&self, key: &str) -> Option<&FieldDecl> {
        self.fields.iter().find(|f| f.key == key)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Binding {
    pub dataref: String,
    pub path: String,
    pub write: bool,
    /// The dataref's unit: SI = raw * factor + offset.
    pub factor: f64,
    pub offset: f64,
    pub file: usize,
    pub span: Range<usize>,
}

/// A setting or value of a developer's own part kind, as written: `"L/min"`, `"expr bool"`, `"part electrical.bus"`.
#[derive(Clone, Debug, PartialEq)]
pub struct KindField {
    pub key: String,
    pub ty: String,
    pub default: Option<String>,
    pub help: String,
    pub span: Range<usize>,
}

/// `[kinds.<name>]`: a part kind the developer declares and steps with their own code.
#[derive(Clone, Debug, PartialEq)]
pub struct KindDecl {
    pub name: String,
    pub help: String,
    pub settings: Vec<KindField>,
    pub values: Vec<KindField>,
    pub file: usize,
    pub span: Range<usize>,
}

#[derive(Clone, Debug, Default)]
pub struct Model {
    pub name: String,
    pub version: String,
    pub parts: Vec<PartDecl>,
    pub bindings: Vec<Binding>,
    pub rates: Vec<(String, f64)>,
    pub kinds: Vec<KindDecl>,
    pub sources: Sources,
}

impl Model {
    pub fn part(&self, id: &str) -> Option<&PartDecl> {
        self.parts.iter().find(|p| p.id == id)
    }
}

fn span(item: &Item) -> Range<usize> {
    item.span().unwrap_or(0..0)
}

fn value_span(v: &Value) -> Range<usize> {
    v.span().unwrap_or(0..0)
}

fn field(v: &Value) -> Option<Field> {
    Some(match v {
        Value::String(s) => Field::Str(s.value().clone()),
        Value::Integer(i) => Field::Num(*i.value() as f64),
        Value::Float(f) => Field::Num(*f.value()),
        Value::Boolean(b) => Field::Bool(*b.value()),
        Value::Array(a) => Field::List(a.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect()),
        _ => return None,
    })
}

fn substitute(f: &Field, n: usize) -> Field {
    let s = |x: &str| x.replace("{n}", &n.to_string());
    match f {
        Field::Str(x) => Field::Str(s(x)),
        Field::List(v) => Field::List(v.iter().map(|x| s(x)).collect()),
        other => other.clone(),
    }
}

struct Loader {
    model: Model,
    diags: Vec<Diagnostic>,
    seen: Vec<PathBuf>,
    variant: bool,
}

const RESERVED: [&str; 6] = ["aircraft", "include", "bindings", "rates", "variant", "kinds"];

impl Loader {
    fn file(&mut self, path: &Path, from: Option<(usize, Range<usize>)>) {
        let canon = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        if self.seen.contains(&canon) {
            return;
        }
        self.seen.push(canon);
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) => {
                let (file, sp) = from.unwrap_or((usize::MAX, 0..0));
                self.diags.push(Diagnostic::error("E0001", format!("cannot read `{}`: {e}", path.display()), file, sp));
                return;
            }
        };
        let name = path.display().to_string().replace('\\', "/");
        let id = self.model.sources.add(name, text.clone());
        let doc = match ImDocument::parse(text) {
            Ok(d) => d,
            Err(e) => {
                let sp = e.span().unwrap_or(0..0);
                self.diags.push(Diagnostic::error("E0002", format!("this is not valid TOML: {}", e.message()), id, sp));
                return;
            }
        };
        let root = doc.as_table();
        let was_variant = self.variant;
        if let Some(v) = root.get("variant").and_then(Item::as_table) {
            match v.get("of").and_then(Item::as_str) {
                Some(of) => {
                    let base = path.parent().unwrap_or(Path::new(".")).join(of);
                    let base = if base.is_dir() { base.join("aircraft.toml") } else { base };
                    self.file(&base, Some((id, span(v.get("of").unwrap()))));
                }
                None => self.diags.push(Diagnostic::error("E0010", "a [variant] needs `of`, the aircraft it changes", id, span(root.get("variant").unwrap())).help("for example `of = \"../twin-jet\"`")),
            }
            if let Some(r) = v.get("remove").and_then(Item::as_array) {
                for x in r.iter() {
                    let Some(rid) = x.as_str() else { continue };
                    let before = self.model.parts.len();
                    self.model.parts.retain(|p| p.id != rid);
                    if self.model.parts.len() == before {
                        self.diags.push(Diagnostic::error("E0011", format!("the base aircraft has no `{rid}` to remove"), id, value_span(x)));
                    }
                }
            }
            self.variant = true;
        }
        if let Some(a) = root.get("aircraft").and_then(Item::as_table) {
            if let Some(n) = a.get("name").and_then(Item::as_str) {
                self.model.name = n.to_owned();
            }
            if let Some(v) = a.get("easyacft").and_then(Item::as_str) {
                self.model.version = v.to_owned();
            }
        }
        if let Some(r) = root.get("rates").and_then(Item::as_table) {
            for (k, v) in r.iter() {
                match v.as_str().map(easyacft_core::parse_quantity) {
                    Some(Ok(q)) if q.dim == easyacft_core::Dim::TIME && q.si > 0.0 => self.model.rates.push((k.to_owned(), q.si)),
                    _ => self.diags.push(Diagnostic::error("E0104", format!("rate `{k}` must be a time such as \"2 ms\""), id, span(v))),
                }
            }
        }
        if let Some(b) = root.get("bindings").and_then(Item::as_table) {
            self.bindings(b, id);
        }
        if let Some(k) = root.get("kinds").and_then(Item::as_table) {
            self.kinds(k, id);
        }
        let base = path.parent().unwrap_or(Path::new("."));
        let inc = root.get("include").or_else(|| root.get("aircraft").and_then(Item::as_table).and_then(|a| a.get("include")));
        if let Some(inc) = inc {
            match inc.as_array() {
                Some(arr) => {
                    for v in arr.iter() {
                        match v.as_str() {
                            Some(p) => self.file(&base.join(p), Some((id, value_span(v)))),
                            None => self.diags.push(Diagnostic::error("E0003", "`include` lists file paths", id, value_span(v))),
                        }
                    }
                }
                None => self.diags.push(Diagnostic::error("E0003", "`include` must be a list of file paths", id, span(inc))),
            }
        }
        for (domain, item) in root.iter().filter(|(k, _)| !RESERVED.contains(k)) {
            let Some(table) = item.as_table() else {
                self.diags.push(Diagnostic::error("E0004", format!("`{domain}` should be a section such as [[{domain}.battery]]"), id, span(item)));
                continue;
            };
            self.domain(domain, table, id);
        }
        self.variant = was_variant;
    }

    fn bindings(&mut self, b: &Table, id: usize) {
        for (k, v) in b.iter() {
            let mut unit = (1.0, 0.0);
            let (path, write) = match v {
                Item::Value(Value::String(s)) => (s.value().clone(), false),
                Item::Value(Value::InlineTable(t)) => {
                    let from = t.get("from").and_then(|x| x.as_str()).map(str::to_owned);
                    let to = t.get("to").and_then(|x| x.as_str()).map(str::to_owned);
                    if let Some(u) = t.get("unit").and_then(|x| x.as_str()) {
                        match easyacft_core::units::parse_unit(u) {
                            Ok((f, o, _)) => unit = (f, o),
                            Err(e) => {
                                self.diags.push(Diagnostic::error("E0005", format!("binding unit: {e}"), id, span(v)));
                                continue;
                            }
                        }
                    }
                    match (from, to) {
                        (Some(f), None) => (f, false),
                        (None, Some(t)) => (t, true),
                        _ => {
                            self.diags.push(Diagnostic::error("E0005", "a binding needs `from` (EasyAcft to X-Plane) or `to` (X-Plane to EasyAcft)", id, span(v)));
                            continue;
                        }
                    }
                }
                _ => {
                    self.diags.push(Diagnostic::error("E0005", "a binding is a value path or { from = ... } / { to = ... }", id, span(v)));
                    continue;
                }
            };
            self.model.bindings.push(Binding { dataref: k.to_owned(), path, write, factor: unit.0, offset: unit.1, file: id, span: span(v) });
        }
    }

    fn kinds(&mut self, t: &Table, id: usize) {
        for (name, item) in t.iter() {
            let Some(k) = item.as_table() else {
                self.diags.push(Diagnostic::error("E0013", format!("write a part kind as a [kinds.{name}] section"), id, span(item)));
                continue;
            };
            let mut decl = KindDecl { name: name.to_owned(), help: k.get("help").and_then(Item::as_str).unwrap_or("").to_owned(), settings: vec![], values: vec![], file: id, span: span(item) };
            for (part, list) in [("settings", &mut decl.settings), ("values", &mut decl.values)] {
                let Some(sec) = k.get(part) else { continue };
                let Some(sec) = sec.as_table_like() else {
                    self.diags.push(Diagnostic::error("E0013", format!("`{part}` is a table of name = type"), id, span(sec)));
                    continue;
                };
                for (key, v) in sec.iter() {
                    let f = match v.as_str() {
                        Some(ty) => Some(KindField { key: key.to_owned(), ty: ty.to_owned(), default: None, help: String::new(), span: span(v) }),
                        None => v.as_table_like().and_then(|t| {
                            let s = |n: &str| t.get(n).and_then(|x| x.as_str()).map(str::to_owned);
                            Some(KindField { key: key.to_owned(), ty: s("type")?, default: s("default"), help: s("help").unwrap_or_default(), span: span(v) })
                        }),
                    };
                    match f {
                        Some(f) => list.push(f),
                        None => self.diags.push(Diagnostic::error("E0013", format!("`{key}` needs a type, such as \"L/min\" or {{ type = \"expr bool\", default = \"false\" }}"), id, span(v))),
                    }
                }
            }
            self.model.kinds.retain(|x| x.name != decl.name);
            self.model.kinds.push(decl);
        }
    }

    fn domain(&mut self, domain: &str, table: &Table, id: usize) {
        for (kind, item) in table.iter() {
            let tables: Vec<&Table> = match item {
                Item::ArrayOfTables(a) => a.iter().collect(),
                Item::Table(t) => vec![t],
                _ => {
                    self.diags.push(Diagnostic::error("E0004", format!("write parts as [[{domain}.{kind}]] tables"), id, span(item)));
                    continue;
                }
            };
            for t in tables {
                let sp = t.span().unwrap_or(0..0);
                let count = t.get("count").and_then(Item::as_integer).unwrap_or(1).max(1) as usize;
                let Some(raw_id) = t.get("id").and_then(Item::as_str) else {
                    self.diags.push(Diagnostic::error("E0006", format!("this {domain}.{kind} has no `id`"), id, sp).help("add `id = \"name\"`"));
                    continue;
                };
                if count > 1 && !raw_id.contains("{n}") {
                    self.diags.push(Diagnostic::error("E0007", "a part with `count` needs `{n}` in its id", id, span(t.get("id").unwrap())).help(format!("for example `id = \"{raw_id}{{n}}\"`")));
                    continue;
                }
                let mut fields = Vec::new();
                for (k, v) in t.iter().filter(|(k, _)| *k != "id" && *k != "count") {
                    let Some(val) = v.as_value() else {
                        self.diags.push(Diagnostic::error("E0008", format!("`{k}` must be a value"), id, span(v)));
                        continue;
                    };
                    match field(val) {
                        Some(f) => fields.push(FieldDecl { key: k.to_owned(), value: f, file: id, span: value_span(val) }),
                        None => self.diags.push(Diagnostic::error("E0008", format!("`{k}` must be text, a number, true/false or a list"), id, value_span(val))),
                    }
                }
                for n in 1..=count {
                    let pid = raw_id.replace("{n}", &n.to_string());
                    if self.variant {
                        if let Some(prev) = self.model.parts.iter_mut().find(|p| p.id == pid) {
                            if prev.domain != domain || prev.kind != kind {
                                let msg = format!("`{pid}` is a {}.{} in the base aircraft, not a {domain}.{kind}", prev.domain, prev.kind);
                                self.diags.push(Diagnostic::error("E0012", msg, id, span(t.get("id").unwrap())));
                                continue;
                            }
                            for f in &fields {
                                let f = FieldDecl { value: substitute(&f.value, n), ..f.clone() };
                                match prev.fields.iter_mut().find(|x| x.key == f.key) {
                                    Some(x) => *x = f,
                                    None => prev.fields.push(f),
                                }
                            }
                            continue;
                        }
                    }
                    if let Some(prev) = self.model.parts.iter().find(|p| p.id == pid) {
                        let at = prev.span.clone();
                        let pf = prev.file;
                        self.diags.push(Diagnostic::error("E0009", format!("`{pid}` is defined twice"), id, span(t.get("id").unwrap())).help(format!("the first is in {} at byte {}", self.model.sources.files.get(pf).map_or("?", |f| f.0.as_str()), at.start)));
                        continue;
                    }
                    let fields = fields.iter().map(|f| FieldDecl { value: substitute(&f.value, n), ..f.clone() }).collect();
                    self.model.parts.push(PartDecl { domain: domain.to_owned(), kind: kind.to_owned(), id: pid, file: id, span: sp.clone(), fields });
                }
            }
        }
    }
}

/// Loads `aircraft.toml` (or the file given) and everything it includes.
pub fn load(path: &Path) -> (Model, Vec<Diagnostic>) {
    let root = if path.is_dir() { path.join("aircraft.toml") } else { path.to_path_buf() };
    let mut l = Loader { model: Model::default(), diags: Vec::new(), seen: Vec::new(), variant: false };
    l.file(&root, None);
    (l.model, l.diags)
}

/// Loads from text, for tests and editors.
pub fn load_str(name: &str, text: &str) -> (Model, Vec<Diagnostic>) {
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let k = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("easyacft-{}-{k}-{}", std::process::id(), name.replace(['/', '\\'], "_")));
    let _ = std::fs::create_dir_all(&dir);
    let p = dir.join("aircraft.toml");
    let _ = std::fs::write(&p, text);
    let out = load(&p);
    let _ = std::fs::remove_dir_all(&dir);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parts_counts_includes_and_bindings() {
        let dir = std::env::temp_dir().join(format!("easyacft-model-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("systems")).unwrap();
        std::fs::write(
            dir.join("aircraft.toml"),
            "[aircraft]\nname = \"Example\"\neasyacft = \"1.0\"\ninclude = [\"systems/elec.toml\"]\n[rates]\nelectrical = \"5 ms\"\n[bindings]\n\"sim/a\" = \"bat1.voltage\"\n\"sim/b\" = { to = \"switch.bat1\" }\n",
        )
        .unwrap();
        std::fs::write(dir.join("systems/elec.toml"), "[[electrical.battery]]\nid = \"bat{n}\"\ncount = 2\nvoltage = \"24 V\"\n[[electrical.bus]]\nid = \"dc1\"\n").unwrap();
        let (m, d) = load(&dir);
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(d.is_empty(), "{d:?}");
        assert_eq!(m.name, "Example");
        assert_eq!(m.parts.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), ["bat1", "bat2", "dc1"]);
        assert_eq!(m.part("bat2").unwrap().get("voltage").unwrap().value, Field::Str("24 V".into()));
        assert_eq!(m.rates, vec![("electrical".to_owned(), 0.005)]);
        assert_eq!(m.bindings.len(), 2);
        assert!(m.bindings[1].write);
    }

    #[test]
    fn mistakes_are_reported_with_their_place() {
        let (_, d) = load_str("dup", "[[electrical.bus]]\nid = \"dc1\"\n[[electrical.bus]]\nid = \"dc1\"\n[[electrical.bus]]\nnominal = \"28 V\"\n");
        let codes: Vec<_> = d.iter().map(|x| x.code).collect();
        assert_eq!(codes, ["E0009", "E0006"]);
        let (_, d) = load_str("bad", "[[electrical.bus]\nid = 1");
        assert_eq!(d[0].code, "E0002");
        let (_, d) = load_str("inc", "include = [\"missing.toml\"]\n");
        assert_eq!(d[0].code, "E0001");
    }

    #[test]
    fn variants_change_add_and_remove_parts() {
        let dir = std::env::temp_dir().join(format!("easyacft-variant-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("base")).unwrap();
        std::fs::create_dir_all(dir.join("long")).unwrap();
        std::fs::write(dir.join("base/aircraft.toml"), "[aircraft]
name = \"Base\"
[[electrical.bus]]
id = \"dc{n}\"
count = 2
nominal = \"28 V\"
[[electrical.bus]]
id = \"spare\"
").unwrap();
        std::fs::write(dir.join("long/aircraft.toml"), "[variant]
of = \"../base\"
remove = [\"spare\"]
[aircraft]
name = \"Long\"
[[electrical.bus]]
id = \"dc2\"
nominal = \"24 V\"
[[electrical.bus]]
id = \"dc3\"
").unwrap();
        let (m, d) = load(&dir.join("long"));
        assert!(d.is_empty(), "{d:?}");
        assert_eq!(m.name, "Long");
        let ids: Vec<&str> = m.parts.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, ["dc1", "dc2", "dc3"]);
        assert_eq!(m.part("dc2").unwrap().get("nominal").unwrap().value, Field::Str("24 V".into()));
        assert_eq!(m.part("dc1").unwrap().get("nominal").unwrap().value, Field::Str("28 V".into()));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
