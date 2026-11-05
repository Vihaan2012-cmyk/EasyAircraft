//! Lists what an aircraft's own systems source (FlyByWire `wasm/systems`) builds; nothing is converted.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Looks up from `from` for the systems source folder.
fn systems_root(from: &Path) -> Option<PathBuf> {
    let mut at = Some(from);
    for _ in 0..12 {
        let d = at?;
        let w = d.join("wasm").join("systems");
        if w.is_dir() {
            return Some(w);
        }
        at = d.parent();
    }
    None
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            if p.file_name().is_some_and(|n| n == "test" || n == "tests") {
                continue;
            }
            rust_files(&p, out);
        } else if p.extension().is_some_and(|e| e == "rs") {
            out.push(p);
        }
    }
}

/// Every quoted id in calls like `Contactor::new(context, "990XU1")`.
fn quoted_after(text: &str, call: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(i) = rest.find(call) {
        rest = &rest[i + call.len()..];
        let window = &rest[..rest.len().min(80)];
        if let Some(q) = window.find('"') {
            if let Some(e) = window[q + 1..].find('"') {
                out.push(window[q + 1..q + 1 + e].to_owned());
            }
        }
    }
    out
}

fn count(text: &str, pat: &str) -> usize {
    text.matches(pat).count()
}

pub fn scan(from: &Path) -> Vec<String> {
    let Some(root) = systems_root(from) else { return Vec::new() };
    let Ok(rd) = std::fs::read_dir(&root) else { return Vec::new() };
    let mut crates: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.is_dir() && p.file_name().is_some_and(|n| n.to_string_lossy().ends_with("_systems"))).collect();
    crates.sort();
    let mut out = Vec::new();
    for c in crates {
        let src = c.join("src");
        let name = c.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let mut files = Vec::new();
        rust_files(&src, &mut files);
        if files.is_empty() {
            continue;
        }
        let mut modules: Vec<(String, usize)> = Vec::new();
        let mut all = String::new();
        for f in &files {
            let Ok(t) = std::fs::read_to_string(f) else { continue };
            let rel = f.strip_prefix(&src).unwrap_or(f);
            let top = rel.components().next().map(|c| c.as_os_str().to_string_lossy().trim_end_matches(".rs").to_owned()).unwrap_or_default();
            match modules.iter_mut().find(|m| m.0 == top) {
                Some(m) => m.1 += t.lines().count(),
                None => modules.push((top, t.lines().count())),
            }
            all.push_str(&t);
        }
        let total: usize = modules.iter().map(|m| m.1).sum();
        out.push(format!("`{name}`: {} source files, {total} lines", files.len()));
        let list: Vec<String> = modules.iter().filter(|m| m.0 != "lib").map(|m| format!("{} ({} lines)", m.0.replace('_', " "), m.1)).collect();
        out.push(format!("systems: {}", list.join(", ")));
        let mut buses: BTreeSet<String> = BTreeSet::new();
        let mut rest = all.as_str();
        while let Some(i) = rest.find("ElectricalBusType::") {
            rest = &rest[i + "ElectricalBusType::".len()..];
            // The variant, and its number when the code gives a literal one.
            let end = rest.find(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_')).unwrap_or(rest.len());
            let name = &rest[..end];
            let after = &rest[end..];
            let number = after.strip_prefix('(').and_then(|a| a.split_once(')')).map(|(n, _)| n).filter(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()));
            if name.chars().next().is_some_and(|c| c.is_ascii_uppercase()) && name != "Sub" && !name.ends_with("Named") {
                buses.insert(match number {
                    Some(n) => format!("{name} {n}"),
                    None if after.starts_with('(') => continue,
                    None => name.to_owned(),
                });
            }
        }
        if !buses.is_empty() {
            out.push(format!("electrical bus types ({}): {}", buses.len(), buses.into_iter().collect::<Vec<_>>().join(", ")));
        }
        let contactors: BTreeSet<String> = quoted_after(&all, "Contactor::new(").into_iter().collect();
        if !contactors.is_empty() {
            out.push(format!("contactors named in the code ({}): {}", contactors.len(), contactors.into_iter().collect::<Vec<_>>().join(", ")));
        }
        let inventory = [("Battery::", "battery constructions"), ("TransformerRectifier::new(", "transformer-rectifiers"), ("StaticInverter::new(", "static inverters"), ("EmergencyGenerator::new(", "emergency generators"), ("ExternalPowerSource::new(", "external power sources"), ("EngineGenerator", "engine generator references"), ("ElectricPump", "electric pump references"), ("EngineDrivenPump", "engine-driven pump references"), ("Accumulator", "accumulator references")];
        let found: Vec<String> = inventory.iter().filter_map(|(pat, what)| {
            let n = count(&all, pat);
            (n > 0).then(|| format!("{n} {what}"))
        }).collect();
        if !found.is_empty() {
            out.push(found.join(", "));
        }
    }
    out
}
