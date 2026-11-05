//! The system diagram as an SVG file, drawn the same way as on screen.

use crate::diagram::{self, Role};
use easyacft_lang::Checked;
use std::collections::HashMap;
use std::fmt::Write;

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

pub fn render(c: &Checked, domain: &str, pinned: &HashMap<String, (f32, f32)>, get: impl Fn(&str) -> Option<f64>) -> String {
    let d = diagram::build(c, domain, pinned, true);
    let (ink, faint) = ("#111", "#999");
    let (mut w, mut h) = (400.0f32, 200.0f32);
    for it in &d.items {
        w = w.max(it.pos.0 + it.size.0 / 2.0 + 40.0);
        h = h.max(it.pos.1 + it.size.1 / 2.0 + 40.0);
    }
    let mut o = String::new();
    let _ = writeln!(o, r##"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" font-family="Segoe UI, Arial, sans-serif" font-size="11"><rect width="100%" height="100%" fill="#fff"/>"##);
    let st = |live: bool, failed: bool, fill: &str| {
        let colour = if live || failed { ink } else { faint };
        let dash = if failed { r#" stroke-dasharray="6 4""# } else { "" };
        format!(r#"stroke="{colour}" stroke-width="{}" fill="{fill}"{dash}"#, if live { 2.6 } else { 1.0 })
    };
    let pts = |p: &[(f32, f32)]| p.iter().map(|(x, y)| format!("{x},{y}")).collect::<Vec<_>>().join(" ");
    for (part, l) in &d.links {
        let (live, failed, _) = diagram::state(&c.parts[*part], &get);
        let _ = writeln!(o, r#"<polyline points="{}" {}/>"#, pts(l), st(live, failed, "none"));
    }
    for e in &d.edges {
        let p = &c.parts[e.part];
        let (live, failed, text) = diagram::state(p, &get);
        let _ = writeln!(o, r#"<polyline points="{}" {}/>"#, pts(&e.path), st(live, failed, "none"));
        let (cx, cy) = e.mark;
        let _ = writeln!(o, r##"<rect x="{}" y="{}" width="14" height="14" rx="2" {}/>"##, cx - 7.0, cy - 7.0, st(live, failed, "#fff"));
        if live {
            let _ = writeln!(o, r#"<line x1="{}" y1="{cy}" x2="{}" y2="{cy}" {}/>"#, cx - 5.0, cx + 5.0, st(true, false, "none"));
        }
        let (tx, ty, anchor) = if e.vertical { (cx + 11.0, cy + 4.0, "start") } else { (cx, cy - 11.0, "middle") };
        let _ = writeln!(o, r##"<text x="{tx}" y="{ty}" font-size="10" text-anchor="{anchor}" fill="{faint}" stroke="#fff" stroke-width="3" paint-order="stroke">{} {}</text>"##, esc(&p.id), esc(&text));
    }
    for it in &d.items {
        let p = &c.parts[it.part];
        let (live, failed, text) = diagram::state(p, &get);
        let (x, y) = it.pos;
        let (iw, ih) = it.size;
        let s = st(live, failed, "#fff");
        let r = if it.role == Role::Source { ih / 2.0 } else { 3.0 };
        let _ = writeln!(o, r#"<rect x="{}" y="{}" width="{iw}" height="{ih}" rx="{r}" {s}/>"#, x - iw / 2.0, y - ih / 2.0);
        match it.role {
            Role::Hub => {
                let _ = writeln!(o, r#"<text x="{}" y="{}" font-size="12" fill="{ink}">{}</text>"#, x - iw / 2.0 + 8.0, y + 4.0, esc(&p.id));
                let _ = writeln!(o, r#"<text x="{}" y="{}" text-anchor="end" fill="{faint}">{}</text>"#, x + iw / 2.0 - 8.0, y + 4.0, esc(&text));
                if let Some(&(b, u)) = d.folded.get(&it.part) {
                    let t = if b == 0 { format!("{u} users") } else { format!("{b} breakers, {u} users") };
                    let _ = writeln!(o, r#"<text x="{x}" y="{}" font-size="10" text-anchor="middle" fill="{faint}">{t}</text>"#, it.bottom() + 12.0);
                }
            }
            Role::Source | Role::Sink => {
                let _ = writeln!(o, r#"<text x="{x}" y="{}" text-anchor="middle" fill="{ink}">{}</text>"#, y + 4.0, esc(&p.id));
                let ty = if it.role == Role::Source { it.bottom() + 13.0 } else { it.top() - 5.0 };
                if !text.is_empty() {
                    let _ = writeln!(o, r#"<text x="{x}" y="{ty}" font-size="10" text-anchor="middle" fill="{faint}">{}</text>"#, esc(&text));
                }
            }
        }
    }
    o.push_str("</svg>\n");
    o
}
