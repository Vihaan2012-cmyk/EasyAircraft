//! Cockpit controls, lights, logic flags, weight and balance.

use crate::catalog::out;
use crate::{expr_of, Domain, Expr};
use easyacft_lang::Checked;

pub fn init(c: &Checked, slots: &mut [f64]) {
    for p in &c.parts {
        match (p.domain, p.kind) {
            ("cockpit", "switch" | "guarded_switch") => slots[out(p, "on")] = p.flag("initial") as u8 as f64,
            ("cockpit", "pushbutton") => slots[out(p, "pressed")] = p.flag("initial") as u8 as f64,
            ("cockpit", "selector") => slots[out(p, "position")] = p.num("initial"),
            ("input", "number") => slots[out(p, "value")] = p.num("initial"),
            ("weight", "station") => {
                slots[out(p, "mass")] = p.num("mass");
                slots[out(p, "arm")] = p.num("arm");
            }
            _ => {}
        }
    }
}

struct Guarded {
    on: usize,
    guard: usize,
    last: f64,
}

struct Light {
    out: usize,
    when: Option<Expr>,
    test: Option<usize>,
    failed: usize,
    pressed: Option<usize>,
}

struct Balance {
    stations: Vec<(usize, usize, f64)>,
    mac_le: f64,
    mac: f64,
    slots: [usize; 4],
}

pub struct Logic {
    flags: Vec<(usize, Expr)>,
    guarded: Vec<Guarded>,
    lights: Vec<Light>,
    selectors: Vec<(usize, f64)>,
    balances: Vec<Balance>,
}

impl Logic {
    pub fn new(c: &Checked) -> Self {
        let mut l = Logic { flags: vec![], guarded: vec![], lights: vec![], selectors: vec![], balances: vec![] };
        let test_slot = |p: &easyacft_lang::CheckedPart| p.part("test").map(|i| c.parts[i].slot);
        for p in &c.parts {
            match (p.domain, p.kind) {
                ("logic", "flag") => {
                    if let Some(e) = expr_of(p, "when") {
                        l.flags.push((out(p, "on"), e));
                    }
                }
                ("cockpit", "guarded_switch") => l.guarded.push(Guarded { on: out(p, "on"), guard: out(p, "guard_open"), last: p.flag("initial") as u8 as f64 }),
                ("cockpit", "pushbutton") => {
                    let failed = out(p, "failed");
                    l.lights.push(Light { out: out(p, "fault_light"), when: expr_of(p, "fault_when"), test: test_slot(p), failed, pressed: None });
                    l.lights.push(Light { out: out(p, "on_light"), when: None, test: test_slot(p), failed, pressed: Some(out(p, "pressed")) });
                }
                ("cockpit", "annunciator") => l.lights.push(Light { out: out(p, "lit"), when: expr_of(p, "when"), test: test_slot(p), failed: out(p, "failed"), pressed: None }),
                ("cockpit", "selector") => l.selectors.push((out(p, "position"), p.num("positions").max(1.0))),
                ("weight", "balance") => {
                    let stations = p.parts("stations").into_iter().map(|i| {
                        let s = &c.parts[i];
                        let lateral = if s.kind == "station" { s.num("lateral") } else { 0.0 };
                        (c.slots.index[&format!("{}.mass", s.id)], c.slots.index[&format!("{}.arm", s.id)], lateral)
                    });
                    l.balances.push(Balance { stations: stations.collect(), mac_le: p.num("mac_leading_edge"), mac: p.num("mac"), slots: [out(p, "mass"), out(p, "cg"), out(p, "cg_lateral"), out(p, "cg_mac")] });
                }
                _ => {}
            }
        }
        l
    }
}

impl Domain for Logic {
    fn name(&self) -> &'static str {
        "logic"
    }

    fn step(&mut self, slots: &mut [f64], dt: f64) {
        for g in &mut self.guarded {
            if slots[g.on] != g.last && slots[g.guard] == 0.0 {
                slots[g.on] = g.last;
            }
            g.last = slots[g.on];
        }
        for (slot, n) in &self.selectors {
            slots[*slot] = slots[*slot].round().clamp(0.0, n - 1.0);
        }
        for (slot, e) in &mut self.flags {
            slots[*slot] = (e.eval(slots, dt) != 0.0) as u8 as f64;
        }
        for l in &mut self.lights {
            let test = l.test.is_some_and(|t| slots[t] != 0.0);
            let lit = match (&mut l.when, l.pressed) {
                (Some(e), _) => e.yes(slots, dt),
                (None, Some(p)) => slots[p] != 0.0,
                _ => false,
            };
            slots[l.out] = ((lit || test) && slots[l.failed] == 0.0) as u8 as f64;
        }
        for b in &self.balances {
            let (mut m, mut mx, mut my) = (0.0, 0.0, 0.0);
            for &(ms, arm, lat) in &b.stations {
                m += slots[ms];
                mx += slots[ms] * slots[arm];
                my += slots[ms] * lat;
            }
            let cg = if m > 0.0 { mx / m } else { 0.0 };
            slots[b.slots[0]] = m;
            slots[b.slots[1]] = cg;
            slots[b.slots[2]] = if m > 0.0 { my / m } else { 0.0 };
            slots[b.slots[3]] = if b.mac > 0.0 { (cg - b.mac_le) / b.mac * 100.0 } else { 0.0 };
        }
    }

    fn state(&self) -> Vec<f64> {
        let mut v: Vec<f64> = self.flags.iter().flat_map(|(_, e)| e.state.clone()).collect();
        v.extend(self.lights.iter().flat_map(|l| l.when.as_ref().map(|e| e.state.clone()).unwrap_or_default()));
        v.extend(self.guarded.iter().map(|g| g.last));
        v
    }

    fn restore(&mut self, s: &[f64]) {
        let mut it = s.iter().copied();
        for (_, e) in &mut self.flags {
            for x in e.state.iter_mut() {
                *x = it.next().unwrap_or(0.0);
            }
        }
        for l in &mut self.lights {
            if let Some(e) = &mut l.when {
                for x in e.state.iter_mut() {
                    *x = it.next().unwrap_or(0.0);
                }
            }
        }
        for g in &mut self.guarded {
            g.last = it.next().unwrap_or(g.last);
        }
    }
}
