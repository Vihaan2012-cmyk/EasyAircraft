//! Edits aircraft files in place, keeping their comments and layout.

use std::path::Path;
use toml_edit::{value, ArrayOfTables, DocumentMut, Item, Table, Value};

fn matches_id(t: &Table, id: &str) -> bool {
    let Some(raw) = t.get("id").and_then(Item::as_str) else { return false };
    if raw == id {
        return true;
    }
    let count = t.get("count").and_then(Item::as_integer).unwrap_or(1).max(1);
    raw.contains("{n}") && (1..=count).any(|n| raw.replace("{n}", &n.to_string()) == id)
}

fn parse_value(text: &str) -> Value {
    let t = text.trim();
    if let Ok(b) = t.parse::<bool>() {
        return Value::from(b);
    }
    if let Ok(i) = t.parse::<i64>() {
        return Value::from(i);
    }
    if let Ok(f) = t.parse::<f64>() {
        return Value::from(f);
    }
    if t.starts_with('[') {
        if let Ok(v) = t.parse::<Value>() {
            return v;
        }
    }
    Value::from(t.trim_matches('"'))
}

fn tables_mut(doc: &mut DocumentMut) -> Vec<&mut Table> {
    let mut out = Vec::new();
    for (_, item) in doc.as_table_mut().iter_mut() {
        if let Some(domain) = item.as_table_mut() {
            for (_, kind) in domain.iter_mut() {
                if let Some(arr) = kind.as_array_of_tables_mut() {
                    out.extend(arr.iter_mut());
                }
            }
        }
    }
    out
}

/// Sets `key` of part `id` in `file`; true if the part was found. Counted parts change for every copy.
pub fn set(file: &Path, id: &str, key: &str, text: &str) -> Result<bool, String> {
    set_as(file, id, key, text, false)
}

/// As `set`; `string` writes the text as a TOML string whatever it looks like.
pub fn set_as(file: &Path, id: &str, key: &str, text: &str, string: bool) -> Result<bool, String> {
    let src = std::fs::read_to_string(file).map_err(|e| e.to_string())?;
    let mut doc: DocumentMut = src.parse().map_err(|e: toml_edit::TomlError| e.to_string())?;
    let mut found = false;
    for t in tables_mut(&mut doc) {
        if matches_id(t, id) {
            if text.trim().is_empty() {
                t.remove(key);
            } else {
                t[key] = value(if string { Value::from(text.trim()) } else { parse_value(text) });
            }
            found = true;
            break;
        }
    }
    if found {
        std::fs::write(file, doc.to_string()).map_err(|e| e.to_string())?;
    }
    Ok(found)
}

/// Appends a new `[[domain.kind]]` part to `file`.
pub fn add(file: &Path, domain: &str, kind: &str, id: &str) -> Result<(), String> {
    let src = std::fs::read_to_string(file).unwrap_or_default();
    let mut doc: DocumentMut = src.parse().map_err(|e: toml_edit::TomlError| e.to_string())?;
    let d = doc.as_table_mut().entry(domain).or_insert(Item::Table(Table::new()));
    let Some(dt) = d.as_table_mut() else { return Err(format!("`{domain}` is not a section")) };
    dt.set_implicit(true);
    let k = dt.entry(kind).or_insert(Item::ArrayOfTables(ArrayOfTables::new()));
    let Some(arr) = k.as_array_of_tables_mut() else { return Err(format!("`{domain}.{kind}` is not a list of parts")) };
    let mut t = Table::new();
    t["id"] = value(id);
    arr.push(t);
    std::fs::write(file, doc.to_string()).map_err(|e| e.to_string())
}

/// Removes part `id` from `file`.
pub fn remove(file: &Path, id: &str) -> Result<bool, String> {
    let src = std::fs::read_to_string(file).map_err(|e| e.to_string())?;
    let mut doc: DocumentMut = src.parse().map_err(|e: toml_edit::TomlError| e.to_string())?;
    let mut found = false;
    for (_, item) in doc.as_table_mut().iter_mut() {
        if let Some(domain) = item.as_table_mut() {
            for (_, kind) in domain.iter_mut() {
                if let Some(arr) = kind.as_array_of_tables_mut() {
                    let at = arr.iter().position(|t| matches_id(t, id));
                    if let Some(i) = at {
                        arr.remove(i);
                        found = true;
                    }
                }
            }
        }
    }
    if found {
        std::fs::write(file, doc.to_string()).map_err(|e| e.to_string())?;
    }
    Ok(found)
}

/// Undo for every file Studio writes, and what changed since the folder was opened.
#[derive(Default)]
pub struct History {
    opened: std::collections::BTreeMap<std::path::PathBuf, String>,
    undo: Vec<Vec<(std::path::PathBuf, String)>>,
    pending: Vec<(std::path::PathBuf, String)>,
}

impl History {
    /// Call before writing `file`; edits until `commit` undo together.
    pub fn touch(&mut self, file: &Path) {
        let text = std::fs::read_to_string(file).unwrap_or_default();
        self.opened.entry(file.to_path_buf()).or_insert_with(|| text.clone());
        if !self.pending.iter().any(|(p, _)| p == file) {
            self.pending.push((file.to_path_buf(), text));
        }
    }

    pub fn commit(&mut self) {
        if !self.pending.is_empty() {
            self.undo.push(std::mem::take(&mut self.pending));
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn undo(&mut self) -> Result<(), String> {
        let Some(step) = self.undo.pop() else { return Ok(()) };
        for (p, t) in step {
            std::fs::write(&p, t).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    /// Files changed since opening, with lines added and removed.
    pub fn changes(&self) -> Vec<(String, usize, usize)> {
        self.opened
            .iter()
            .filter_map(|(p, old)| {
                let new = std::fs::read_to_string(p).unwrap_or_default();
                if &new == old {
                    return None;
                }
                let a: Vec<&str> = old.lines().collect();
                let b: Vec<&str> = new.lines().collect();
                let added = b.iter().filter(|l| !a.contains(l)).count();
                let removed = a.iter().filter(|l| !b.contains(l)).count();
                Some((p.display().to_string(), added, removed))
            })
            .collect()
    }
}

/// Adds a dataref binding to the aircraft's main file.
pub fn bind(file: &Path, dataref: &str, path: &str, write: bool, unit: &str) -> Result<(), String> {
    let src = std::fs::read_to_string(file).map_err(|e| e.to_string())?;
    let mut doc: DocumentMut = src.parse().map_err(|e: toml_edit::TomlError| e.to_string())?;
    let b = doc.as_table_mut().entry("bindings").or_insert(Item::Table(Table::new()));
    let Some(bt) = b.as_table_mut() else { return Err("`bindings` is not a section".into()) };
    if unit.is_empty() && !write {
        bt[dataref] = value(path);
    } else {
        let mut t = toml_edit::InlineTable::new();
        t.insert(if write { "to" } else { "from" }, path.into());
        if !unit.is_empty() {
            t.insert("unit", unit.into());
        }
        bt[dataref] = value(t);
    }
    std::fs::write(file, doc.to_string()).map_err(|e| e.to_string())
}

/// Copies a template aircraft folder to a new one.
pub fn copy_dir(from: &Path, to: &Path) -> Result<(), String> {
    if to.join("aircraft.toml").exists() {
        return Err(format!("{} already has an aircraft", to.display()));
    }
    std::fs::create_dir_all(to).map_err(|e| e.to_string())?;
    for e in std::fs::read_dir(from).map_err(|e| e.to_string())?.flatten() {
        let (src, dst) = (e.path(), to.join(e.file_name()));
        if src.is_dir() {
            copy_dir(&src, &dst)?;
        } else if src.file_name().is_some_and(|n| n != "studio-layout.toml" && n != "persistent.txt") {
            std::fs::copy(&src, &dst).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_keep_comments_and_reach_counted_parts() {
        let dir = std::env::temp_dir().join(format!("easyacft-edit-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("e.toml");
        std::fs::write(&f, "# main buses\n[[electrical.bus]]\nid = \"dc{n}\"\ncount = 2\nnominal = \"28 V\" # spec\n").unwrap();
        assert!(set(&f, "dc2", "nominal", "\"24 V\"").unwrap());
        assert!(set(&f, "dc1", "powered_at", "0.7").unwrap());
        add(&f, "electrical", "battery", "bat").unwrap();
        let t = std::fs::read_to_string(&f).unwrap();
        assert!(t.contains("# main buses"), "{t}");
        assert!(t.contains("nominal = \"24 V\""), "{t}");
        assert!(t.contains("powered_at = 0.7"), "{t}");
        assert!(t.contains("[[electrical.battery]]\nid = \"bat\""), "{t}");
        assert!(remove(&f, "bat").unwrap());
        assert!(!std::fs::read_to_string(&f).unwrap().contains("battery"));
        assert!(!set(&f, "nope", "x", "1").unwrap());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
