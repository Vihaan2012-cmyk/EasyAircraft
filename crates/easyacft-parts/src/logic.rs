//! Cockpit switches and logic flags.

use crate::catalog::out;
use crate::{expr_of, Domain, Expr};
use easyacft_lang::Checked;

pub fn init(c: &Checked, slots: &mut [f64]) {
    for p in c.parts.iter().filter(|p| p.domain == "cockpit" && p.kind == "switch") {
        slots[out(p, "on")] = p.flag("initial") as u8 as f64;
    }
}

pub struct Logic {
    flags: Vec<(usize, Expr)>,
}

impl Logic {
    pub fn new(c: &Checked) -> Self {
        let flags = c.parts.iter().filter(|p| p.domain == "logic" && p.kind == "flag").filter_map(|p| Some((out(p, "on"), expr_of(p, "when")?))).collect();
        Self { flags }
    }
}

impl Domain for Logic {
    fn name(&self) -> &'static str {
        "logic"
    }
    fn step(&mut self, slots: &mut [f64], dt: f64) {
        for (slot, e) in &mut self.flags {
            let v = e.eval(slots, dt);
            slots[*slot] = (v != 0.0) as u8 as f64;
        }
    }
    fn state(&self) -> Vec<f64> {
        self.flags.iter().flat_map(|(_, e)| e.state.clone()).collect()
    }
    fn restore(&mut self, s: &[f64]) {
        let mut at = 0;
        for (_, e) in &mut self.flags {
            let n = e.state.len();
            if at + n <= s.len() {
                e.state.copy_from_slice(&s[at..at + n]);
            }
            at += n;
        }
    }
}
