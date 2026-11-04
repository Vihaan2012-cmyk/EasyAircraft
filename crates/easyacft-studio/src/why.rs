//! The chain of causes behind a part's state.

use crate::diagram::{edge_ends, role, state, Role};
use easyacft_lang::Checked;

pub struct Line {
    pub depth: usize,
    pub text: String,
    pub part: Option<usize>,
}

pub fn names_in(src: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    for ch in src.chars().chain(std::iter::once(' ')) {
        if ch == '\'' {
            quoted = !quoted;
            cur.clear();
            continue;
        }
        if !quoted && (ch.is_alphanumeric() || ch == '_' || ch == '.') {
            cur.push(ch);
        } else {
            if cur.contains('.') && cur.starts_with(|c: char| c.is_alphabetic() || c == '_') && !out.contains(&cur) {
                out.push(cur.clone());
            }
            cur.clear();
        }
    }
    out
}

fn bad(c: &Checked, i: usize, get: &dyn Fn(&str) -> Option<f64>) -> bool {
    let p = &c.parts[i];
    if get(&format!("{}.failed", p.id)).is_some_and(|v| v != 0.0) {
        return true;
    }
    if role(p).is_some() || edge_ends(p).is_some() {
        return !state(p, get).0;
    }
    ["powered", "running", "available", "valid", "on", "closed", "open", "active", "lit"].iter().any(|o| get(&format!("{}.{o}", p.id)) == Some(0.0))
}

/// Explains part `start`: what it reads, what feeds it, and why those are off, a few levels deep.
pub fn explain(c: &Checked, start: usize, get: &dyn Fn(&str) -> Option<f64>, fmt: &dyn Fn(&str, f64) -> String) -> Vec<Line> {
    let mut out = Vec::new();
    let mut seen = vec![start];
    walk(c, start, 0, get, fmt, &mut seen, &mut out);
    out
}

fn walk(c: &Checked, i: usize, depth: usize, get: &dyn Fn(&str) -> Option<f64>, fmt: &dyn Fn(&str, f64) -> String, seen: &mut Vec<usize>, out: &mut Vec<Line>) {
    if depth > 4 || out.len() > 60 {
        return;
    }
    let p = &c.parts[i];
    if get(&format!("{}.failed", p.id)).is_some_and(|v| v != 0.0) {
        out.push(Line { depth, text: format!("{} has failed", p.id), part: Some(i) });
        return;
    }
    let mut next = Vec::new();
    if role(p).is_some_and(|r| r.0 == Role::Hub) {
        for (k, q) in c.parts.iter().enumerate() {
            let feeds = q.kind != "breaker" && edge_ends(q).is_some_and(|(a, b)| a == i || b == i) || role(q).is_some_and(|(r, f)| r == Role::Source && q.part(f) == Some(i));
            if feeds {
                let (live, failed, text) = state(q, get);
                let how = if failed { "failed".to_owned() } else if live { format!("supplying {text}") } else { format!("off {text}") };
                out.push(Line { depth, text: format!("{}: {how}", q.id), part: Some(k) });
                if !live && !seen.contains(&k) {
                    seen.push(k);
                    next.push((k, depth + 1));
                }
            }
        }
    }
    for (key, src) in &p.exprs {
        out.push(Line { depth, text: format!("{key} = {src}"), part: None });
        for name in names_in(src) {
            let Some(v) = get(&name) else { continue };
            let owner = name.rsplit_once('.').and_then(|(id, _)| c.find(id));
            out.push(Line { depth: depth + 1, text: format!("{name} = {}", fmt(&name, v)), part: owner });
            if let Some(k) = owner {
                if !seen.contains(&k) && bad(c, k, get) {
                    seen.push(k);
                    next.push((k, depth + 2));
                }
            }
        }
    }
    for (k, d) in next {
        walk(c, k, d, get, fmt, seen, out);
    }
}
