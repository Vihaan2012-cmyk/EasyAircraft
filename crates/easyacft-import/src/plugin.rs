//! Reads the A380 for X-Plane 12 plugin's source and catalogue: deep areas, loads, breakers, failures, alerts.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Area {
    pub name: String,
    pub files: usize,
    pub lines: usize,
    pub failures: Vec<(String, String)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Load {
    pub id: String,
    pub name: String,
    pub ata: u32,
    pub bus: String,
    pub watts: f64,
    pub pf: f64,
    pub kind: &'static str,
    pub second: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Deep {
    pub root: String,
    pub areas: Vec<Area>,
    pub loads: Vec<Load>,
    pub estimated: usize,
    pub catalogue: Option<Catalogue>,
    /// The plugin's engine model constants, from `src/physics/engine/params.rs`.
    pub engine: HashMap<String, f64>,
}

/// A consumer the network was given: (load id, its bus, load type, ATA, name).
pub type Unit = (String, String, &'static str, u32, String);

use super::catalogue::{bus_name, Breaker, Catalogue};
use super::msfs_fuel::id as ident;

/// Export the plugin's catalogue with its own test exporter, into `json`.
pub fn export_catalogue(root: &Path, json: &Path) -> Result<(), String> {
    let st = std::process::Command::new("cargo")
        .current_dir(root)
        .env("CATALOGUE_OUT", json)
        .args(["test", "--lib", "--", "--ignored", "--exact", "study::catalogue::tests::dump_catalogue"])
        .stdout(std::process::Stdio::null())
        .status()
        .map_err(|e| format!("cannot run cargo in `{}`: {e}", root.display()))?;
    if !st.success() || !json.is_file() {
        return Err(format!("the plugin's catalogue export failed in `{}`", root.display()));
    }
    Ok(())
}

/// Input ids for every component parameter, unique.
fn param_ids(c: &Catalogue) -> HashMap<(String, String), String> {
    let mut out = HashMap::new();
    let mut seen = BTreeSet::new();
    for comp in &c.components {
        for (p, _, _) in &comp.params {
            let base = super::msfs_fuel::id(&format!("p {} {p}", comp.id));
            let mut id = base.clone();
            let mut n = 2;
            while !seen.insert(id.clone()) {
                id = format!("{base}_{n}");
                n += 1;
            }
            out.insert((comp.id.clone(), p.clone()), id);
        }
    }
    out
}

fn any_failed(ids: &[u64]) -> String {
    if ids.is_empty() {
        "false".into()
    } else {
        ids.iter().map(|i| format!("f{i}.value > 0")).collect::<Vec<_>>().join(" || ")
    }
}

fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `systems/failures.toml`, `systems/components.toml` and `systems/ecam.toml` from the catalogue.
pub fn catalogue_files(c: &Catalogue, head: &str) -> Vec<(String, String)> {
    let mut f = head.to_owned();
    for (id, name, ata, comp) in &c.failures {
        writeln!(f, "# {} (ATA {ata}, {})\n[[input.number]]\nid = \"f{id}\"\n", one_line(name), one_line(comp)).unwrap();
    }
    let ids = param_ids(c);
    let mut p = head.to_owned();
    for comp in &c.components {
        for (name, healthy, meaning) in &comp.params {
            let m = one_line(meaning);
            let m: String = m.chars().take(160).collect();
            writeln!(p, "# {} {name}: {m}\n[[input.number]]\nid = \"{}\"\ninitial = {healthy:?}\n", one_line(&comp.name), ids[&(comp.id.clone(), name.clone())]).unwrap();
        }
    }
    let mut e = head.to_owned();
    let mut seen = BTreeSet::new();
    for (key, title, level, fs) in &c.alerts {
        let id = format!("ecam_{}", super::msfs_fuel::id(key));
        if !seen.insert(id.clone()) {
            continue;
        }
        writeln!(e, "# {} ({level})\n[[logic.flag]]\nid = \"{id}\"\nwhen = \"{}\"\n", one_line(title), any_failed(fs)).unwrap();
    }
    vec![("systems/failures.toml".into(), f), ("systems/components.toml".into(), p), ("systems/ecam.toml".into(), e)]
}

fn base_load(id: &str) -> &str {
    for suf in ["-normal-bkr", "-2nd-bkr", "-3rd-bkr", "-bkr"] {
        if let Some(b) = id.strip_suffix(suf) {
            return b;
        }
    }
    id
}

/// The plugin's root: a folder with `src/deep`, from `path` or above it.
pub fn find(path: &Path) -> Option<PathBuf> {
    let mut at = Some(path);
    for _ in 0..8 {
        let d = at?;
        if d.join("src").join("deep").is_dir() {
            return Some(d.to_path_buf());
        }
        at = d.parent();
    }
    None
}

fn rs_lines(dir: &Path, files: &mut usize, lines: &mut usize) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            rs_lines(&p, files, lines);
        } else if p.extension().is_some_and(|x| x == "rs") {
            *files += 1;
            *lines += std::fs::read_to_string(&p).map(|t| t.lines().count()).unwrap_or(0);
        }
    }
}

/// Rows of a markdown table whose first cell is an ATA number: (ATA, name).
fn failures(md: &str) -> Vec<(String, String)> {
    md.lines()
        .filter_map(|l| {
            let cells: Vec<&str> = l.trim().trim_matches('|').split('|').map(|c| c.trim()).collect();
            (cells.len() >= 2 && !cells[0].is_empty() && cells[0].chars().all(|c| c.is_ascii_digit())).then(|| (cells[0].to_owned(), cells[1].replace('`', "")))
        })
        .collect()
}

/// Splits a call's argument list at top-level commas, from just after its opening parenthesis.
fn args(text: &str) -> (Vec<String>, usize) {
    let mut depth = 0i32;
    let mut cur = String::new();
    let mut out = Vec::new();
    let mut in_str = false;
    for (i, c) in text.char_indices() {
        match c {
            '"' => {
                in_str = !in_str;
                cur.push(c);
            }
            '(' | '[' if !in_str => {
                depth += 1;
                cur.push(c);
            }
            ')' | ']' if !in_str => {
                if depth == 0 {
                    out.push(cur.trim().to_owned());
                    return (out, i);
                }
                depth -= 1;
                cur.push(c);
            }
            ',' if !in_str && depth == 0 => {
                out.push(cur.trim().to_owned());
                cur.clear();
            }
            _ => cur.push(c),
        }
    }
    (out, text.len())
}

/// Tables like `const FANS: [(&str, &str, BusId); 4] = [("a", "A", BusId::Ac1), ...];`.
fn tables(text: &str) -> HashMap<String, Vec<Vec<String>>> {
    let mut out = HashMap::new();
    let mut rest = text;
    while let Some(i) = rest.find("const ") {
        rest = &rest[i + 6..];
        let Some(colon) = rest.find(':') else { break };
        let name = rest[..colon].trim().to_owned();
        let Some(eq) = rest.find("] = [") else { break };
        // A plain list of names, `const IDS: [&str; 3] = ["a", "b", "c"];`: one single-field row each.
        if rest[colon..eq].contains("[&str") && !rest[colon..eq].contains('\n') {
            let body = &rest[eq + 5..];
            let list = &body[..body.find("];").unwrap_or(body.len())];
            let rows: Vec<Vec<String>> = list.split(',').map(|s| s.trim()).filter(|s| s.starts_with('"')).map(|s| vec![s.to_owned()]).collect();
            if !rows.is_empty() {
                out.insert(name, rows);
            }
            continue;
        }
        if !rest[colon..eq].contains("[(") {
            continue;
        }
        let body = &rest[eq + 5..];
        let mut rows = Vec::new();
        let mut b = body;
        loop {
            let t = b.trim_start();
            if t.starts_with(']') || t.is_empty() {
                break;
            }
            let Some(open) = b.find('(') else { break };
            if b[..open].contains(']') {
                break;
            }
            let (row, end) = args(&b[open + 1..]);
            rows.push(row);
            b = &b[open + 1 + end + 1..];
        }
        if !rows.is_empty() {
            out.insert(name, rows);
        }
    }
    out
}

fn unquote(s: &str) -> String {
    s.trim().trim_matches('"').to_owned()
}

/// A little arithmetic over numbers and table variables: `500.0`, `watts`, `seats as f64 * 40.0`.
fn value(expr: &str, vars: &HashMap<String, String>) -> Option<f64> {
    let e = expr.replace("as f64", "").replace("_f64", "");
    let mut total = 1.0;
    for factor in e.split('*') {
        let f = factor.trim();
        let v = vars.get(f).map(|s| s.as_str()).unwrap_or(f);
        total *= v.trim().replace('_', "").parse::<f64>().ok()?;
    }
    Some(total)
}

fn bus_of(s: &str, vars: &HashMap<String, String>) -> Option<String> {
    let s = s.trim();
    let s = vars.get(s).map(|v| v.as_str()).unwrap_or(s);
    s.strip_prefix("BusId::").map(|b| b.to_owned())
}

/// Every load spec call in `loads.rs`, with table loops unrolled.
pub fn loads(text: &str) -> (Vec<Load>, usize) {
    let tabs = tables(text);
    // Loop spans: `for (a, b, c) in TABLE {` to its closing brace.
    let mut spans: Vec<(usize, usize, Vec<String>, String)> = Vec::new();
    let mut at = 0;
    while let Some(i) = text[at..].find("for (") {
        let start = at + i;
        let Some(close) = text[start..].find(") in ") else { break };
        let vars: Vec<String> = text[start + 5..start + close].split(',').map(|v| v.trim().to_owned()).collect();
        let after = &text[start + close + 5..];
        let tname: String = after.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
        let Some(brace) = after.find('{') else { break };
        let body_start = start + close + 5 + brace + 1;
        let mut depth = 1;
        let mut end = body_start;
        for (k, c) in text[body_start..].char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = body_start + k;
                        break;
                    }
                }
                _ => {}
            }
        }
        spans.push((body_start, end, vars, tname));
        at = start + 5;
    }
    // `for id in IDS {` over a plain list of names.
    let mut at = 0;
    while let Some(i) = text[at..].find("for ") {
        let start = at + i;
        at = start + 4;
        let rest = &text[start + 4..];
        let var: String = rest.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
        let Some(after) = rest[var.len()..].strip_prefix(" in ") else { continue };
        let tname: String = after.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
        if var.is_empty() || tname.is_empty() || !tabs.contains_key(&tname) {
            continue;
        }
        let Some(brace) = after.find('{') else { continue };
        let body_start = start + 4 + var.len() + 4 + brace + 1;
        let mut depth = 1;
        let mut end = body_start;
        for (k, c) in text[body_start..].char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = body_start + k;
                        break;
                    }
                }
                _ => {}
            }
        }
        spans.push((body_start, end, vec![var], tname));
    }
    let mut out = Vec::new();
    let mut estimated = 0;
    let kinds = [("frequency_sensitive_motor_spec(", "induction_motor"), ("motor_spec(", "motor"), ("resistive_spec(", "resistive"), ("avionics_spec(", "constant_power")];
    let mut pos = 0;
    while pos < text.len() {
        let next = kinds.iter().filter_map(|(k, kind)| text[pos..].find(k).map(|i| (pos + i, k.len(), *kind))).min_by_key(|x| (x.0, usize::MAX - x.1));
        let Some((i, len, kind)) = next else { break };
        pos = i + len;
        // Skip the helpers' own definitions and the motor_spec inside frequency_sensitive_motor_spec.
        if text[..i].ends_with("fn ") || text[..i].ends_with("frequency_sensitive_") {
            continue;
        }
        let (a, end) = args(&text[pos..]);
        if a.len() < 5 {
            continue;
        }
        // Dual and triple feeds name their other buses on the add call that follows.
        let tail = &text[pos + end..(pos + end + 400).min(text.len())];
        let seconds: Vec<String> = if let Some(k) = tail.find("add_dual(").or_else(|| tail.find("add_triple(")) {
            let (b, _) = args(&tail[k + tail[k..].find('(').unwrap() + 1..]);
            b.iter().skip(5).cloned().collect()
        } else {
            Vec::new()
        };
        let span = spans.iter().filter(|s| s.0 <= i && i < s.1).max_by_key(|s| s.0);
        let rows: Vec<HashMap<String, String>> = match span {
            Some((_, _, vars, t)) => tabs.get(t).map(|rows| rows.iter().map(|r| vars.iter().cloned().zip(r.iter().cloned()).collect()).collect()).unwrap_or_default(),
            None => vec![HashMap::new()],
        };
        for vars in rows {
            let get = |k: usize| {
                let s = a[k].trim().to_owned();
                vars.get(&s).cloned().unwrap_or(s)
            };
            let Some(bus) = bus_of(&a[3], &vars) else { continue };
            let watts = value(&a[4], &vars);
            if watts.is_none() {
                estimated += 1;
            }
            let pf = if kind == "motor" || kind == "induction_motor" { a.get(5).and_then(|p| value(p, &vars)).unwrap_or(0.85) } else { 1.0 };
            let second = seconds.iter().filter_map(|s| bus_of(s, &vars)).collect();
            let id = unquote(&get(0));
            // A name built at run time from the id, as `Box::leak(id.replace(..).to_uppercase())`.
            let name = if get(1).starts_with('"') { unquote(&get(1)) } else { id.replace('-', " ").to_uppercase() };
            out.push(Load { id, name, ata: get(2).trim().parse().unwrap_or(0), bus, watts: watts.unwrap_or(100.0), pf, kind, second });
        }
    }
    let mut seen = BTreeSet::new();
    out.retain(|l| seen.insert(l.id.clone()));
    (out, estimated)
}

pub fn scan(root: &Path) -> Deep {
    let deep_dir = root.join("src").join("deep");
    let mut d = Deep { root: root.display().to_string().replace('\\', "/"), ..Default::default() };
    if let Ok(rd) = std::fs::read_dir(&deep_dir) {
        let mut dirs: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
        dirs.sort();
        for p in dirs {
            let (mut files, mut lines) = (0, 0);
            rs_lines(&p, &mut files, &mut lines);
            let f = std::fs::read_to_string(p.join("FAILURES.md")).map(|t| failures(&t)).unwrap_or_default();
            d.areas.push(Area { name: p.file_name().unwrap().to_string_lossy().into_owned(), files, lines, failures: f });
        }
    }
    if let Ok(t) = std::fs::read_to_string(root.join("src").join("physics").join("engine").join("params.rs")) {
        d.engine = consts(&t);
    }
    if let Ok(t) = std::fs::read_to_string(deep_dir.join("electrical").join("loads.rs")) {
        let (l, e) = loads(&t);
        d.loads = l;
        d.estimated = e;
    }
    d
}

fn bus_id(b: &str) -> String {
    let mut out = String::new();
    for (i, c) in b.chars().enumerate() {
        if c.is_ascii_uppercase() && i > 0 {
            out.push('_');
        }
        out.push(c.to_ascii_lowercase());
    }
    out.replace("gnd_flt_svc", "gnd_flt")
}

fn is_ac(b: &str) -> bool {
    b.starts_with("Ac") || b.starts_with("Galley")
}

/// Standard aircraft breaker sizes, amperes.
const SIZES: [f64; 20] = [1.0, 2.0, 2.5, 3.0, 5.0, 7.5, 10.0, 15.0, 20.0, 25.0, 30.0, 35.0, 40.0, 50.0, 60.0, 70.0, 75.0, 100.0, 125.0, 150.0];

fn breaker_size(watts: f64, volts: f64, pf: f64) -> f64 {
    let i = watts / (volts * pf.max(0.1)) * 1.25;
    SIZES.iter().copied().find(|&s| s >= i).unwrap_or(150.0)
}

/// The A380's sources and feed paths joined to every catalogue and load bus, each load behind its own breakers.
pub fn electrical(d: &Deep, head: &str, engines: usize) -> (String, Vec<Unit>) {
    let mut f = head.to_owned();
    let mut units: Vec<Unit> = Vec::new();
    f += "# Electrical network from the plugin's load catalogue: four 150 kVA VFGs on AC 1-4, BCRUs to DC 1, DC 2 and DC ESS,\n";
    f += "# AC ESS from AC 1, batteries on the hot buses. Feed paths follow the A380's public architecture; refine with the plugin's\n";
    f += "# own transfer logic.\n\n";
    let mut buses: BTreeMap<String, bool> = BTreeMap::new();
    for b in ["Ac1", "Ac2", "Ac3", "Ac4", "AcEss", "Dc1", "Dc2", "DcEss", "DcBat", "DcHot1", "DcHot2"] {
        buses.insert(b.to_owned(), is_ac(b));
    }
    for l in &d.loads {
        buses.insert(l.bus.clone(), is_ac(&l.bus));
        for s in &l.second {
            buses.insert(s.clone(), is_ac(s));
        }
    }
    let cat = d.catalogue.as_ref();
    let mut shorts: HashMap<String, Vec<u64>> = HashMap::new();
    if let Some(c) = cat {
        for b in &c.breakers {
            buses.insert(b.bus.clone(), is_ac(&b.bus));
        }
        for comp in &c.components {
            if let Some(name) = comp.id.strip_prefix("24_elec.bus.") {
                shorts.entry(bus_name(name)).or_default().extend(&comp.failures);
            }
        }
    }
    for (b, ac) in &buses {
        let short = shorts.get(b).filter(|v| !v.is_empty()).map(|v| format!("short_when = \"{}\"\n", any_failed(v))).unwrap_or_default();
        writeln!(f, "[[electrical.bus]]\nid = \"{}\"\nnominal = \"{}\"\n{short}", bus_id(b), if *ac { "115 V" } else { "28 V" }).unwrap();
    }
    for n in 1..=4 {
        let eng = if engines == 0 { "false".to_owned() } else { format!("eng{}.running", (n - 1) % engines + 1) };
        writeln!(f, "[[electrical.bus]]\nid = \"vfg{n}_out\"\nnominal = \"115 V\"\n\n[[electrical.generator]]\nid = \"vfg{n}\"\nbus = \"vfg{n}_out\"\nvoltage = \"115 V\"\nfrequency = \"400 Hz\"\nrating = \"150 kVA\"\nvariable_frequency = true\ndriven_when = \"{eng}\"\n\n[[electrical.contactor]]\nid = \"glc{n}\"\nbetween = [\"vfg{n}_out\", \"ac{n}\"]\nclose_when = \"vfg{n}.ready\"\n").unwrap();
    }
    for (g, bus, vfg) in [("a", "ac2", "glc2"), ("b", "ac3", "glc3")] {
        let n = &bus[2..];
        writeln!(f, "[[electrical.bus]]\nid = \"apu_gen_{g}_out\"\nnominal = \"115 V\"\n\n[[electrical.generator]]\nid = \"apu_gen_{g}\"\nbus = \"apu_gen_{g}_out\"\nvoltage = \"115 V\"\nfrequency = \"400 Hz\"\nrating = \"120 kVA\"\ndriven_when = \"apu.available\"\n\n[[electrical.contactor]]\nid = \"apu_glc_{g}\"\nbetween = [\"apu_gen_{g}_out\", \"{bus}\"]\nclose_when = \"apu_gen_{g}.ready && !{vfg}.closed && !epc{n}.closed\"\n").unwrap();
    }
    f += "# External power: four 90 kVA receptacles, one per AC bus, taken only while that side's own generator is off line.\n";
    for n in 1..=4 {
        writeln!(f, "[[cockpit.switch]]\nid = \"ext_pwr_{n}\"\n\n[[electrical.bus]]\nid = \"ext{n}_out\"\nnominal = \"115 V\"\n\n[[electrical.ext_power]]\nid = \"ext{n}\"\nbus = \"ext{n}_out\"\nvoltage = \"115 V\"\nfrequency = \"400 Hz\"\nrating = \"90 kVA\"\nconnected_when = \"ext_pwr_{n}.on\"\n\n[[electrical.contactor]]\nid = \"epc{n}\"\nbetween = [\"ext{n}_out\", \"ac{n}\"]\nclose_when = \"ext{n}.available && !glc{n}.closed\"\n").unwrap();
    }
    let feed = |f: &mut String, id: &str, from: &str, to: &str| {
        if buses.contains_key(to) && from != to {
            writeln!(f, "[[electrical.contactor]]\nid = \"{id}\"\nbetween = [\"{}\", \"{}\"]\n", bus_id(from), bus_id(to)).unwrap();
        }
    };
    feed(&mut f, "ac_ess_norm", "Ac1", "AcEss");
    feed(&mut f, "ac_ess_shed_c", "AcEss", "AcEssShed");
    feed(&mut f, "ac_gnd_flt_c", "Ac2", "AcGndFltSvc");
    feed(&mut f, "ac_eha_c", "Ac3", "AcEha");
    feed(&mut f, "ac_emer_c", "AcEss", "AcEmer");
    for g in 1..=4 {
        feed(&mut f, &format!("galley{g}_c"), &format!("Ac{g}"), &format!("Galley{g}"));
    }
    for (id, from, to) in [("bcru1", "Ac2", "Dc1"), ("bcru2", "Ac3", "Dc2"), ("bcru_ess", "AcEss", "DcEss")] {
        writeln!(f, "[[electrical.tru]]\nid = \"{id}\"\ninput = \"{}\"\noutput = \"{}\"\nvoltage = \"29 V\"\nrating = \"8.4 kW\"\nregulated = true\n", bus_id(from), bus_id(to)).unwrap();
    }
    if buses.contains_key("DcApu") {
        writeln!(f, "[[electrical.tru]]\nid = \"apu_tr\"\ninput = \"ac4\"\noutput = \"dc_apu\"\nvoltage = \"29 V\"\nrating = \"8.4 kW\"\n").unwrap();
    }
    feed(&mut f, "dc_ess_shed_c", "DcEss", "DcEssShed");
    feed(&mut f, "dc_gnd_flt_c", "Dc1", "DcGndFltSvc");
    feed(&mut f, "dc_eha_c", "Dc2", "DcEha");
    feed(&mut f, "ess_bat_c", "DcBat", "DcEss");
    for (id, bus) in [("bat1", "DcHot1"), ("bat2", "DcHot2"), ("ess_bat", "DcBat"), ("apu_bat", "DcApu")] {
        if buses.contains_key(bus) {
            writeln!(f, "[[electrical.battery]]\nid = \"{id}\"\nbus = \"{}\"\nvoltage = \"24 V\"\ncapacity = \"50 Ah\"\n", bus_id(bus)).unwrap();
        }
    }
    // DC 1 and DC 2 take their batteries only while their own BCRU runs, to charge them; on batteries alone only the hot buses and DC ESS live.
    for (id, a, b, tr) in [("dc_bat_c", "dc1", "dc_bat", "bcru1"), ("dc_hot1_c", "dc_hot1", "dc1", "bcru1"), ("dc_hot2_c", "dc_hot2", "dc2", "bcru2")] {
        writeln!(f, "[[electrical.contactor]]
id = \"{id}\"
between = [\"{a}\", \"{b}\"]
close_when = \"{tr}.active\"
").unwrap();
    }
    let mut covered = BTreeSet::new();
    if let Some(c) = cat {
        let pids = param_ids(c);
        let mut groups: Vec<(String, Vec<&Breaker>)> = Vec::new();
        for b in &c.breakers {
            let base = base_load(&b.id).to_owned();
            match groups.iter_mut().find(|g| g.0 == base) {
                Some(g) => g.1.push(b),
                None => groups.push((base, vec![b])),
            }
        }
        let mut seen = BTreeSet::new();
        for (base, bs) in &groups {
            let lid = ident(base);
            if !seen.insert(lid.clone()) {
                continue;
            }
            let load = d.loads.iter().find(|l| l.id == *base || bs.iter().any(|b| b.id == l.id));
            if let Some(l) = load {
                covered.insert(l.id.clone());
            }
            let mut bs = bs.clone();
            bs.sort_by_key(|b| if b.id.ends_with("-3rd-bkr") { 2 } else if b.id.ends_with("-2nd-bkr") { 1 } else { 0 });
            let lb = format!("{lid}_bus");
            let ac = load.map_or_else(|| is_ac(&bs[0].bus), |l| is_ac(&l.bus));
            let v = if ac { 115.0 } else { 28.0 };
            let name = load.map(|l| l.name.clone()).unwrap_or_else(|| one_line(&bs[0].name));
            writeln!(f, "# {name} (ATA {})\n[[electrical.bus]]\nid = \"{lb}\"\nnominal = \"{}\"\n", bs[0].ata, if ac { "115 V" } else { "28 V" }).unwrap();
            // A unit fed from two buses of its kind takes each through its own diode, so neither feeds the other.
            let mut primary = bs.iter().filter(|b| is_ac(&b.bus) == ac).count() > 1;
            for (k, b) in bs.iter().enumerate() {
                let to = if !primary && is_ac(&b.bus) == ac {
                    primary = true;
                    lb.clone()
                } else {
                    let to = format!("{lb}_f{}", k + 1);
                    let same = is_ac(&b.bus) == ac;
                    writeln!(f, "{}[[electrical.bus]]\nid = \"{to}\"\nnominal = \"{}\"\n", if same { "" } else { "# Feed of the other kind, powering the unit's own converter\n" }, if is_ac(&b.bus) { "115 V" } else { "28 V" }).unwrap();
                    if same {
                        writeln!(f, "[[electrical.diode]]\nid = \"{to}_d\"\nbetween = [\"{to}\", \"{lb}\"]\n").unwrap();
                    }
                    to
                };
                let rating = if b.rating > 0.0 { b.rating } else { breaker_size(load.map_or(100.0, |l| l.watts), if ac { 3.0 * v } else { v }, 1.0) };
                let cb = format!("cb_{}", ident(&b.id));
                if b.sspc {
                    let par = |n: &str| pids.get(&(format!("24_elec.bkr.{}", b.id), n.to_owned())).map(|p| format!("{p}.value > 0.5"));
                    let closed = par("fails_to_trip").map(|w| format!("failed_closed_when = \"{w}\"\n")).unwrap_or_default();
                    let open = par("nuisance_trip").map(|w| format!("failed_open_when = \"{w}\"\n")).unwrap_or_default();
                    writeln!(f, "[[electrical.sspc]]\nid = \"{cb}\"\nbetween = [\"{}\", \"{to}\"]\nrating = \"{rating} A\"\n{closed}{open}", bus_id(&b.bus)).unwrap();
                } else {
                    writeln!(f, "[[electrical.breaker]]\nid = \"{cb}\"\nbetween = [\"{}\", \"{to}\"]\nrating = \"{rating} A\"\n", bus_id(&b.bus)).unwrap();
                }
            }
            let (watts, pf, kind) = match load {
                Some(l) => (l.watts, l.pf, if l.kind == "induction_motor" && !ac { "motor" } else { l.kind }),
                None => (bs[0].rating.max(1.0) / 1.25 * v * if ac { 3.0 } else { 1.0 } * 0.4, 1.0, "constant_power"),
            };
            let pf = if (pf - 1.0).abs() > 1e-9 { format!("power_factor = {pf:.2}\n") } else { String::new() };
            let rated = if kind == "induction_motor" { format!("rated_power = \"{watts:.0} W\"\n") } else { String::new() };
            writeln!(f, "[[electrical.load]]\nid = \"{lid}_load\"\nbus = \"{lb}\"\npower = \"{watts:.0} W\"\ntype = \"{kind}\"\n{pf}{rated}").unwrap();
            units.push((lid.clone(), lb.clone(), kind, bs[0].ata, name.clone()));
        }
    }
    for l in d.loads.iter().filter(|l| !covered.contains(&l.id)) {
        let lb = format!("{}_bus", super::msfs_fuel::id(&l.id));
        let ac = is_ac(&l.bus);
        let v = if ac { 115.0 } else { 28.0 };
        let rating = breaker_size(l.watts, if ac { 3.0 * v } else { v }, l.pf);
        writeln!(f, "# {} (ATA {})\n[[electrical.bus]]\nid = \"{lb}\"\nnominal = \"{}\"\n", l.name, l.ata, if ac { "115 V" } else { "28 V" }).unwrap();
        writeln!(f, "[[electrical.breaker]]\nid = \"cb_{}\"\nbetween = [\"{}\", \"{lb}\"]\nrating = \"{rating} A\"\n", super::msfs_fuel::id(&l.id), bus_id(&l.bus)).unwrap();
        for (k, s) in l.second.iter().enumerate().filter(|(_, s)| is_ac(s) == ac) {
            writeln!(f, "[[electrical.diode]]\nid = \"cb_{}_feed{}\"\nbetween = [\"{}\", \"{lb}\"]\n", super::msfs_fuel::id(&l.id), k + 2, bus_id(s)).unwrap();
        }
        let pf = if (l.pf - 1.0).abs() > 1e-9 { format!("power_factor = {:.2}\n", l.pf) } else { String::new() };
        let kind = if l.kind == "induction_motor" && !ac { "motor" } else { l.kind };
        let rated = if kind == "induction_motor" { format!("rated_power = \"{:.0} W\"\n", l.watts) } else { String::new() };
        writeln!(f, "[[electrical.load]]\nid = \"{}_load\"\nbus = \"{lb}\"\npower = \"{:.0} W\"\ntype = \"{kind}\"\n{pf}{rated}", super::msfs_fuel::id(&l.id), l.watts).unwrap();
        units.push((super::msfs_fuel::id(&l.id), lb.clone(), kind, l.ata, l.name.clone()));
    }
    // A 28 V bus stays powered down to 18 V, the DC drop-out; at the 80 % default a battery-fed bus dips out and back each step.
    (f.replace("nominal = \"28 V\"\n", "nominal = \"28 V\"\npowered_at = 0.643\n"), units)
}

/// `pub const NAME: f64 = expr;` lines whose expression is a product of numbers.
pub fn consts(text: &str) -> HashMap<String, f64> {
    let mut out = HashMap::new();
    for l in text.lines() {
        let Some(rest) = l.trim().strip_prefix("pub const ") else { continue };
        let Some((name, rest)) = rest.split_once(": f64 = ") else { continue };
        let expr = rest.trim_end_matches(';').trim();
        if let Some(v) = value(expr, &out.iter().map(|(k, v): (&String, &f64)| (k.clone(), v.to_string())).collect()) {
            out.insert(name.trim().to_owned(), v);
        }
    }
    out
}

/// ATA chapters whose constant-power consumers are computers or radios.
const AVIONICS_ATA: [u32; 12] = [22, 23, 27, 28, 31, 32, 34, 42, 44, 45, 46, 73];

/// Engines, FADECs, APU, hydraulics, gear, computers and their cockpit inputs for the plugin's A380.
pub fn systems(d: &Deep, head: &str, units: &[Unit], feeds: &[Option<String>]) -> Vec<(String, String)> {
    let e = |k: &str, dflt: f64| d.engine.get(k).copied().unwrap_or(dflt);
    let mut eng = head.to_owned();
    writeln!(eng, "# Four three-spool turbofans from the plugin's engine model (src/physics/engine/params.rs): BPR {}, OPR {}.\n", e("BYPASS_RATIO", 8.41), e("OPR_DESIGN", 38.68)).unwrap();
    for n in 1..=4 {
        let fuel = match feeds.get(n - 1).cloned().flatten() { Some(f) => format!("{f}.flow"), None => format!("fadec{n}.fuel_command") };
        writeln!(
            eng,
            "[[engine.gas_turbine]]\nid = \"eng{n}\"\nair = \"air\"\nfuel_flow = \"{fuel}\"\ndesign_thrust = \"{:.0} N\"\nairflow = \"{:.0} kg/s\"\nbypass_ratio = {}\nspools = 3\nfan_pr = {}\nipc_pr = {}\nhpc_pr = {}\nfan_efficiency = {}\nipc_efficiency = {}\nhpc_efficiency = {}\nhpt_efficiency = {}\nipt_efficiency = {}\nlpt_efficiency = {}\nlp_rpm = \"{:.0} rpm\"\nip_rpm = \"{:.0} rpm\"\nhp_rpm = \"{:.0} rpm\"\nlp_inertia = 900\nip_inertia = 40\nhp_inertia = 20\nstarter_torque = \"if(fadec{n}.starter, 1800 N*m, 0 N*m)\"\nofftake = \"vfg{n}.load / 0.9\"\nreverser = \"fadec{n}.reverser\"\n",
            e("STATIC_THRUST_N", 356_805.0),
            e("MDOT_TOTAL_DESIGN_KG_S", 1298.0),
            e("BYPASS_RATIO", 8.41),
            e("PR_FAN_DESIGN", 1.6),
            e("PR_IPC_DESIGN", 4.0),
            e("PR_HPC_DESIGN", 7.11),
            e("ETA_FAN_DESIGN", 0.9),
            e("ETA_IPC_DESIGN", 0.89),
            e("ETA_HPC_DESIGN", 0.86),
            e("ETA_HPT_DESIGN", 0.9),
            e("ETA_IPT_DESIGN", 0.91),
            e("ETA_LPT_DESIGN", 0.925),
            e("N1_DESIGN_RPM", 2900.0),
            e("N2_DESIGN_RPM", 8300.0),
            e("N3_DESIGN_RPM", 12200.0),
        )
        .unwrap();
        writeln!(eng, "[[fadec.controller]]\nid = \"fadec{n}\"\nengine = \"eng{n}\"\nlever = \"tla{n}.value\"\nmaster = \"eng{n}_master.on\"\nstart = \"eng_mode.value > 1.5\"\nidle = \"{}\"\nmax = \"100\"\nmax_reverse = \"{}\"\negt_limit = \"1173 K\"\nstart_egt_limit = \"1023 K\"\nfuel_on_at = 10\nstarter_cutout = 40\non_ground = \"air.on_ground\"\n", 20, if n == 2 || n == 3 { 70 } else { 0 }).unwrap();
        writeln!(eng, "[[input.number]]\nid = \"tla{n}\"\n\n[[cockpit.switch]]\nid = \"eng{n}_master\"\n").unwrap();
    }
    eng += "# ENG MODE selector: 0 CRANK, 1 NORM, 2 IGN START\n[[input.number]]\nid = \"eng_mode\"\ninitial = 1\n\n";
    eng += "[[apu.unit]]\nid = \"apu\"\nair = \"air\"\nmaster = \"apu_master.on\"\nstart = \"apu_start.on\"\nstart_time = \"60 s\"\ngenerator_load = \"apu_gen_a.load + apu_gen_b.load\"\ngenerator_rating = \"240 kW\"\nfuel_flow = \"0.07 kg/s\"\n\n";
    eng += "[[cockpit.switch]]\nid = \"apu_master\"\n\n[[cockpit.switch]]\nid = \"apu_start\"\n\n";
    eng += "[bindings]\n";
    for n in 1..=4 {
        writeln!(eng, "\"sim/cockpit2/engine/actuators/throttle_ratio[{}]\" = {{ to = \"tla{n}.value\" }}", n - 1).unwrap();
    }

    let mut h = head.to_owned();
    let none = Catalogue::default();
    let cat = d.catalogue.as_ref().unwrap_or(&none);
    let f = |id: &str, words: &[&str]| super::deep_parts::fails(cat, id, words);
    let or = |a: String, b: String| match (a.as_str(), b.as_str()) {
        ("false", _) => b,
        (_, "false") => a,
        _ => format!("{a} || {b}"),
    };
    let when = |k: &str, e: String| if e == "false" { String::new() } else { format!("{k} = \"{e}\"\n") };
    h += "# Two 5000 psi systems: green from engines 1 and 2, yellow from 3 and 4, two engine pumps per engine and two AC motor pumps per system; each unit's failures from the plugin's catalogue (29_hyd, 29_elec).\n\n";
    let duct = f("36_pneu.hydraulic_reservoir_pressurisation_duct", &["leak", "rupture"]);
    for c in ["green", "yellow"] {
        let res = format!("29_hyd.{c}_reservoir");
        writeln!(
            h,
            "[[hydraulic.circuit]]\nid = \"{c}\"\nnominal = \"5000 psi\"\nfluid = \"125 L\"\nlow_pressure = \"3000 psi\"\ncompliance = 4e-11\nleak = \"6.8 L/min\"\n{}{}{}",
            when("leak_when", f(&res, &["leak"])),
            when("reservoir_failed_when", or(f(&res, &["pressurisation"]), duct.clone())),
            when("filter_clog_when", f(&format!("29_hyd.{c}_return_filter"), &[])),
        )
        .unwrap();
    }
    for n in 1..=4 {
        let c = if n <= 2 { "green" } else { "yellow" };
        writeln!(h, "[[cockpit.switch]]\nid = \"eng{n}_fire_pb\"\n").unwrap();
        for x in ["a", "b"] {
            let u = format!("29_hyd.{c}_edp_{n}{x}");
            writeln!(
                h,
                "[[hydraulic.pump]]\nid = \"edp{n}{x}\"\ncircuit = \"{c}\"\npressure = \"5000 psi\"\nflow = \"if(eng{n}.n3 > 30, min(1, eng{n}.n3 / 60) * 150 L/min, 0 L/min)\"\nrunning_when = \"eng{n}.n3 > 30\"\n{}{}{}{}",
                when("worn_when", f(&u, &["displacement"])),
                when("seized_when", f(&u, &["seizure"])),
                when("check_open_when", f(&u, &["stuck open"])),
                when("check_shut_when", f(&u, &["stuck shut"])),
            )
            .unwrap();
            writeln!(h, "# Its fire shutoff valve, shut by the ENG {n} FIRE pushbutton.\n[[hydraulic.valve]]\nid = \"edp{n}{x}_fsov\"\ncircuit = \"{c}\"\nserves = [\"edp{n}{x}\"]\nopen_when = \"!eng{n}_fire_pb.on\"\n{}", when("stuck_when", f(&u, &["fire shutoff"]))).unwrap();
        }
    }
    for (c, x, bus, e) in [("green", "a", "ac1", "ga"), ("green", "b", "ac2", "gb"), ("yellow", "a", "ac3", "ya"), ("yellow", "b", "ac4", "yb")] {
        let u = format!("29_hyd.{c}_electric_pump_{x}");
        let motor = or(f(&format!("29_elec.hyd-epump-{e}"), &[]), f(&format!("29_elec.hyd-epump-{e}-coil"), &[]));
        writeln!(
            h,
            "[[cockpit.switch]]\nid = \"{c}_epump_{x}_sw\"\n\n[[hydraulic.pump]]\nid = \"{c}_epump_{x}\"\ncircuit = \"{c}\"\ntype = \"electric\"\npressure = \"5000 psi\"\nflow = \"16 L/min\"\nrunning_when = \"{c}_epump_{x}_sw.on && {bus}.powered\"\n{}{}{}",
            when("worn_when", f(&u, &["displacement"])),
            when("seized_when", f(&u, &["seizure"])),
            when("powered_when", if motor == "false" { "false".into() } else { super::deep_parts::not(&motor) }),
        )
        .unwrap();
    }
    for (id, c, vol) in [("brake_acc_g", "green", "12 L"), ("brake_acc_y", "yellow", "12 L"), ("cargo_door_acc", "yellow", "2 L")] {
        let lost = if id.starts_with("brake") { f(&format!("29_hyd.{c}_accumulator"), &[]) } else { "false".into() };
        writeln!(h, "[[hydraulic.accumulator]]\nid = \"{id}\"\ncircuit = \"{c}\"\nvolume = \"{vol}\"\nprecharge = \"2000 psi\"\ncheck_valve = true\n{}", when("precharge_lost_when", lost)).unwrap();
    }
    h += "[[logic.flag]]\nid = \"flying\"\nwhen = \"!air.on_ground\"\n\n";
    for (id, flow) in [("ailerons", 40), ("elevators", 30), ("rudder", 15), ("ths", 20)] {
        for (c, x) in [("green", "g"), ("yellow", "y")] {
            writeln!(h, "[[hydraulic.consumer]]\nid = \"{id}_{x}\"\ncircuit = \"{c}\"\nflow = \"{flow} L/min\"\nmoving_when = \"flying.on\"\n").unwrap();
        }
    }
    h += "[[input.number]]\nid = \"flap_lever\"\n\n[[logic.flag]]\nid = \"flaps_moving\"\nwhen = \"off_delay(rising(flap_lever.value > 0.5) || rising(flap_lever.value < 0.5), 40 s)\"\n\n";
    h += "[[hydraulic.consumer]]\nid = \"flaps_motor_g\"\ncircuit = \"green\"\nflow = \"60 L/min\"\nmoving_when = \"flaps_moving.on\"\n\n[[hydraulic.consumer]]\nid = \"slats_motor_g\"\ncircuit = \"green\"\nflow = \"40 L/min\"\nmoving_when = \"flaps_moving.on\"\n\n";
    h += "# Each system's supply lines to its gear, brakes, steering, reversers and cargo doors, each able to leak, the gear and cargo door lines behind the priority valve.\n";
    for c in ["green", "yellow"] {
        for line in ["gear", "brakes", "steering", "reversers", "cargo_doors"] {
            writeln!(h, "[[hydraulic.consumer]]\nid = \"{c}_line_{line}\"\ncircuit = \"{c}\"\nflow = \"0 L/min\"\n{}", when("leak_when", f(&format!("29_hyd.{c}_line_{line}"), &[]))).unwrap();
        }
        writeln!(h, "[[hydraulic.valve]]\nid = \"{c}_priority_valve\"\ncircuit = \"{c}\"\nserves = [\"{c}_line_gear\", \"{c}_line_cargo_doors\"]\nopen_when = \"{c}.pressure > 3000 psi\"\nopen_time = \"0.2 s\"\n{}", when("stuck_when", f(&format!("29_hyd.{c}_priority_valve"), &[]))).unwrap();
    }
    h += "# Five legs; the body and wing gear on green, the nose on yellow.\n";
    for (id, c, t) in [("nose_gear", "yellow", 10), ("wing_gear_l", "green", 14), ("wing_gear_r", "green", 14), ("body_gear_l", "green", 14), ("body_gear_r", "green", 14)] {
        writeln!(h, "[[gear.leg]]\nid = \"{id}\"\ncircuit = \"{c}\"\ndown_when = \"gear_lever.on\"\ntime = \"{t} s\"\n").unwrap();
    }

    let mut c = head.to_owned();
    c += "# Every constant-power avionics consumer in the catalogue, as a computer on its own breaker bus.\n\n";
    let mut n = 0;
    for (lid, lb, kind, ata, name) in units {
        if !lid.starts_with("cpiom_") && (*kind != "constant_power" || !AVIONICS_ATA.contains(ata)) || ["pump", "valve", "heater", "fan", "light"].iter().any(|w| lid.contains(w)) {
            continue;
        }
        let flight = ["prim", "sec", "fcdc"].iter().any(|k| lid.starts_with(k));
        writeln!(c, "# {} (ATA {ata})\n[[computer.unit]]\nid = \"{lid}\"\npower = [\"{lb}\"]\nboot_time = \"{}\"\n{}", one_line(name), if flight { "15 s" } else { "5 s" }, if flight { "channels = 2\n" } else { "" }).unwrap();
        n += 1;
    }
    let _ = n;
    vec![("systems/engines.toml".into(), eng), ("systems/hydraulic.toml".into(), h), ("systems/computers.toml".into(), c)]
}

pub fn report(d: &Deep) -> String {
    let mut r = format!("\n## In the plugin's deep systems ({})\n\n", d.root);
    let total_f: usize = d.areas.iter().map(|a| a.failures.len()).sum();
    let total_l: usize = d.areas.iter().map(|a| a.lines).sum();
    writeln!(r, "{} areas, {} lines of source, {} failure types listed in their FAILURES.md files. See `DEEP.md` for every one.\n", d.areas.len(), total_l, total_f).unwrap();
    for a in &d.areas {
        writeln!(r, "- {}: {} files, {} lines, {} failure types", a.name.replace('_', " "), a.files, a.lines, a.failures.len()).unwrap();
    }
    if !d.loads.is_empty() {
        let mut by_bus: BTreeMap<&str, usize> = BTreeMap::new();
        for l in &d.loads {
            *by_bus.entry(&l.bus).or_default() += 1;
        }
        let watts: f64 = d.loads.iter().map(|l| l.watts).sum();
        writeln!(r, "\nElectrical loads parsed from `deep/electrical/loads.rs`: {} consumers, {:.0} kW in all, each behind its own breaker{}; on {}.", d.loads.len(), watts / 1e3, if d.estimated > 0 { format!(", {} with power assumed", d.estimated) } else { String::new() }, by_bus.iter().map(|(b, n)| format!("{b} {n}")).collect::<Vec<_>>().join(", ")).unwrap();
    }
    if let Some(c) = &d.catalogue {
        let sspc = c.breakers.iter().filter(|b| b.sspc).count();
        let params: usize = c.components.iter().map(|x| x.params.len()).sum();
        let linked: usize = c.alerts.iter().filter(|a| !a.3.is_empty()).count();
        writeln!(r, "\nFrom the plugin's catalogue, all imported:\n").unwrap();
        writeln!(r, "- {} failures across {} ATA chapters, one input each in `systems/failures.toml` (`f<id>`, 0 healthy)", c.failures.len(), c.chapters.len()).unwrap();
        writeln!(r, "- {} components with {} health parameters, one input each in `systems/components.toml` at its healthy value", c.components.len(), params).unwrap();
        writeln!(r, "- {} breakers ({} SSPCs, {} thermal) on the network in `systems/electrical.toml`, at their own ratings and buses; SSPC fail-to-trip and nuisance-trip parameters wired", c.breakers.len(), sspc, c.breakers.len() - sspc).unwrap();
        writeln!(r, "- {} ECAM alerts as flags in `systems/ecam.toml`, {} raised by their linked failures", c.alerts.len(), linked).unwrap();
        writeln!(r, "- {} MEL entries, listed in `DEEP.md`", c.mel.len()).unwrap();
    }
    r
}

pub fn appendix(d: &Deep) -> String {
    let mut r = String::from("# Deep systems found in the plugin\n\n");
    for a in &d.areas {
        writeln!(r, "## {} ({} lines)\n", a.name.replace('_', " "), a.lines).unwrap();
        if a.failures.is_empty() {
            r += "No FAILURES.md table.\n\n";
            continue;
        }
        r += "| ATA | Failure |\n|---|---|\n";
        for (ata, n) in &a.failures {
            writeln!(r, "| {ata} | {n} |").unwrap();
        }
        r += "\n";
    }
    if !d.loads.is_empty() {
        r += "## Electrical loads\n\n| Id | Name | ATA | Bus | Power | Type |\n|---|---|---|---|---|---|\n";
        for l in &d.loads {
            writeln!(r, "| {} | {} | {} | {}{} | {:.0} W | {} |", l.id, l.name, l.ata, l.bus, if l.second.is_empty() { String::new() } else { format!(" + {}", l.second.join(", ")) }, l.watts, l.kind).unwrap();
        }
    }
    if let Some(c) = &d.catalogue {
        let cell = |s: &str| one_line(s).replace('|', "/");
        r += "\n## Catalogue failures\n\n";
        for (n, ch) in &c.chapters {
            let fs: Vec<_> = c.failures.iter().filter(|f| f.2 == *n).collect();
            writeln!(r, "### {} ({} failures)\n\n| Input | Failure | Component |\n|---|---|---|", cell(ch), fs.len()).unwrap();
            for (id, name, _, comp) in fs {
                writeln!(r, "| f{id} | {} | {} |", cell(name), cell(comp)).unwrap();
            }
            r += "\n";
        }
        r += "## Catalogue components\n\n| Component | ATA | Failures | Parameters |\n|---|---|---|---|\n";
        for x in &c.components {
            writeln!(r, "| {} | {} | {} | {} |", cell(&x.name), x.ata, x.failures.iter().map(|f| format!("f{f}")).collect::<Vec<_>>().join(" "), x.params.iter().map(|p| p.0.as_str()).collect::<Vec<_>>().join(", ")).unwrap();
        }
        r += "\n## Breakers\n\n| Id | Name | Bus | Rating | Kind | ATA |\n|---|---|---|---|---|---|\n";
        for b in &c.breakers {
            writeln!(r, "| {} | {} | {} | {} A | {} | {} |", b.id, cell(&b.name), b.bus, b.rating, if b.sspc { "SSPC" } else { "thermal" }, b.ata).unwrap();
        }
        r += "\n## ECAM alerts\n\n| Flag | Title | Level | Failures |\n|---|---|---|---|\n";
        for (key, title, level, fs) in &c.alerts {
            writeln!(r, "| ecam_{} | {} | {level} | {} |", ident(key), cell(title), fs.len()).unwrap();
        }
        r += "\n## MEL\n\n| Reference | Failures |\n|---|---|\n";
        for (m, fs) in &c.mel {
            writeln!(r, "| {m} | {} |", fs.iter().map(|f| format!("f{f}")).collect::<Vec<_>>().join(" ")).unwrap();
        }
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_loops_unroll_into_loads() {
        let src = r#"
fn ata21(net: &mut Network, cat: &mut Catalog) {
    const FANS: [(&str, &str, BusId); 2] = [("cab-fan-1", "CAB FAN 1", BusId::Ac1), ("cab-fan-2", "CAB FAN 2", BusId::Ac2)];
    for (id, name, bus) in FANS {
        let spec = motor_spec(id, name, 21, bus, 500.0, 0.85, 3.0, 1.0, "basis");
        add(net, cat, LoadCategory::Other, spec, a);
    }
    let spec = avionics_spec("prim-1", "PRIM 1", 27, BusId::DcEss, 120.0, "basis (with, commas)");
    add_dual(net, cat, LoadCategory::Other, spec, a, BusId::Dc1);
}
"#;
        let (l, est) = loads(src);
        assert_eq!(est, 0);
        assert_eq!(l.len(), 3, "{l:?}");
        assert_eq!((l[0].id.as_str(), l[0].bus.as_str(), l[0].watts, l[0].pf), ("cab-fan-1", "Ac1", 500.0, 0.85));
        assert_eq!((l[2].id.as_str(), l[2].bus.as_str(), l[2].second.clone()), ("prim-1", "DcEss", vec!["Dc1".to_owned()]));
    }

    #[test]
    fn a_feed_of_the_other_kind_gets_no_diode() {
        let b = |id: &str, bus: &str| Breaker { id: id.into(), name: id.into(), bus: bus.into(), rating: 3.0, sspc: true, consumer: String::new(), ata: 34 };
        let cat = Catalogue { breakers: vec![b("adirs-1-2nd-bkr", "DcEss"), b("adirs-1-normal-bkr", "AcEss"), b("prim-1-normal-bkr", "DcEss"), b("prim-1-2nd-bkr", "Dc1")], ..Default::default() };
        let d = Deep { catalogue: Some(cat), ..Default::default() };
        let (el, _) = electrical(&d, "", 4);
        assert!(el.contains("between = [\"ac_ess\", \"adirs_1_bus\"]"), "{el}");
        assert!(!el.contains("id = \"adirs_1_bus_f2_d\""), "{el}");
        assert!(el.contains("between = [\"prim_1_bus_f2\", \"prim_1_bus\"]"), "{el}");
    }

    #[test]
    fn failure_tables_read_their_ata_rows() {
        let md = "| ATA | name | x |\n|---|---|---|\n| 29 | EDP displacement loss | a |\n| 29 | Check valve stuck open | b |\n";
        assert_eq!(failures(md).len(), 2);
    }
}

#[cfg(test)]
mod probe {
    #[test]
    #[ignore]
    fn real_loads() {
        let t = std::fs::read_to_string("E:/fbw-int/plugin/src/deep/electrical/loads.rs").unwrap();
        let (l, e) = super::loads(&t);
        println!("loads {} estimated {}", l.len(), e);
        for x in l.iter().take(5) {
            println!("{x:?}");
        }
    }
}
