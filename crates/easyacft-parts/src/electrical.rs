//! Electrical domain: buses, sources, switching, protection, loads and shedding.

use crate::util::Slots;
use crate::{expr_of, Domain, Expr};
use easyacft_core::Network;
use easyacft_lang::{Checked, CheckedPart};
use std::collections::HashMap;

const SWITCH_OHM: f64 = 1e-4;
const REGULATION: f64 = 0.02;
const TRIP_HEAT: f64 = 10.0;
const UNSHED_MARGIN: f64 = 0.9;
/// Below this share of charge a battery's open-circuit voltage falls away to nothing.
const KNEE: f64 = 0.1;
/// A dropped-out load comes back once its bus is this far above its minimum voltage for `RECOVER_S`.
const RECOVER_RATIO: f64 = 1.1;
const RECOVER_S: f64 = 0.5;
/// Constant-power loads are linearised at last step's voltage, never below this share of nominal: a
/// lagged constant-power load diverges once its conductance passes the source's. Loads drop out at their
/// minimum voltage above it, so the floor only holds through a transient.
const LINEARISE_FLOOR: f64 = 0.5;

/// Open-circuit voltage at a state of charge: a gentle slope over the useful range, then collapse.
fn ocv(nominal: f64, charge: f64) -> f64 {
    let at_knee = 0.88 + 0.17 * KNEE;
    nominal * if charge >= KNEE { 0.88 + 0.17 * charge } else { at_knee * charge.max(0.0) / KNEE }
}

struct Bus {
    node: usize,
    nominal: f64,
    powered_at: f64,
    s: Slots,
}

struct Battery {
    node: usize,
    nominal: f64,
    capacity: f64,
    g: f64,
    limit: f64,
    g_now: f64,
    charge: f64,
    s: Slots,
}

struct Source {
    node: usize,
    voltage: f64,
    frequency: f64,
    rating: f64,
    power_factor: f64,
    g: f64,
    when: Option<Expr>,
    ext: bool,
    s: Slots,
}

struct Switch {
    link: usize,
    when: Option<Expr>,
    rating: f64,
    heat: f64,
    tripped: bool,
    s: Slots,
}

struct Load {
    bus: usize,
    node: usize,
    power: f64,
    power_e: Option<Expr>,
    when: Option<Expr>,
    priority: f64,
    shed: bool,
    min_voltage: f64,
    dropped: bool,
    recover: f64,
    g: f64,
    s: Slots,
}

struct Converter {
    input: usize,
    in_node: usize,
    out_node: usize,
    voltage: f64,
    rating: f64,
    eff: f64,
    g: f64,
    when: Option<Expr>,
    drawn: f64,
    s: Slots,
}

pub struct Electrical {
    net: Network,
    ports: crate::ports::Ports,
    buses: Vec<Bus>,
    bus_of: HashMap<usize, usize>,
    batteries: Vec<Battery>,
    sources: Vec<Source>,
    switches: Vec<Switch>,
    loads: Vec<Load>,
    converters: Vec<Converter>,
    /// Loads that can be shed, highest priority (shed first) first.
    shed_order: Vec<usize>,
    scratch: Scratch,
}

/// Per-step working space, kept to avoid allocating every step.
#[derive(Default)]
struct Scratch {
    powered: Vec<bool>,
    wanted: Vec<bool>,
    islands: Vec<usize>,
    capacity: Vec<f64>,
    demand: Vec<f64>,
    has_capacity: Vec<bool>,
}

impl Electrical {
    pub fn new(c: &Checked) -> Option<Self> {
        let parts: Vec<(usize, &CheckedPart)> = c.parts.iter().enumerate().filter(|(_, p)| p.domain == "electrical").collect();
        if parts.is_empty() {
            return None;
        }
        let mut net = Network::new();
        let mut e = Electrical { net: Network::new(), ports: Default::default(), buses: vec![], bus_of: HashMap::new(), batteries: vec![], sources: vec![], switches: vec![], loads: vec![], converters: vec![], shed_order: vec![], scratch: Scratch::default() };
        for &(i, p) in parts.iter().filter(|(_, p)| p.kind == "bus") {
            let node = net.add_node(0.0);
            e.bus_of.insert(i, e.buses.len());
            e.buses.push(Bus { node, nominal: p.num("nominal"), powered_at: p.num("powered_at"), s: Slots::of(p, &["voltage", "powered", "failed"]) });
        }
        let node = |e: &Electrical, part: Option<usize>| part.and_then(|x| e.bus_of.get(&x)).map(|&b| e.buses[b].node);
        for &(_, p) in &parts {
            match p.kind {
                "battery" => {
                    let Some(n) = node(&e, p.part("bus")) else { continue };
                    e.batteries.push(Battery { node: n, nominal: p.num("voltage"), capacity: p.num("capacity").max(1.0), g: 1.0 / p.num("resistance").max(1e-6), limit: p.num("charge_limit").max(0.1), g_now: 1.0 / p.num("resistance").max(1e-6), charge: p.num("charge").clamp(0.0, 1.0), s: Slots::of(p, &["voltage", "current", "charge", "failed"]) });
                }
                "generator" | "ext_power" => {
                    let Some(n) = node(&e, p.part("bus")) else { continue };
                    let (v, r) = (p.num("voltage"), p.num("rating").max(1.0));
                    let ext = p.kind == "ext_power";
                    let names: &[&'static str] = if ext { &["voltage", "current", "load", "available", "failed"] } else { &["voltage", "frequency", "current", "load", "ready", "failed"] };
                    e.sources.push(Source { node: n, voltage: v, frequency: p.num("frequency"), rating: r, power_factor: p.num("power_factor").clamp(0.05, 1.0), g: r / (REGULATION * v * v).max(1e-9), when: expr_of(p, if ext { "connected_when" } else { "driven_when" }), ext, s: Slots::of(p, names) });
                }
                "contactor" | "breaker" => {
                    let ends = p.parts("between");
                    let (Some(a), Some(b)) = (node(&e, ends.first().copied()), node(&e, ends.get(1).copied())) else { continue };
                    let link = net.add_link(a, b, SWITCH_OHM, 0.0);
                    let breaker = p.kind == "breaker";
                    let names: &[&'static str] = if breaker { &["closed", "tripped", "pulled", "current", "failed"] } else { &["closed", "current", "failed"] };
                    e.switches.push(Switch { link, when: if breaker { None } else { expr_of(p, "close_when") }, rating: if breaker { p.num("rating") } else { 0.0 }, heat: 0.0, tripped: false, s: Slots::of(p, names) });
                }
                "load" => {
                    let Some(&b) = p.part("bus").and_then(|x| e.bus_of.get(&x)) else { continue };
                    let min = p.num("min_voltage");
                    let min_voltage = if min > 0.0 { min } else { 0.5 * e.buses[b].nominal };
                    e.loads.push(Load { bus: b, node: e.buses[b].node, power: 0.0, power_e: expr_of(p, "power"), when: expr_of(p, "on_when"), priority: p.num("shed_priority"), shed: false, min_voltage, dropped: false, recover: 0.0, g: 0.0, s: Slots::of(p, &["powered", "current", "power", "shed", "failed"]) });
                }
                "tru" | "inverter" => {
                    let (Some(&ib), Some(&ob)) = (p.part("input").and_then(|x| e.bus_of.get(&x)), p.part("output").and_then(|x| e.bus_of.get(&x))) else { continue };
                    let (v, r) = (p.num("voltage"), p.num("rating").max(1.0));
                    e.converters.push(Converter {
                        input: ib,
                                        in_node: e.buses[ib].node,
                        out_node: e.buses[ob].node,
                        voltage: v,
                        rating: r,
                        eff: p.num("efficiency").clamp(0.05, 1.0),
                        g: r / (REGULATION * v * v).max(1e-9),
                        when: expr_of(p, "on_when"),
                        drawn: 0.0,
                        s: Slots::of(p, &["current", "power", "active", "failed"]),
                    });
                }
                _ => {}
            }
        }
        e.ports = crate::ports::Ports::new(c, "electrical", &mut net, |i| e.bus_of.get(&i).map(|&b| e.buses[b].node));
        e.net = net;
        e.shed_order = (0..e.loads.len()).filter(|&k| e.loads[k].priority > 0.0).collect();
        e.shed_order.sort_by(|&a, &b| e.loads[b].priority.total_cmp(&e.loads[a].priority));
        Some(e)
    }

    fn bus_powered(&self, slots: &[f64], b: usize) -> bool {
        self.buses[b].s.on(slots, "powered")
    }
}

impl Domain for Electrical {
    fn name(&self) -> &'static str {
        "electrical"
    }

    fn step(&mut self, slots: &mut [f64], dt: f64) {
        self.net.begin_step();
        for sw in &mut self.switches {
            let failed = sw.s.on(slots, "failed");
            let closed = if sw.rating > 0.0 {
                !failed && !sw.tripped && !sw.s.on(slots, "pulled")
            } else {
                !failed && sw.when.as_mut().is_none_or(|w| w.yes(slots, dt))
            };
            self.net.set_closed(sw.link, closed);
            sw.s.flag(slots, "closed", closed);
        }
        for b in &self.buses {
            if b.s.on(slots, "failed") {
                self.net.stamp(b.node, easyacft_core::STIFF, 0.0);
            }
        }
        for bat in &mut self.batteries {
            if !bat.s.on(slots, "failed") {
                let ocv = ocv(bat.nominal, bat.charge);
                let over = bat.s.get(slots, "voltage") - ocv;
                bat.g_now = if over > 0.0 { bat.g.min(bat.limit / over) } else { bat.g };
                self.net.stamp(bat.node, bat.g_now, bat.g_now * ocv);
            }
        }
        for src in &mut self.sources {
            let on = !src.s.on(slots, "failed") && src.when.as_mut().is_none_or(|w| w.yes(slots, dt));
            src.s.flag(slots, if src.ext { "available" } else { "ready" }, on);
            if on {
                self.net.stamp(src.node, src.g, src.g * src.voltage);
            }
        }
        let mut sc = std::mem::take(&mut self.scratch);
        sc.powered.clear();
        sc.powered.extend((0..self.buses.len()).map(|b| self.bus_powered(slots, b)));
        for cv in &mut self.converters {
            let on = !cv.s.on(slots, "failed") && sc.powered[cv.input] && cv.when.as_mut().is_none_or(|w| w.yes(slots, dt));
            cv.s.flag(slots, "active", on);
            if on {
                self.net.stamp(cv.out_node, cv.g, cv.g * cv.voltage);
                let vin = self.buses[cv.input].s.get(slots, "voltage").max(0.3 * self.buses[cv.input].nominal);
                self.net.stamp(cv.in_node, cv.drawn / cv.eff / (vin * vin), 0.0);
            }
        }
        sc.islands.clear();
        sc.islands.extend_from_slice(self.net.islands_of());
        let count = sc.islands.iter().max().map_or(0, |m| m + 1);
        for v in [&mut sc.capacity, &mut sc.demand] {
            v.clear();
            v.resize(count, 0.0);
        }
        sc.has_capacity.clear();
        sc.has_capacity.resize(count, false);
        let islands = &sc.islands;
        for src in &self.sources {
            if slots[src.s.base[if src.ext { "available" } else { "ready" }]] != 0.0 {
                sc.capacity[islands[src.node]] += src.rating * src.power_factor;
                sc.has_capacity[islands[src.node]] = true;
            }
        }
        for cv in &self.converters {
            if cv.s.on(slots, "active") {
                sc.capacity[islands[cv.out_node]] += cv.rating;
                sc.has_capacity[islands[cv.out_node]] = true;
            }
        }
        sc.wanted.clear();
        for l in self.loads.iter_mut() {
            l.power = l.power_e.as_mut().map_or(0.0, |e| e.eval(slots, dt).max(0.0));
            let wanted = !l.s.on(slots, "failed") && l.when.as_mut().is_none_or(|w| w.yes(slots, dt)) && sc.powered[l.bus];
            sc.wanted.push(wanted);
            if wanted && !l.shed && !l.dropped {
                sc.demand[islands[l.node]] += l.power;
            }
        }
        for cv in &self.converters {
            if cv.s.on(slots, "active") {
                sc.demand[islands[cv.in_node]] += cv.drawn / cv.eff;
            }
        }
        for &k in &self.shed_order {
            let isl = islands[self.loads[k].node];
            if !sc.has_capacity[isl] || !sc.wanted[k] {
                continue;
            }
            let (cap, dem) = (sc.capacity[isl], sc.demand[isl]);
            let l = &mut self.loads[k];
            if !l.shed && dem > cap {
                l.shed = true;
                sc.demand[isl] -= l.power;
            } else if l.shed && dem + l.power <= cap * UNSHED_MARGIN {
                l.shed = false;
                sc.demand[isl] += l.power;
            }
        }
        for (k, l) in self.loads.iter_mut().enumerate() {
            l.g = 0.0;
            if sc.wanted[k] && !l.shed && !l.dropped {
                let b = &self.buses[l.bus];
                let v = b.s.get(slots, "voltage").max(LINEARISE_FLOOR * b.nominal);
                l.g = l.power / (v * v);
                self.net.stamp(l.node, l.g, 0.0);
            }
        }
        self.ports.stamp(&mut self.net, slots);
        self.net.step(dt);
        self.ports.read(&self.net, slots);
        for b in &self.buses {
            let v = self.net.potential(b.node).max(0.0);
            b.s.put(slots, "voltage", v);
            b.s.flag(slots, "powered", v >= b.powered_at * b.nominal);
        }
        for bat in &mut self.batteries {
            let v = self.net.potential(bat.node);
            let ocv = ocv(bat.nominal, bat.charge);
            let i = if bat.s.on(slots, "failed") { 0.0 } else { bat.g_now * (ocv - v) };
            bat.charge = (bat.charge - i * dt / bat.capacity).clamp(0.0, 1.0);
            bat.s.put(slots, "voltage", v.max(0.0));
            bat.s.put(slots, "current", i);
            bat.s.put(slots, "charge", bat.charge);
        }
        for src in &self.sources {
            let on = slots[src.s.base[if src.ext { "available" } else { "ready" }]] != 0.0;
            let v = self.net.potential(src.node);
            let i = if on { src.g * (src.voltage - v) } else { 0.0 };
            src.s.put(slots, "voltage", if on { v.max(0.0) } else { 0.0 });
            src.s.put(slots, "current", i);
            src.s.put(slots, "load", i * v.max(0.0));
            if !src.ext {
                src.s.put(slots, "frequency", if on { src.frequency } else { 0.0 });
            }
        }
        for cv in &mut self.converters {
            let on = cv.s.on(slots, "active");
            let v = self.net.potential(cv.out_node);
            let i = if on { cv.g * (cv.voltage - v) } else { 0.0 };
            cv.drawn = (i * v).max(0.0);
            cv.s.put(slots, "current", i);
            cv.s.put(slots, "power", cv.drawn);
        }
        for (k, l) in self.loads.iter_mut().enumerate() {
            let v = self.net.potential(l.node).max(0.0);
            let drawn = l.g * v * v;
            let wanted = sc.wanted[k] && !l.shed;
            if !wanted {
                l.dropped = false;
                l.recover = 0.0;
            } else if !l.dropped && v < l.min_voltage {
                l.dropped = true;
                l.recover = 0.0;
            } else if l.dropped {
                l.recover = if v >= l.min_voltage * RECOVER_RATIO { l.recover + dt } else { 0.0 };
                if l.recover >= RECOVER_S {
                    l.dropped = false;
                }
            }
            l.s.flag(slots, "powered", wanted && !l.dropped);
            l.s.flag(slots, "shed", l.shed && sc.wanted[k]);
            l.s.put(slots, "power", drawn);
            l.s.put(slots, "current", if v > 0.0 { drawn / v } else { 0.0 });
        }
        for sw in &mut self.switches {
            let i = self.net.flow(sw.link);
            sw.s.put(slots, "current", i);
            if sw.rating > 0.0 {
                let r = (i / sw.rating).powi(2);
                sw.heat = (sw.heat + (r - 1.0) * dt).max(0.0);
                if sw.heat > TRIP_HEAT {
                    sw.tripped = true;
                }
                if sw.tripped && sw.s.on(slots, "pulled") {
                    sw.tripped = false;
                    sw.heat = 0.0;
                }
                sw.s.flag(slots, "tripped", sw.tripped);
            }
        }
        self.scratch = sc;
    }

    fn state(&self) -> Vec<f64> {
        let mut v = self.net.state();
        v.extend(self.batteries.iter().map(|b| b.charge));
        v.extend(self.switches.iter().flat_map(|s| [s.heat, s.tripped as u8 as f64]));
        v.extend(self.loads.iter().flat_map(|l| [l.shed as u8 as f64, l.dropped as u8 as f64, l.recover]));
        v.extend(self.converters.iter().map(|c| c.drawn));
        v
    }

    fn restore(&mut self, s: &[f64]) {
        let n = self.net.state().len();
        if s.len() < n {
            return;
        }
        self.net.restore(&s[..n]);
        let mut at = n;
        let mut next = || {
            let v = s.get(at).copied().unwrap_or(0.0);
            at += 1;
            v
        };
        for b in &mut self.batteries {
            b.charge = next();
        }
        for sw in &mut self.switches {
            sw.heat = next();
            sw.tripped = next() != 0.0;
        }
        for l in &mut self.loads {
            l.shed = next() != 0.0;
            l.dropped = next() != 0.0;
            l.recover = next();
        }
        for c in &mut self.converters {
            c.drawn = next();
        }
    }
}
