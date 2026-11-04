//! Hydraulic domain: circuits with their reservoirs, fluid temperature and leaks; pumps that cavitate; gas-law
//! accumulators; consumers behind priority valves; power transfer units.

use crate::util::{Packer, Slots};
use crate::{expr_of, Domain, Expr};
use easyacft_core::{Dim, Network};
use easyacft_lang::schema::{opt, req, Kind, PartSpec};
use easyacft_lang::{Checked, Type};
use std::collections::HashMap;

const P: Type = Type::Num(Dim::PRESSURE);
const Q: Type = Type::Num(Dim::VOLUME_FLOW);
const VOL: Type = Type::Num(Dim::VOLUME);
const T: Type = Type::Num(Dim::TEMPERATURE);
const W: Type = Type::Num(Dim::POWER);
const N: Type = Type::Num(Dim::NONE);
const B: Type = Type::Bool;
const WHEN: Kind = Kind::Expr(Type::Bool);
const CIRCUIT: &[&str] = &["hydraulic.circuit"];
/// Pump outlet pressure drop at full flow, share of nominal: the pressure compensator's band.
const DROOP: f64 = 0.15;
/// Fluid density and specific heat, with the pipes and components counted as twice the fluid's heat capacity.
const FLUID_HEAT: f64 = 870.0 * 1900.0 * 3.0;
/// Heat exchanger conductance per litre of reservoir fluid when none is given, W/K.
const EXCHANGER_PER_LITRE: f64 = 15.0;
/// Polytropic exponent of accumulator nitrogen.
const GAS_N: f64 = 1.3;
/// Pumps start losing flow when the reservoir holds less than this share of its starting fluid.
const LEVEL_STARVE: f64 = 0.02;

pub static SPECS: &[PartSpec] = &[
    PartSpec {
        domain: "hydraulic",
        kind: "circuit",
        help: "One hydraulic system with its reservoir: fluid in lines and accumulators comes out of the reservoir, external leaks lose it, pump and user losses heat it and a heat exchanger cools it.",
        fields: &[
            req("nominal", Kind::Quantity(Dim::PRESSURE), "for example \"3000 psi\""),
            req("fluid", Kind::Quantity(Dim::VOLUME), "reservoir quantity at start, for example \"20 L\""),
            opt("low_pressure", Kind::Quantity(Dim::PRESSURE), "1500 psi", "below this the circuit counts as lost"),
            opt("compliance", Kind::Number, "1e-10", "volume per pascal the lines and hoses store"),
            opt("leak", Kind::Quantity(Dim::VOLUME_FLOW), "0.05 L/min", "internal leakage back to the reservoir at nominal pressure"),
            opt("leak_when", WHEN, "false", "an external leak: fluid goes overboard"),
            opt("leak_rate", Kind::Quantity(Dim::VOLUME_FLOW), "2 L/min", "that leak at nominal pressure"),
            opt("low_level", Kind::Quantity(Dim::VOLUME), "0 L", "reservoir low-level warning; 0 L means a quarter of the starting fluid"),
            opt("reservoir_pressure", Kind::Expr(P), "50 psi", "reservoir air pressurisation above ambient"),
            opt("ambient", Kind::Expr(P), "101325 Pa", "outside pressure, for an unpressurised reservoir"),
            opt("cavitation_pressure", Kind::Quantity(Dim::PRESSURE), "15 psi", "absolute pump inlet pressure needed at full flow"),
            opt("cooling_temperature", Kind::Expr(T), "288.15 K", "what the heat exchanger dumps heat into, usually fuel"),
            opt("heat_exchanger", Kind::Number, "0", "heat exchanger conductance, W per K; 0 sizes it from the reservoir"),
            opt("overheat_temperature", Kind::Quantity(Dim::TEMPERATURE), "368.15 K", "fluid overheat warning"),
        ],
        outputs: &[("pressure", P), ("fluid", VOL), ("temperature", T), ("available", B), ("low", B), ("low_level", B), ("overheat", B), ("failed", B)],
    },
    PartSpec {
        domain: "hydraulic",
        kind: "pump",
        help: "An engine-driven, electric or wind-driven pump with a pressure compensator; it cavitates on a low reservoir or low inlet pressure.",
        fields: &[
            req("circuit", Kind::Ref(CIRCUIT), "the circuit it feeds"),
            req("pressure", Kind::Quantity(Dim::PRESSURE), "outlet pressure at no flow"),
            req("flow", Kind::Expr(Q), "flow it delivers at full output, for example \"140 L/min\", or an expression"),
            opt("running_when", WHEN, "true", "when it turns"),
            opt("speed", Kind::Expr(N), "1", "drive speed as a share of rated; flow follows it"),
            opt("efficiency", Kind::Number, "0.85", "hydraulic power out per shaft power in"),
        ],
        outputs: &[("running", B), ("flow", Q), ("pressure", P), ("power", W), ("cavitating", B), ("failed", B)],
    },
    PartSpec {
        domain: "hydraulic",
        kind: "accumulator",
        help: "A nitrogen-charged accumulator: its gas follows the gas law, it fills from and gives back to its circuit, or behind a check valve only fills.",
        fields: &[
            req("circuit", Kind::Ref(CIRCUIT), "the circuit it is on"),
            req("volume", Kind::Quantity(Dim::VOLUME), "gas volume"),
            req("precharge", Kind::Quantity(Dim::PRESSURE), "gas precharge pressure"),
            opt("check_valve", Kind::Bool, "false", "a check valve keeps its fluid for its own users, such as the brakes"),
            opt("draw", Kind::Expr(Q), "0 L/min", "flow its own users take from it"),
            opt("precharge_lost_when", WHEN, "false", "the nitrogen has leaked away"),
        ],
        outputs: &[("pressure", P), ("fluid", VOL), ("failed", B)],
    },
    PartSpec {
        domain: "hydraulic",
        kind: "consumer",
        help: "Actuators or a motor that use flow while they move.",
        fields: &[
            req("circuit", Kind::Ref(CIRCUIT), "the circuit it uses"),
            req("flow", Kind::Expr(Q), "flow while moving at full rate, or an expression"),
            opt("moving_when", WHEN, "false", "when it is moving"),
            opt("minimum", Kind::Quantity(Dim::PRESSURE), "1000 psi", "lowest pressure it works on"),
            opt("priority_below", Kind::Quantity(Dim::PRESSURE), "0 psi", "a priority valve throttles it to hold the circuit at this, shut 5 % below"),
        ],
        outputs: &[("powered", B), ("flow", Q), ("rate", N), ("failed", B)],
    },
    PartSpec {
        domain: "hydraulic",
        kind: "ptu",
        help: "A power transfer unit: a motor on the high side drives a pump on the low side; it moves power, not fluid.",
        fields: &[
            req("between", Kind::Refs(CIRCUIT), "two circuit ids"),
            opt("threshold", Kind::Quantity(Dim::PRESSURE), "500 psi", "pressure difference that starts it"),
            req("flow", Kind::Quantity(Dim::VOLUME_FLOW), "most flow it delivers"),
            opt("efficiency", Kind::Number, "0.8", "power out per power in"),
            opt("enabled_when", WHEN, "true", "when it is armed"),
        ],
        outputs: &[("running", B), ("flow", Q), ("power", W), ("failed", B)],
    },
];

fn eval(e: &mut Option<Expr>, slots: &[f64], dt: f64, or: f64) -> f64 {
    e.as_mut().map_or(or, |x| x.eval(slots, dt))
}

fn yes(e: &mut Option<Expr>, slots: &[f64], dt: f64, or: bool) -> bool {
    e.as_mut().map_or(or, |x| x.yes(slots, dt))
}

struct Circuit {
    node: usize,
    nominal: f64,
    low: f64,
    compliance: f64,
    fluid: f64,
    fluid0: f64,
    level: f64,
    low_level: f64,
    leak_g: f64,
    leak_when: Option<Expr>,
    ext_g: f64,
    reservoir: Option<Expr>,
    ambient: Option<Expr>,
    inlet: f64,
    cavitation: f64,
    cooling: Option<Expr>,
    exchanger: f64,
    heat_capacity: f64,
    overheat: f64,
    temp: f64,
    heat: f64,
    s: Slots,
}

struct Pump {
    circuit: usize,
    g: f64,
    max_flow: f64,
    flow_e: Option<Expr>,
    speed: Option<Expr>,
    pressure: f64,
    eff: f64,
    share: f64,
    when: Option<Expr>,
    running: bool,
    s: Slots,
}

struct Accumulator {
    circuit: usize,
    volume: f64,
    precharge: f64,
    check: bool,
    draw: Option<Expr>,
    lost: Option<Expr>,
    fluid: f64,
    g: f64,
    p_gas: f64,
    stamped: bool,
    s: Slots,
}

struct Consumer {
    circuit: usize,
    flow: f64,
    flow_e: Option<Expr>,
    minimum: f64,
    priority: f64,
    share: f64,
    when: Option<Expr>,
    demand: f64,
    s: Slots,
}

struct Ptu {
    a: usize,
    b: usize,
    threshold: f64,
    flow: f64,
    eff: f64,
    g: f64,
    when: Option<Expr>,
    on: bool,
    hi: usize,
    lo: usize,
    e: f64,
    out_power: f64,
    s: Slots,
}

pub struct Hydraulic {
    net: Network,
    ports: crate::ports::Ports,
    circuits: Vec<Circuit>,
    pumps: Vec<Pump>,
    accumulators: Vec<Accumulator>,
    consumers: Vec<Consumer>,
    ptus: Vec<Ptu>,
}

impl Accumulator {
    fn gas_pressure(&self, pre: f64) -> f64 {
        let gas = (self.volume - self.fluid).max(0.02 * self.volume);
        pre * (self.volume / gas).powf(GAS_N)
    }
}

impl Hydraulic {
    pub fn new(c: &Checked) -> Option<Self> {
        let parts: Vec<(usize, &easyacft_lang::CheckedPart)> = c.parts.iter().enumerate().filter(|(_, p)| p.domain == "hydraulic").collect();
        if parts.is_empty() {
            return None;
        }
        let mut net = Network::new();
        let mut of: HashMap<usize, usize> = HashMap::new();
        let mut h = Hydraulic { net: Network::new(), ports: Default::default(), circuits: vec![], pumps: vec![], accumulators: vec![], consumers: vec![], ptus: vec![] };
        for &(i, p) in parts.iter().filter(|(_, p)| p.kind == "circuit") {
            let cap = p.num("compliance").max(0.0);
            let nominal = p.num("nominal").max(1.0);
            let fluid = p.num("fluid");
            let low_level = p.num("low_level");
            let exchanger = p.num("heat_exchanger");
            of.insert(i, h.circuits.len());
            h.circuits.push(Circuit {
                node: net.add_node(cap),
                nominal,
                low: p.num("low_pressure"),
                compliance: cap,
                fluid,
                fluid0: fluid.max(1e-6),
                level: fluid,
                low_level: if low_level > 0.0 { low_level } else { 0.25 * fluid },
                leak_g: p.num("leak") / nominal,
                leak_when: expr_of(p, "leak_when"),
                ext_g: p.num("leak_rate") / nominal,
                reservoir: expr_of(p, "reservoir_pressure"),
                ambient: expr_of(p, "ambient"),
                inlet: 101_325.0,
                cavitation: p.num("cavitation_pressure"),
                cooling: expr_of(p, "cooling_temperature"),
                exchanger: if exchanger > 0.0 { exchanger } else { EXCHANGER_PER_LITRE * fluid * 1000.0 },
                heat_capacity: (fluid * FLUID_HEAT).max(1.0),
                overheat: p.num("overheat_temperature"),
                temp: f64::NAN,
                heat: 0.0,
                s: Slots::of(p, &["pressure", "fluid", "temperature", "available", "low", "low_level", "overheat", "failed"]),
            });
        }
        let circ = |p: &easyacft_lang::CheckedPart| p.part("circuit").and_then(|x| of.get(&x).copied());
        for &(_, p) in &parts {
            match p.kind {
                "pump" => {
                    let Some(ci) = circ(p) else { continue };
                    h.pumps.push(Pump {
                        circuit: ci,
                        g: 0.0,
                        max_flow: 0.0,
                        flow_e: expr_of(p, "flow"),
                        speed: expr_of(p, "speed"),
                        pressure: p.num("pressure").max(1.0),
                        eff: p.num("efficiency").clamp(0.05, 1.0),
                        share: 0.0,
                        when: expr_of(p, "running_when"),
                        running: false,
                        s: Slots::of(p, &["running", "flow", "pressure", "power", "cavitating", "failed"]),
                    });
                }
                "accumulator" => {
                    let Some(ci) = circ(p) else { continue };
                    h.accumulators.push(Accumulator {
                        circuit: ci,
                        volume: p.num("volume").max(1e-6),
                        precharge: p.num("precharge").max(1.0),
                        check: p.flag("check_valve"),
                        draw: expr_of(p, "draw"),
                        lost: expr_of(p, "precharge_lost_when"),
                        fluid: 0.0,
                        g: 0.0,
                        p_gas: p.num("precharge"),
                        stamped: false,
                        s: Slots::of(p, &["pressure", "fluid", "failed"]),
                    });
                }
                "consumer" => {
                    let Some(ci) = circ(p) else { continue };
                    h.consumers.push(Consumer {
                        circuit: ci,
                        flow: 0.0,
                        flow_e: expr_of(p, "flow"),
                        minimum: p.num("minimum"),
                        priority: p.num("priority_below"),
                        share: 1.0,
                        when: expr_of(p, "moving_when"),
                        demand: 0.0,
                        s: Slots::of(p, &["powered", "flow", "rate", "failed"]),
                    });
                }
                "ptu" => {
                    let ends = p.parts("between");
                    let (Some(&a), Some(&b)) = (ends.first().and_then(|x| of.get(x)), ends.get(1).and_then(|x| of.get(x))) else { continue };
                    let threshold = p.num("threshold").max(1.0);
                    let flow = p.num("flow");
                    h.ptus.push(Ptu {
                        a,
                        b,
                        threshold,
                        flow,
                        eff: p.num("efficiency").clamp(0.05, 1.0),
                        g: flow / threshold,
                        when: expr_of(p, "enabled_when"),
                        on: false,
                        hi: a,
                        lo: b,
                        e: 0.0,
                        out_power: 0.0,
                        s: Slots::of(p, &["running", "flow", "power", "failed"]),
                    });
                }
                _ => {}
            }
        }
        h.ports = crate::ports::Ports::new(c, "hydraulic", &mut net, |i| of.get(&i).map(|&ci| h.circuits[ci].node));
        h.net = net;
        Some(h)
    }
}

impl Domain for Hydraulic {
    fn name(&self) -> &'static str {
        "hydraulic"
    }

    fn step(&mut self, slots: &mut [f64], dt: f64) {
        self.net.begin_step();
        for c in &mut self.circuits {
            let failed = c.s.on(slots, "failed");
            let ext = if failed { c.leak_g * 1e4 } else if yes(&mut c.leak_when, slots, dt, false) { c.ext_g } else { 0.0 };
            c.inlet = eval(&mut c.reservoir, slots, dt, 0.0).max(0.0) + eval(&mut c.ambient, slots, dt, 101_325.0).max(0.0);
            self.net.stamp(c.node, c.leak_g + ext, 0.0);
            let p = c.s.get(slots, "pressure");
            c.fluid = (c.fluid - ext * p * dt).max(0.0);
            let cool = eval(&mut c.cooling, slots, dt, 288.15);
            if c.temp.is_nan() {
                c.temp = cool;
            }
            c.temp += (c.heat - c.exchanger * (c.temp - cool)) * dt / c.heat_capacity;
            c.heat = 0.0;
        }
        for pm in self.pumps.iter_mut() {
            let c = &self.circuits[pm.circuit];
            pm.max_flow = eval(&mut pm.flow_e, slots, dt, 0.0).max(0.0) * eval(&mut pm.speed, slots, dt, 1.0).max(0.0);
            let level = (c.level / (LEVEL_STARVE * c.fluid0)).clamp(0.0, 1.0);
            let need = c.cavitation * pm.share * pm.share;
            let suction = if need > 1.0 { (c.inlet / need).clamp(0.0, 1.0) } else { 1.0 };
            let factor = level * suction;
            pm.running = !pm.s.on(slots, "failed") && c.level > 0.0 && yes(&mut pm.when, slots, dt, true);
            pm.g = pm.max_flow * factor / (DROOP * pm.pressure);
            pm.s.flag(slots, "running", pm.running);
            pm.s.flag(slots, "cavitating", pm.running && pm.max_flow > 0.0 && factor < 0.95);
            if pm.running {
                self.net.stamp(c.node, pm.g, pm.g * pm.pressure);
            }
        }
        for a in &mut self.accumulators {
            let c = &self.circuits[a.circuit];
            let p = c.s.get(slots, "pressure");
            let pre = if yes(&mut a.lost, slots, dt, false) { 0.05 * a.precharge } else { a.precharge };
            a.fluid = (a.fluid - eval(&mut a.draw, slots, dt, 0.0).max(0.0) * dt).max(0.0);
            a.p_gas = a.gas_pressure(pre);
            let compliance = (a.volume - a.fluid).max(0.02 * a.volume) / (GAS_N * a.p_gas);
            a.g = compliance / (4.0 * dt.max(1e-4));
            let charging = p > a.p_gas;
            a.stamped = !a.s.on(slots, "failed") && if a.check { charging } else { charging || a.fluid > 0.0 };
            if a.stamped {
                self.net.stamp(c.node, a.g, a.g * a.p_gas);
            }
        }
        for u in self.consumers.iter_mut() {
            u.flow = eval(&mut u.flow_e, slots, dt, 0.0).max(0.0);
            let c = &self.circuits[u.circuit];
            let p = c.s.get(slots, "pressure");
            u.share = if u.priority > 0.0 { ((p - u.priority) / (0.05 * u.priority)).clamp(0.0, 1.0) } else { 1.0 };
            let moving = !u.s.on(slots, "failed") && yes(&mut u.when, slots, dt, false);
            u.demand = 0.0;
            if moving && p > 0.0 && u.share > 0.0 {
                u.demand = u.flow * u.share;
                self.net.stamp(c.node, u.demand / c.nominal, 0.0);
            }
        }
        for t in &mut self.ptus {
            let (pa, pb) = (self.circuits[t.a].s.get(slots, "pressure"), self.circuits[t.b].s.get(slots, "pressure"));
            let armed = !t.s.on(slots, "failed") && yes(&mut t.when, slots, dt, true);
            let dp = (pa - pb).abs();
            let was = t.on;
            t.on = armed && (dp > t.threshold || t.on && dp > 0.3 * t.threshold);
            if t.on && !was {
                (t.hi, t.lo) = if pa >= pb { (t.a, t.b) } else { (t.b, t.a) };
                t.e = self.circuits[t.hi].s.get(slots, "pressure") - 0.5 * t.threshold;
            }
            t.s.flag(slots, "running", t.on);
            if t.on {
                let p_hi = self.circuits[t.hi].s.get(slots, "pressure").max(1.0);
                t.e = t.e.min(p_hi - 0.5 * t.threshold).max(0.0);
                self.net.stamp(self.circuits[t.lo].node, t.g, t.g * t.e);
                self.net.stamp(self.circuits[t.hi].node, t.out_power / t.eff / (p_hi * p_hi), 0.0);
            }
        }
        self.ports.stamp(&mut self.net, slots);
        self.net.step(dt);
        self.ports.read(&self.net, slots);
        for pm in self.pumps.iter_mut() {
            let pc = self.net.potential(self.circuits[pm.circuit].node);
            let q = if pm.running { (pm.g * (pm.pressure - pc)).max(0.0) } else { 0.0 };
            pm.share = if pm.max_flow > 0.0 { q / pm.max_flow } else { 0.0 };
            let power = q * pc.max(0.0) / pm.eff;
            self.circuits[pm.circuit].heat += power;
            pm.s.put(slots, "flow", q);
            pm.s.put(slots, "pressure", if pm.running { pc.max(0.0) } else { 0.0 });
            pm.s.put(slots, "power", power);
        }
        for a in &mut self.accumulators {
            let p = self.net.potential(self.circuits[a.circuit].node);
            if a.stamped {
                a.fluid = (a.fluid + a.g * (p - a.p_gas) * dt).clamp(0.0, 0.98 * a.volume);
            }
            a.s.put(slots, "pressure", a.p_gas.max(if a.fluid > 0.0 { 0.0 } else { a.precharge }));
            a.s.put(slots, "fluid", a.fluid);
        }
        for t in &mut self.ptus {
            if !t.on {
                t.out_power = 0.0;
                t.s.put(slots, "flow", 0.0);
                t.s.put(slots, "power", 0.0);
                continue;
            }
            let p_lo = self.net.potential(self.circuits[t.lo].node).max(0.0);
            let q = (t.g * (t.e - p_lo)).max(0.0);
            t.e = (t.e + 0.5 * (t.flow - q) / t.g).max(0.0);
            t.out_power = q * p_lo;
            self.circuits[t.lo].heat += t.out_power;
            self.circuits[t.hi].heat -= t.out_power;
            t.s.put(slots, "flow", q);
            t.s.put(slots, "power", t.out_power);
        }
        for (k, c) in self.circuits.iter_mut().enumerate() {
            let p = self.net.potential(c.node).max(0.0);
            let in_accumulators: f64 = self.accumulators.iter().filter(|a| a.circuit == k).map(|a| a.fluid).sum();
            c.level = (c.fluid - in_accumulators - c.compliance * p).max(0.0);
            c.s.put(slots, "pressure", p);
            c.s.put(slots, "fluid", c.level);
            c.s.put(slots, "temperature", c.temp);
            c.s.flag(slots, "available", p >= c.low);
            c.s.flag(slots, "low", p < c.low);
            c.s.flag(slots, "low_level", c.level < c.low_level);
            c.s.flag(slots, "overheat", c.temp > c.overheat);
        }
        for u in self.consumers.iter() {
            let c = &self.circuits[u.circuit];
            let p = c.s.get(slots, "pressure");
            let rate = (p / c.nominal).min(1.0);
            u.s.flag(slots, "powered", p >= u.minimum && u.share > 0.0 && !u.s.on(slots, "failed"));
            u.s.put(slots, "flow", u.demand * rate);
            u.s.put(slots, "rate", if u.demand > 0.0 { rate * u.share } else { 0.0 });
        }
    }

    fn state(&self) -> Vec<f64> {
        let mut p = Packer(self.net.state(), 0);
        for c in &self.circuits {
            [c.fluid, c.level, c.temp, c.heat].into_iter().for_each(|v| p.put(v));
        }
        self.pumps.iter().for_each(|x| p.put(x.share));
        self.accumulators.iter().for_each(|a| p.put(a.fluid));
        self.consumers.iter().for_each(|u| p.put(u.share));
        for t in &self.ptus {
            [t.on as u8 as f64, t.hi as f64, t.lo as f64, t.e, t.out_power].into_iter().for_each(|v| p.put(v));
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
        for c in &mut self.circuits {
            c.fluid = p.take();
            c.level = p.take();
            c.temp = p.take();
            c.heat = p.take();
        }
        self.pumps.iter_mut().for_each(|x| x.share = p.take());
        self.accumulators.iter_mut().for_each(|a| a.fluid = p.take());
        self.consumers.iter_mut().for_each(|u| u.share = p.take());
        for t in &mut self.ptus {
            t.on = p.take() != 0.0;
            t.hi = p.take() as usize;
            t.lo = p.take() as usize;
            t.e = p.take();
            t.out_power = p.take();
        }
    }
}
