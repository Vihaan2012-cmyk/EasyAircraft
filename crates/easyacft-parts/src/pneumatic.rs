//! Pneumatic domain: bleed sources behind check valves, ducts with an energy balance, leaks and overheat,
//! pneumatically actuated shutoff, crossbleed and regulating valves, precoolers, consumers.

use crate::cabin::orifice;
use crate::util::{Packer, Slots};
use crate::{expr_of, Domain, Expr};
use easyacft_core::{Dim, Network};
use easyacft_lang::schema::{opt, req, Kind, PartSpec};
use easyacft_lang::{Checked, CheckedPart, Type};
use std::collections::HashMap;

const P: Type = Type::Num(Dim::PRESSURE);
const T: Type = Type::Num(Dim::TEMPERATURE);
const MF: Type = Type::Num(Dim::MASS_FLOW);
const N: Type = Type::Num(Dim::NONE);
const B: Type = Type::Bool;
const WHEN: Kind = Kind::Expr(Type::Bool);
const DUCT: &[&str] = &["pneumatic.duct"];
const R_AIR: f64 = 287.05;
const CP: f64 = 1005.0;
const CV: f64 = 718.0;
const AMBIENT_K: f64 = 288.15;
const AMBIENT_P: f64 = 101_325.0;
const DISCHARGE: f64 = 0.8;
/// Leak flow a duct's detection loop notices, and how long it takes.
const LEAK_DETECT: f64 = 0.05;
const LEAK_DETECT_S: f64 = 3.0;

pub static SPECS: &[PartSpec] = &[
    PartSpec {
        domain: "pneumatic",
        kind: "duct",
        help: "A bleed duct or manifold: its air temperature follows an energy balance of what flows in and what it loses to the nacelle or bay; it can leak and its loop detects the leak.",
        fields: &[
            opt("volume", Kind::Quantity(Dim::VOLUME), "0.05 m3", "air volume it holds"),
            opt("ambient", Kind::Expr(T), "288.15 K", "temperature around it"),
            opt("ambient_pressure", Kind::Expr(P), "101325 Pa", "pressure around it, for a leak"),
            opt("heat_loss", Kind::Number, "20", "heat lost through its insulation, W per K"),
            opt("leak_when", WHEN, "false", "a duct rupture or leak"),
            opt("leak_area", Kind::Number, "0.002", "effective area of that leak, m²"),
            opt("overheat_temperature", Kind::Quantity(Dim::TEMPERATURE), "0 K", "overheat warning; 0 K for none"),
        ],
        outputs: &[("pressure", P), ("temperature", T), ("leak_flow", MF), ("leak_detected", B), ("overheat", B), ("failed", B)],
    },
    PartSpec {
        domain: "pneumatic",
        kind: "source",
        help: "An engine bleed port, APU load compressor or ground air cart, behind its check valve.",
        fields: &[
            req("duct", Kind::Ref(DUCT), "the duct it feeds"),
            req("pressure", Kind::Expr(P), "delivery pressure at no flow, or an expression such as the engine's compressor pressure"),
            req("temperature", Kind::Expr(T), "delivery temperature, or an expression"),
            req("flow", Kind::Expr(MF), "flow at full output, or an expression"),
            opt("available_when", WHEN, "true", "when it can supply"),
        ],
        outputs: &[("available", B), ("flow", MF), ("failed", B)],
    },
    PartSpec {
        domain: "pneumatic",
        kind: "valve",
        help: "A shutoff, crossbleed or pressure-regulating valve: travel time, spring-closed pneumatic actuation, overpressure shutoff, sticking, and an optional precooler with fan air.",
        fields: &[
            req("between", Kind::Refs(DUCT), "upstream and downstream duct"),
            opt("open_when", WHEN, "true", "when it is commanded open"),
            opt("flow", Kind::Quantity(Dim::MASS_FLOW), "2 kg/s", "flow at 1 bar across it, when no area is given"),
            opt("area", Kind::Number, "0", "open flow area, m²; flow is then compressible and chokes"),
            opt("open_time", Kind::Quantity(Dim::TIME), "0 s", "full travel time"),
            opt("actuation_pressure", Kind::Quantity(Dim::PRESSURE), "0 Pa", "upstream pressure it needs to open; it springs shut below; 0 for an electric valve"),
            opt("regulate", Kind::Quantity(Dim::PRESSURE), "0 Pa", "downstream pressure it holds; 0 for a plain valve"),
            opt("overpressure", Kind::Quantity(Dim::PRESSURE), "0 Pa", "it shuts when downstream goes above this, and reopens 10 % below"),
            opt("stuck_when", WHEN, "false", "it stays where it is"),
            opt("cool_to", Kind::Quantity(Dim::TEMPERATURE), "0 K", "precooler outlet temperature its fan-air valve aims for; 0 for none"),
            opt("effectiveness", Kind::Number, "0.8", "precooler effectiveness with full fan air"),
            opt("fan_temperature", Kind::Expr(T), "318.15 K", "fan air temperature"),
            opt("fan_air_when", WHEN, "true", "fan air reaches the precooler"),
        ],
        outputs: &[("open", B), ("position", N), ("flow", MF), ("outlet_temperature", T), ("failed", B)],
    },
    PartSpec {
        domain: "pneumatic",
        kind: "consumer",
        help: "A pack, anti-ice valve, engine starter or other user of bleed air; a flow control valve holds its flow.",
        fields: &[
            req("duct", Kind::Ref(DUCT), "the duct it draws from"),
            req("flow", Kind::Expr(MF), "flow it takes at nominal pressure, or an expression"),
            opt("nominal", Kind::Quantity(Dim::PRESSURE), "45 psi", "pressure its flow is given at"),
            opt("minimum", Kind::Quantity(Dim::PRESSURE), "15 psi", "below this it is not supplied"),
            opt("on_when", WHEN, "true", "when it draws"),
            opt("regulated", Kind::Bool, "false", "a flow control valve holds its flow above the minimum pressure"),
        ],
        outputs: &[("supplied", B), ("flow", MF), ("temperature", T), ("failed", B)],
    },
];

fn eval(e: &mut Option<Expr>, slots: &[f64], dt: f64, or: f64) -> f64 {
    e.as_mut().map_or(or, |x| x.eval(slots, dt))
}

fn yes(e: &mut Option<Expr>, slots: &[f64], dt: f64, or: bool) -> bool {
    e.as_mut().map_or(or, |x| x.yes(slots, dt))
}

/// Linearised resistance of an orifice at the pressures across it.
fn orifice_r(cda: f64, p1: f64, t1: f64, p2: f64) -> f64 {
    let hi = p1.max(p2).max(1e3);
    let dp = (p1 - p2).abs().max(0.005 * hi);
    dp / orifice(cda, hi, t1, hi - dp).max(1e-9)
}

struct Duct {
    node: usize,
    volume: f64,
    temp: f64,
    ambient: Option<Expr>,
    ambient_p: Option<Expr>,
    t_amb: f64,
    p_amb: f64,
    heat_loss: f64,
    leak_when: Option<Expr>,
    leak_cda: f64,
    leak_g: f64,
    leak_timer: f64,
    overheat: f64,
    s: Slots,
}

struct Source {
    duct: usize,
    g: f64,
    flow_e: Option<Expr>,
    pressure_e: Option<Expr>,
    temp_e: Option<Expr>,
    pressure: f64,
    temp: f64,
    when: Option<Expr>,
    on: bool,
    s: Slots,
}

struct Valve {
    link: usize,
    ends: [usize; 2],
    r_open: f64,
    cda: f64,
    open_time: f64,
    actuation: f64,
    regulate: f64,
    overpressure: f64,
    shut_op: bool,
    stuck: Option<Expr>,
    cool_to: f64,
    effectiveness: f64,
    fan_t: Option<Expr>,
    fan_when: Option<Expr>,
    t_out: f64,
    opening: f64,
    position: f64,
    when: Option<Expr>,
    s: Slots,
}

struct Consumer {
    duct: usize,
    flow: f64,
    flow_e: Option<Expr>,
    nominal: f64,
    minimum: f64,
    regulated: bool,
    g: f64,
    drawing: bool,
    when: Option<Expr>,
    s: Slots,
}

pub struct Pneumatic {
    net: Network,
    ports: crate::ports::Ports,
    ducts: Vec<Duct>,
    sources: Vec<Source>,
    valves: Vec<Valve>,
    consumers: Vec<Consumer>,
    heat_in: Vec<(f64, f64)>,
}

impl Pneumatic {
    pub fn new(c: &Checked) -> Option<Self> {
        let parts: Vec<(usize, &CheckedPart)> = c.parts.iter().enumerate().filter(|(_, p)| p.domain == "pneumatic").collect();
        if parts.is_empty() {
            return None;
        }
        let mut net = Network::new();
        let mut of: HashMap<usize, usize> = HashMap::new();
        let mut n = Pneumatic { net: Network::new(), ports: Default::default(), ducts: vec![], sources: vec![], valves: vec![], consumers: vec![], heat_in: vec![] };
        for &(i, p) in parts.iter().filter(|(_, p)| p.kind == "duct") {
            of.insert(i, n.ducts.len());
            let volume = p.num("volume").max(1e-6);
            n.ducts.push(Duct {
                node: net.add_node(volume / (R_AIR * 450.0)),
                volume,
                temp: AMBIENT_K,
                ambient: expr_of(p, "ambient"),
                ambient_p: expr_of(p, "ambient_pressure"),
                t_amb: AMBIENT_K,
                p_amb: AMBIENT_P,
                heat_loss: p.num("heat_loss").max(0.0),
                leak_when: expr_of(p, "leak_when"),
                leak_cda: DISCHARGE * p.num("leak_area").max(0.0),
                leak_g: 0.0,
                leak_timer: 0.0,
                overheat: p.num("overheat_temperature"),
                s: Slots::of(p, &["pressure", "temperature", "leak_flow", "leak_detected", "overheat", "failed"]),
            });
        }
        let duct = |p: &CheckedPart| p.part("duct").and_then(|x| of.get(&x).copied());
        for &(_, p) in &parts {
            match p.kind {
                "source" => {
                    let Some(d) = duct(p) else { continue };
                    n.sources.push(Source {
                        duct: d,
                        g: 0.0,
                        flow_e: expr_of(p, "flow"),
                        pressure_e: expr_of(p, "pressure"),
                        temp_e: expr_of(p, "temperature"),
                        pressure: 0.0,
                        temp: AMBIENT_K,
                        when: expr_of(p, "available_when"),
                        on: false,
                        s: Slots::of(p, &["available", "flow", "failed"]),
                    });
                }
                "valve" => {
                    let e = p.parts("between");
                    let (Some(&a), Some(&b)) = (e.first().and_then(|x| of.get(x)), e.get(1).and_then(|x| of.get(x))) else { continue };
                    let r = 1e5 / p.num("flow").max(1e-9);
                    let link = net.add_link(n.ducts[a].node, n.ducts[b].node, r, 0.0);
                    n.valves.push(Valve {
                        link,
                        ends: [a, b],
                        r_open: r,
                        cda: DISCHARGE * p.num("area").max(0.0),
                        open_time: p.num("open_time").max(0.0),
                        actuation: p.num("actuation_pressure"),
                        regulate: p.num("regulate"),
                        overpressure: p.num("overpressure"),
                        shut_op: false,
                        stuck: expr_of(p, "stuck_when"),
                        cool_to: p.num("cool_to"),
                        effectiveness: p.num("effectiveness").clamp(0.0, 1.0),
                        fan_t: expr_of(p, "fan_temperature"),
                        fan_when: expr_of(p, "fan_air_when"),
                        t_out: AMBIENT_K,
                        opening: 1.0,
                        position: 0.0,
                        when: expr_of(p, "open_when"),
                        s: Slots::of(p, &["open", "position", "flow", "outlet_temperature", "failed"]),
                    });
                }
                "consumer" => {
                    let Some(d) = duct(p) else { continue };
                    n.consumers.push(Consumer {
                        duct: d,
                        flow: 0.0,
                        flow_e: expr_of(p, "flow"),
                        nominal: p.num("nominal").max(1.0),
                        minimum: p.num("minimum"),
                        regulated: p.flag("regulated"),
                        g: 0.0,
                        drawing: false,
                        when: expr_of(p, "on_when"),
                        s: Slots::of(p, &["supplied", "flow", "temperature", "failed"]),
                    });
                }
                _ => {}
            }
        }
        n.ports = crate::ports::Ports::new(c, "pneumatic", &mut net, |i| of.get(&i).map(|&d| n.ducts[d].node));
        n.net = net;
        n.heat_in = vec![(0.0, 0.0); n.ducts.len()];
        Some(n)
    }
}

impl Domain for Pneumatic {
    fn name(&self) -> &'static str {
        "pneumatic"
    }

    fn step(&mut self, slots: &mut [f64], dt: f64) {
        self.net.begin_step();
        for s in self.sources.iter_mut() {
            s.pressure = eval(&mut s.pressure_e, slots, dt, 0.0).max(0.0);
            s.temp = eval(&mut s.temp_e, slots, dt, AMBIENT_K);
            s.g = eval(&mut s.flow_e, slots, dt, 0.0).max(0.0) / (0.3 * s.pressure.max(1e3));
            let behind_check = self.ducts[s.duct].s.get(slots, "pressure") < s.pressure;
            s.on = !s.s.on(slots, "failed") && s.pressure > 0.0 && yes(&mut s.when, slots, dt, true);
            s.s.flag(slots, "available", s.on);
            if s.on && behind_check {
                self.net.stamp(self.ducts[s.duct].node, s.g, s.g * s.pressure);
            }
        }
        for v in &mut self.valves {
            let up = self.ducts[v.ends[0]].s.get(slots, "pressure");
            let down = self.ducts[v.ends[1]].s.get(slots, "pressure");
            if v.overpressure > 0.0 {
                v.shut_op = if v.shut_op { down > 0.9 * v.overpressure } else { down > v.overpressure };
            }
            let commanded = !v.s.on(slots, "failed") && yes(&mut v.when, slots, dt, true) && !v.shut_op && (v.actuation <= 0.0 || up >= v.actuation);
            if !yes(&mut v.stuck, slots, dt, false) {
                let target = if commanded { 1.0 } else { 0.0 };
                v.position = if v.open_time > 0.0 { v.position + (target - v.position).clamp(-dt / v.open_time, dt / v.open_time) } else { target };
            }
            if v.regulate > 0.0 && commanded {
                let err = (v.regulate - down) / v.regulate;
                v.opening = (v.opening + err * 4.0 * dt).clamp(0.02, 1.0);
            } else {
                v.opening = 1.0;
            }
            let open = v.position > 1e-3;
            let base = if v.cda > 0.0 { orifice_r(v.cda, up, self.ducts[v.ends[0]].temp, down) } else { v.r_open };
            self.net.set_closed(v.link, open);
            self.net.set_resistance(v.link, base / (v.position * v.opening).max(1e-3));
            v.s.flag(slots, "open", open);
            v.s.put(slots, "position", v.position);
        }
        for c in self.consumers.iter_mut() {
            c.flow = eval(&mut c.flow_e, slots, dt, 0.0).max(0.0);
            c.drawing = !c.s.on(slots, "failed") && yes(&mut c.when, slots, dt, true);
            let p = self.ducts[c.duct].s.get(slots, "pressure");
            c.g = if c.regulated { c.flow / p.max(c.minimum).max(1.0) } else { c.flow / c.nominal };
            if c.drawing {
                self.net.stamp(self.ducts[c.duct].node, c.g, 0.0);
            }
        }
        for d in &mut self.ducts {
            d.t_amb = eval(&mut d.ambient, slots, dt, AMBIENT_K);
            d.p_amb = eval(&mut d.ambient_p, slots, dt, AMBIENT_P);
            if d.s.on(slots, "failed") {
                self.net.stamp(d.node, 1e-4, 0.0);
            }
            let p = d.s.get(slots, "pressure");
            d.leak_g = if yes(&mut d.leak_when, slots, dt, false) && p > d.p_amb { orifice(d.leak_cda, p, d.temp, d.p_amb) / (p - d.p_amb).max(1.0) } else { 0.0 };
            if d.leak_g > 0.0 {
                self.net.stamp(d.node, d.leak_g, d.leak_g * d.p_amb);
            }
        }
        self.ports.stamp(&mut self.net, slots);
        self.net.step(dt);
        self.ports.read(&self.net, slots);
        let heat_in = &mut self.heat_in;
        heat_in.iter_mut().for_each(|h| *h = (0.0, 0.0));
        for s in &self.sources {
            let p = self.net.potential(self.ducts[s.duct].node);
            let q = if s.on { (s.g * (s.pressure - p)).max(0.0) } else { 0.0 };
            s.s.put(slots, "flow", q);
            heat_in[s.duct].0 += q;
            heat_in[s.duct].1 += q * s.temp;
        }
        for v in &mut self.valves {
            let q = if v.position > 1e-3 { self.net.flow(v.link) } else { 0.0 };
            let (from, to) = if q >= 0.0 { (v.ends[0], v.ends[1]) } else { (v.ends[1], v.ends[0]) };
            let t_in = self.ducts[from].temp;
            v.t_out = if v.cool_to > 0.0 {
                let t_fan = eval(&mut v.fan_t, slots, dt, 318.15);
                let eff = if yes(&mut v.fan_when, slots, dt, true) { v.effectiveness } else { 0.1 * v.effectiveness };
                v.cool_to.max(t_in - eff * (t_in - t_fan).max(0.0)).min(t_in)
            } else {
                t_in
            };
            v.s.put(slots, "flow", q.abs());
            v.s.put(slots, "outlet_temperature", v.t_out);
            heat_in[to].0 += q.abs();
            heat_in[to].1 += q.abs() * v.t_out;
        }
        for (k, d) in self.ducts.iter_mut().enumerate() {
            let p = self.net.potential(d.node).max(0.0);
            let (m_in, h_in) = heat_in[k];
            let mass = (p.max(d.p_amb) * d.volume / (R_AIR * d.temp.max(100.0))).max(1e-4);
            d.temp = (mass * CV * d.temp + dt * (CP * h_in + d.heat_loss * d.t_amb)) / (mass * CV + dt * (CP * m_in + d.heat_loss));
            let leak = d.leak_g * (p - d.p_amb).max(0.0);
            d.leak_timer = if leak > LEAK_DETECT { d.leak_timer + dt } else { 0.0 };
            d.s.put(slots, "pressure", p);
            d.s.put(slots, "temperature", d.temp);
            d.s.put(slots, "leak_flow", leak);
            d.s.flag(slots, "leak_detected", d.leak_timer >= LEAK_DETECT_S);
            d.s.flag(slots, "overheat", d.overheat > 0.0 && d.temp > d.overheat);
        }
        for c in &self.consumers {
            let p = self.ducts[c.duct].s.get(slots, "pressure");
            let q = if c.drawing { c.g * p } else { 0.0 };
            c.s.flag(slots, "supplied", c.drawing && p >= c.minimum);
            c.s.put(slots, "flow", q);
            c.s.put(slots, "temperature", self.ducts[c.duct].temp);
        }
    }

    fn state(&self) -> Vec<f64> {
        let mut p = Packer(self.net.state(), 0);
        for d in &self.ducts {
            p.put(d.temp);
            p.put(d.leak_timer);
        }
        for v in &self.valves {
            [v.opening, v.position, v.shut_op as u8 as f64].into_iter().for_each(|x| p.put(x));
        }
        p.0
    }

    fn restore(&mut self, s: &[f64]) {
        let n = self.net.state().len();
        if s.len() < n {
            return;
        }
        self.net.restore(&s[..n]);
        let mut p = Packer(s[n..].to_vec(), 0);
        for d in &mut self.ducts {
            d.temp = p.take();
            d.leak_timer = p.take();
        }
        for v in &mut self.valves {
            v.opening = p.take();
            v.position = p.take();
            v.shut_op = p.take() != 0.0;
        }
    }
}
