//! Cabin pressure and temperature: cabin air mass from inflow and compressible orifice outflow through the
//! outflow valves, leakage, safety valves, negative relief and any breach; a pressure controller holding the
//! cabin at the lowest altitude its differential allows, rate limited, arriving at the landing field.

use crate::util::Slots;
use crate::{expr_of, Expr};
use easyacft_core::Dim;
use easyacft_lang::schema::{opt, req, Kind, PartSpec};
use easyacft_lang::{Checked, CheckedPart, Type};

const B: Type = Type::Bool;
const N: Type = Type::Num(Dim::NONE);
const P: Type = Type::Num(Dim::PRESSURE);
const T: Type = Type::Num(Dim::TEMPERATURE);
const LEN: Type = Type::Num(Dim::LENGTH);
const SPD: Type = Type::Num(Dim::SPEED);
const MF: Type = Type::Num(Dim::MASS_FLOW);
const M: Type = Type::Num(Dim::MASS);
const WHEN: Kind = Kind::Expr(Type::Bool);
const AIR: &[&str] = &["environment.air"];

pub static SPECS: &[PartSpec] = &[PartSpec {
    domain: "cabin",
    kind: "pressurization",
    help: "Cabin pressure and temperature from pack inflow, outflow valves, leakage, safety and negative relief valves, under an automatic controller or manual.",
    fields: &[
        req("air", Kind::Ref(AIR), "outside air"),
        req("volume", Kind::Quantity(Dim::VOLUME), "pressurised volume"),
        req("inflow", Kind::Expr(MF), "pack flow, for example \"pack1.flow + pack2.flow\""),
        opt("inflow_temperature", Kind::Expr(T), "295 K", "temperature of the air the packs deliver"),
        opt("max_differential", Kind::Quantity(Dim::PRESSURE), "8.6 psi", "safety valve opening differential"),
        opt("max_operating_differential", Kind::Quantity(Dim::PRESSURE), "0 psi", "highest differential the controller holds; 0 means 0.25 psi below the safety valves"),
        opt("max_negative", Kind::Quantity(Dim::PRESSURE), "0.5 psi", "outside-over-cabin differential at which the negative relief valves open"),
        opt("cabin_ceiling", Kind::Quantity(Dim::LENGTH), "8000 ft", "cabin altitude the controller holds at the highest"),
        opt("rate", Kind::Quantity(Dim::SPEED), "2.5 m/s", "fastest cabin climb the controller schedules"),
        opt("descent_rate", Kind::Quantity(Dim::SPEED), "1.5 m/s", "fastest cabin descent the controller schedules"),
        opt("landing_elevation", Kind::Expr(LEN), "0 m", "destination field elevation the cabin descends to"),
        opt("outflow_area", Kind::Number, "0.1", "outflow valve effective area fully open, m2"),
        opt("valve_time", Kind::Quantity(Dim::TIME), "8 s", "outflow valve travel time closed to open"),
        opt("safety_area", Kind::Number, "0.06", "safety valve effective area fully open, m2"),
        opt("negative_area", Kind::Number, "0.06", "negative relief valve effective area, m2"),
        opt("leak", Kind::Number, "0.002", "leak area, m2"),
        opt("breach_area", Kind::Expr(N), "0", "a hole in the fuselage, m2: a door or window lost"),
        opt("cabin_heat", Kind::Expr(Type::Num(Dim::POWER)), "0 W", "heat from passengers, galleys and electrics"),
        opt("wall_conductance", Kind::Number, "1000", "heat lost through the fuselage per kelvin of cabin-to-outside difference, W/K"),
        opt("warning_altitude", Kind::Quantity(Dim::LENGTH), "10000 ft", "cabin altitude that counts as excessive"),
        opt("manual_when", WHEN, "false", "outflow valve in manual"),
        opt("manual_open", Kind::Expr(N), "0", "manual outflow valve opening 0 to 1"),
        opt("valve_failed_when", WHEN, "false", "outflow valve jammed where it is"),
    ],
    outputs: &[
        ("pressure", P),
        ("altitude", LEN),
        ("differential", P),
        ("vertical_speed", SPD),
        ("outflow", N),
        ("excess_altitude", B),
        ("target_altitude", LEN),
        ("temperature", T),
        ("mass", M),
        ("inflow", MF),
        ("outflow_flow", MF),
        ("leak_flow", MF),
        ("safety_valve", N),
        ("negative_relief", B),
        ("excess_differential", B),
        ("failed", B),
    ],
}];

pub const R_AIR: f64 = 287.05;
const GAMMA: f64 = 1.4;
const CP: f64 = 1005.0;
const CV: f64 = CP - R_AIR;
const PSI: f64 = 6894.757;

pub fn isa_altitude(p: f64) -> f64 {
    44_330.8 * (1.0 - (p.max(1.0) / 101_325.0).powf(0.190_263))
}

pub fn isa_pressure(h: f64) -> f64 {
    101_325.0 * (1.0 - h / 44_330.8).max(0.0).powf(5.255_88)
}

/// Mass flow through an orifice of effective area `cda` from `p_up` at `t_up` to `p_down`, compressible,
/// choked below the critical pressure ratio.
pub fn orifice(cda: f64, p_up: f64, t_up: f64, p_down: f64) -> f64 {
    if cda <= 0.0 || p_up <= p_down || p_up <= 0.0 {
        return 0.0;
    }
    let r = (p_down / p_up).max(0.0);
    let crit = (2.0 / (GAMMA + 1.0)).powf(GAMMA / (GAMMA - 1.0));
    let k = (GAMMA / (R_AIR * t_up.max(100.0))).sqrt();
    if r <= crit {
        cda * p_up * k * (2.0 / (GAMMA + 1.0)).powf((GAMMA + 1.0) / (2.0 * (GAMMA - 1.0)))
    } else {
        cda * p_up * (2.0 * GAMMA / (R_AIR * t_up.max(100.0) * (GAMMA - 1.0)) * (r.powf(2.0 / GAMMA) - r.powf((GAMMA + 1.0) / GAMMA)).max(0.0)).sqrt()
    }
}

/// Signed flow out of the cabin through `cda` (negative when the outside pushes in).
fn through(cda: f64, pc: f64, tc: f64, pa: f64, ta: f64) -> f64 {
    if pc >= pa { orifice(cda, pc, tc, pa) } else { -orifice(cda, pa, ta, pc) }
}

pub struct Cabin {
    air: [usize; 3],
    e: [Option<Expr>; 8],
    volume: f64,
    max_diff: f64,
    max_op: f64,
    max_neg: f64,
    ceiling: f64,
    climb: f64,
    descend: f64,
    outflow_area: f64,
    valve_rate: f64,
    safety_area: f64,
    negative_area: f64,
    leak: f64,
    wall: f64,
    warning: f64,
    m: f64,
    t: f64,
    target_h: f64,
    open: f64,
    integral: f64,
    last_h: f64,
    s: Slots,
}

fn slot(c: &Checked, p: &CheckedPart, field: &str, out: &str) -> Option<usize> {
    p.part(field).and_then(|i| c.slots.index.get(&format!("{}.{out}", c.parts[i].id)).copied())
}

impl Cabin {
    pub fn new(c: &Checked, p: &CheckedPart) -> Option<Self> {
        let air = [slot(c, p, "air", "pressure")?, slot(c, p, "air", "temperature")?, slot(c, p, "air", "on_ground")?];
        let max_diff = p.num("max_differential");
        let max_op = p.num("max_operating_differential");
        let volume = p.num("volume").max(1.0);
        Some(Cabin {
            air,
            e: ["inflow", "inflow_temperature", "landing_elevation", "breach_area", "cabin_heat", "manual_when", "manual_open", "valve_failed_when"].map(|k| expr_of(p, k)),
            volume,
            max_diff,
            max_op: if max_op > 0.0 { max_op.min(max_diff) } else { (max_diff - 0.25 * PSI).max(0.0) },
            max_neg: p.num("max_negative"),
            ceiling: p.num("cabin_ceiling"),
            climb: p.num("rate").max(0.01),
            descend: p.num("descent_rate").max(0.01),
            outflow_area: p.num("outflow_area").max(0.0),
            valve_rate: 1.0 / p.num("valve_time").max(0.1),
            safety_area: p.num("safety_area").max(0.0),
            negative_area: p.num("negative_area").max(0.0),
            leak: p.num("leak").max(0.0),
            wall: p.num("wall_conductance").max(0.0),
            warning: p.num("warning_altitude"),
            m: 101_325.0 * volume / (R_AIR * 295.0),
            t: 295.0,
            target_h: 0.0,
            open: 1.0,
            integral: 0.0,
            last_h: 0.0,
            s: Slots::of(p, &["pressure", "altitude", "differential", "vertical_speed", "outflow", "excess_altitude", "target_altitude", "temperature", "mass", "inflow", "outflow_flow", "leak_flow", "safety_valve", "negative_relief", "excess_differential", "failed"]),
        })
    }

    pub fn step(&mut self, slots: &mut [f64], dt: f64) {
        let ev = |e: &mut Option<Expr>, slots: &[f64]| e.as_mut().map_or(0.0, |x| x.eval(slots, dt));
        let (pa, ta, ground) = (slots[self.air[0]].max(1.0), slots[self.air[1]].max(150.0), slots[self.air[2]] != 0.0);
        let inflow = ev(&mut self.e[0], slots).max(0.0);
        let t_in = { let v = ev(&mut self.e[1], slots); if v > 100.0 { v } else { 295.0 } };
        let landing = ev(&mut self.e[2], slots);
        let breach = ev(&mut self.e[3], slots).max(0.0);
        let heat = ev(&mut self.e[4], slots);
        let manual = ev(&mut self.e[5], slots) != 0.0;
        let manual_open = ev(&mut self.e[6], slots).clamp(0.0, 1.0);
        let jammed = ev(&mut self.e[7], slots) != 0.0 || self.s.on(slots, "failed");
        let pc = self.m * R_AIR * self.t / self.volume;
        let h_out = isa_altitude(pa);
        let h_lowest = isa_altitude(pa + self.max_op);
        let goal = if ground { h_out } else { h_lowest.max(landing.min(self.ceiling)).min(self.ceiling.max(h_lowest)) };
        let step = if goal > self.target_h { self.climb * dt } else { self.descend * dt };
        self.target_h += (goal - self.target_h).clamp(-step, step);
        if !ground {
            self.target_h = self.target_h.max(h_lowest);
        }
        let p_target = isa_pressure(self.target_h).min(pa + self.max_op);
        let demand = if manual {
            manual_open
        } else if ground {
            1.0
        } else {
            let err = (pc - p_target) / 1000.0;
            self.integral = (self.integral + err * dt * 0.05).clamp(0.0, 1.0);
            (0.15 * err + self.integral).clamp(0.0, 1.0)
        };
        if !jammed {
            self.open += (demand - self.open).clamp(-self.valve_rate * dt, self.valve_rate * dt);
        }
        let diff = pc - pa;
        let safety = ((diff - self.max_diff) / (0.25 * PSI)).clamp(0.0, 1.0);
        let negative = -diff > self.max_neg;
        let out_valve = through(self.open * self.outflow_area, pc, self.t, pa, ta);
        let leak = through(self.leak, pc, self.t, pa, ta);
        let relief = through(safety * self.safety_area, pc, self.t, pa, ta).max(0.0);
        let neg_in = if negative { through(self.negative_area, pc, self.t, pa, ta).min(0.0) } else { 0.0 };
        let hole = through(breach, pc, self.t, pa, ta);
        let net_out = out_valve + leak + relief + neg_in + hole;
        let inward = (-out_valve).max(0.0) + (-leak).max(0.0) + (-neg_in) + (-hole).max(0.0);
        // Energy: inflow and inward leakage bring their enthalpy, outflow carries the cabin's, plus heat sources and wall loss.
        let h_in = inflow * CP * t_in + inward * CP * ta;
        let h_out_flow = (net_out + inward) * CP * self.t;
        let wall = self.wall * (self.t - ta);
        let energy = self.m * CV * self.t + (h_in - h_out_flow + heat - wall) * dt;
        self.m = (self.m + (inflow - net_out) * dt).max(1e-3);
        self.t = (energy / (self.m * CV)).clamp(150.0, 400.0);
        let pc = self.m * R_AIR * self.t / self.volume;
        let h = isa_altitude(pc);
        let vs = (h - self.last_h) / dt.max(1e-6);
        self.last_h = h;
        let s = &self.s;
        s.put(slots, "pressure", pc);
        s.put(slots, "altitude", h);
        s.put(slots, "differential", pc - pa);
        s.put(slots, "vertical_speed", vs);
        s.put(slots, "outflow", self.open);
        s.flag(slots, "excess_altitude", h > self.warning);
        s.put(slots, "target_altitude", self.target_h);
        s.put(slots, "temperature", self.t);
        s.put(slots, "mass", self.m);
        s.put(slots, "inflow", inflow);
        s.put(slots, "outflow_flow", out_valve);
        s.put(slots, "leak_flow", leak + hole);
        s.put(slots, "safety_valve", safety);
        s.flag(slots, "negative_relief", negative);
        s.flag(slots, "excess_differential", pc - pa > self.max_diff);
    }

    pub fn state(&self, v: &mut Vec<f64>) {
        v.extend([self.m, self.t, self.target_h, self.open, self.integral, self.last_h]);
    }

    pub fn restore(&mut self, it: &mut impl Iterator<Item = f64>) {
        for x in [&mut self.m, &mut self.t, &mut self.target_h, &mut self.open, &mut self.integral, &mut self.last_h] {
            *x = it.next().unwrap_or(*x);
        }
    }
}
