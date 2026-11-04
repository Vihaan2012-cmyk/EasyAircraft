//! The in-sim systems window's content: system tabs, one line per part, fail and repair.

use easyacft_lang::Type;
use easyacft_parts::{catalog, Aircraft};

pub const TABS: &[&str] = &["electrical", "hydraulic", "fuel", "pneumatic", "engine", "computer", "all"];
pub const LINE: i32 = 16;
pub const TAB_W: i32 = 84;
pub const HEAD: i32 = 46;

pub struct Row {
    pub part: usize,
    pub text: String,
    pub failed: Option<bool>,
}

#[derive(Default)]
pub struct Panel {
    pub tab: usize,
    pub scroll: usize,
}

fn value(t: Type, v: f64) -> String {
    use easyacft_core::Dim;
    match t {
        Type::Bool => String::new(),
        Type::Num(d) if d == Dim::VOLTAGE => format!("{v:.1} V"),
        Type::Num(d) if d == Dim::CURRENT => format!("{v:.0} A"),
        Type::Num(d) if d == Dim::POWER => format!("{:.1} kW", v / 1e3),
        Type::Num(d) if d == Dim::PRESSURE => format!("{:.0} psi", v / 6894.757),
        Type::Num(d) if d == Dim::TEMPERATURE => format!("{:.0} C", v - 273.15),
        Type::Num(d) if d == Dim::MASS => format!("{v:.0} kg"),
        Type::Num(d) if d == Dim::MASS_FLOW => format!("{v:.2} kg/s"),
        Type::Num(d) if d == Dim::FORCE => format!("{:.1} kN", v / 1e3),
        _ => format!("{v:.2}"),
    }
}

/// One line per part of the open tab: id, its true flags and first few values.
pub fn rows(a: &Aircraft, tab: usize) -> Vec<Row> {
    let want = TABS.get(tab).copied().unwrap_or("all");
    let mut out = Vec::new();
    for (i, p) in a.checked.parts.iter().enumerate() {
        let shown = match want {
            "all" => true,
            "engine" => matches!(p.domain, "engine" | "fadec" | "apu"),
            "computer" => matches!(p.domain, "computer" | "data" | "sensor"),
            d => p.domain == d,
        };
        if !shown {
            continue;
        }
        let Some(spec) = catalog::find(p.domain, p.kind) else { continue };
        let mut bits = Vec::new();
        let mut failed = None;
        for (n, t) in spec.outputs {
            let Some(v) = a.get(&format!("{}.{n}", p.id)) else { continue };
            if *n == "failed" {
                failed = Some(v != 0.0);
                continue;
            }
            match t {
                Type::Bool if v != 0.0 => bits.push((*n).to_owned()),
                Type::Bool => {}
                _ if bits.len() < 4 => bits.push(value(*t, v)),
                _ => {}
            }
        }
        out.push(Row { part: i, text: format!("{:<14} {:<12} {}", p.id, p.kind, bits.join("  ")), failed });
    }
    out
}

pub enum Click {
    Tab(usize),
    Toggle(usize),
    None,
}

impl Panel {
    /// What a click at (x, y) from the window's top-left hits.
    pub fn click(&self, rows: &[Row], x: i32, y: i32, width: i32) -> Click {
        if y < 24 {
            let k = (x - 8) / TAB_W;
            return if (0..TABS.len() as i32).contains(&k) { Click::Tab(k as usize) } else { Click::None };
        }
        if y < HEAD || x < width - 74 {
            return Click::None;
        }
        let k = (y - HEAD) / LINE + self.scroll as i32;
        match rows.get(k as usize) {
            Some(r) if r.failed.is_some() => Click::Toggle(r.part),
            _ => Click::None,
        }
    }

    pub fn wheel(&mut self, clicks: i32, rows: usize, visible: usize) {
        let max = rows.saturating_sub(visible);
        self.scroll = (self.scroll as i32 - clicks).clamp(0, max as i32) as usize;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn twin_jet_rows_and_clicks() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/twin-jet");
        let (mut a, _) = Aircraft::load(&dir).map_err(|r| r.render()).unwrap();
        a.set("bat_sw.on", 1.0);
        for _ in 0..40 {
            a.step(0.05);
        }
        let rows = rows(&a, 0);
        let dc1 = rows.iter().position(|r| r.text.starts_with("dc1 ")).expect("dc1 listed");
        assert!(rows[dc1].text.contains("powered"), "{}", rows[dc1].text);
        let p = Panel::default();
        let y = HEAD + dc1 as i32 * LINE + 2;
        assert!(matches!(p.click(&rows, 590, y, 600), Click::Toggle(i) if a.checked.parts[i].id == "dc1"));
        assert!(matches!(p.click(&rows, 100, y, 600), Click::None));
        assert!(matches!(p.click(&rows, 8 + TAB_W + 3, 10, 600), Click::Tab(1)));
    }
}
