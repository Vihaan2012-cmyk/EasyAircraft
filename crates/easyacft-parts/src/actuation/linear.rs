//! A linear hydraulic actuator at component level: two chambers whose pressures follow fluid compressibility
//! (with entrained air) as the piston moves and the valves meter flow; a servo valve, selector valve or
//! electro-hydrostatic pump driving them; a solenoid mode valve that drops to damping, blocked, check or float;
//! relief and anti-cavitation valves; internal and external seal leakage that thin with temperature; seal friction
//! that grows with pressure (Stribeck); stroke end stops with cushions; and the actuator's own heat. The mechanism
//! it drives owns the motion; this computes force, chamber pressure rates and port flows.

use super::fluid::{Fluid, Leak, P_ATM};
use super::ode::{soft_sign, smooth_step};
use super::valve::{Centre, Oil, PortFlows, Refill, Relief, Selector, ServoValve, SpoolFault, CD};
use crate::util::Slots;
use crate::{expr_of, Expr};
use easyacft_core::Dim;
use easyacft_lang::schema::{opt, Kind, PartSpec};
use easyacft_lang::{Checked, CheckedPart, Type};

const B: Type = Type::Bool;
const N: Type = Type::Num(Dim::NONE);
const P: Type = Type::Num(Dim::PRESSURE);
const T: Type = Type::Num(Dim::TEMPERATURE);
const VF: Type = Type::Num(Dim::VOLUME_FLOW);
const VOL: Type = Type::Num(Dim::VOLUME);
const WHEN: Kind = Kind::Expr(Type::Bool);
pub(crate) const CIRCUIT: &[&str] = &["hydraulic.circuit"];
const PSI: f64 = 6_894.757;
/// States each actuator adds to its mechanism: chamber A and B pressure, valve position, spool velocity, pump speed.
pub const STATES: usize = 5;

pub static SPEC: PartSpec = PartSpec {
    domain: "actuator",
    kind: "linear",
    help: "A hydraulic linear actuator: servo valve, selector valve, electro-hydrostatic (EHA) or electrical backup (EBHA); chambers, mode valve, relief and anti-cavitation valves, seal friction and leakage, end stops. A flight control surface, gear leg, door or steering unit lists it; on its own it drives a mass.",
    fields: &[
        opt("circuit", Kind::Ref(CIRCUIT), "", "hydraulic circuit that powers it; none for an EHA"),
        opt("control", Kind::Text, "servo", "servo, selector, eha or ebha"),
        opt("force", Kind::Quantity(Dim::FORCE), "50 kN", "stall force at the circuit's nominal pressure"),
        opt("rate", Kind::Quantity(Dim::SPEED), "0.1 m/s", "no-load rate with the valve wide open at nominal pressure"),
        opt("stroke", Kind::Quantity(Dim::LENGTH), "0 m", "full stroke; 0 sizes it from the mechanism's travel"),
        opt("rod", Kind::Number, "0", "rod area share of the piston: 0 for a balanced (through-rod) actuator, 0.3 to 0.5 for a gear jack"),
        opt("command", Kind::Expr(N), "0.5", "on its own: wanted extension, 0 retracted to 1 extended; selector actuators: 1 extend, -1 retract, 0 centre"),
        opt("active_when", WHEN, "true", "the actuator's computer engages it: mode valve solenoid energised"),
        opt("passive", Kind::Text, "damping", "mode when not active: damping, blocked, check (holds against extension, as spoilers) or float"),
        opt("damping_rate", Kind::Quantity(Dim::SPEED), "0 m/s", "rate the piston moves at stall force in damping mode; 0 means six times the rate, so a standby actuator costs the active one about a sixth of its force at full rate"),
        opt("bandwidth", Kind::Number, "15", "position loop bandwidth, rad/s"),
        opt("valve_bandwidth", Kind::Number, "80", "servo valve spool bandwidth, Hz"),
        opt("null_leak", Kind::Quantity(Dim::VOLUME_FLOW), "0.6 L/min", "servo valve leakage at null at nominal pressure"),
        opt("seal_leak", Kind::Quantity(Dim::VOLUME_FLOW), "0.02 L/min", "piston seal leakage at nominal pressure difference and 40 C"),
        opt("friction", Kind::Number, "0.01", "seal friction at nominal pressure, share of stall force"),
        opt("relief", Kind::Quantity(Dim::PRESSURE), "0 Pa", "chamber relief valves crack at this; 0 means 1.35 times nominal"),
        opt("switch_time", Kind::Quantity(Dim::TIME), "0.04 s", "mode or selector valve switching time"),
        opt("fluid", Kind::Text, "phosphate_ester", "phosphate_ester, mineral, synthetic or low_temperature; an EHA's own fluid"),
        opt("motor_power", Kind::Quantity(Dim::POWER), "0 W", "EHA or EBHA motor power; 0 sizes it from force and rate"),
        opt("powered_when", WHEN, "true", "EHA or EBHA motor has its electrical supply"),
        opt("backup_when", WHEN, "true", "EBHA: switch to its electrical backup when the circuit is lost"),
        opt("heat_capacity", Kind::Number, "0", "J per K of the actuator body; 0 sizes it from force"),
        opt("overheat_temperature", Kind::Quantity(Dim::TEMPERATURE), "393.15 K", "body overheat; an EHA stops past it"),
        opt("winding_limit", Kind::Quantity(Dim::TEMPERATURE), "453.15 K", "EHA motor winding temperature its drive cuts out at"),
        opt("ambient", Kind::Expr(T), "288.15 K", "temperature around it"),
        opt("mass", Kind::Quantity(Dim::MASS), "20 kg", "on its own: moving mass"),
        opt("load", Kind::Expr(Type::Num(Dim::FORCE)), "0 N", "on its own: external force, positive resisting extension"),
        opt("jam_when", WHEN, "false", "piston seized"),
        opt("runaway_when", WHEN, "false", "servo valve hard over to extend"),
        opt("valve_jam_when", WHEN, "false", "servo valve spool seized where it is"),
        opt("disconnect_when", WHEN, "false", "rod end or attachment broken: transmits no force"),
        opt("seal_failed_when", WHEN, "false", "piston seal blown: chambers bypass each other"),
        opt("external_leak_when", WHEN, "false", "a leak to outside; the circuit loses fluid"),
        opt("lvdt_1_fault", Kind::Expr(N), "0", "position transducer channel 1: 0 healthy, 1 open primary, 2 open secondary, 3 core detached, 4 drifting"),
        opt("lvdt_2_fault", Kind::Expr(N), "0", "channel 2, the same codes"),
        opt("voltage", Kind::Expr(Type::Num(Dim::VOLTAGE)), "0 V", "EHA or EBHA motor drive supply; 0 means rated"),
        opt("rated_voltage", Kind::Quantity(Dim::VOLTAGE), "540 V", "drive supply the motor's torque is rated at"),
        opt("cushion", Kind::Number, "-1", "stroke-end snubbing length, share of stroke; -1 means 0.05 for selector actuators and none for servos"),
        opt("seal_life", Kind::Quantity(Dim::LENGTH), "400000 m", "piston travel after which worn seals leak about thirty times as much"),
        opt("seal_wear", Kind::Number, "0", "seal wear at start, share of its life"),
        opt("line_length", Kind::Quantity(Dim::LENGTH), "0 m", "supply and return line run from the circuit's manifold; long wing and tail runs lose pressure, more so cold"),
        opt("line_bore", Kind::Quantity(Dim::LENGTH), "10 mm", "line inside diameter"),
        opt("oscillation_when", WHEN, "false", "oscillatory failure: a coil or connector fault adds a 3 Hz current to the servo valve"),
    ],
    outputs: &[
        ("position", N),
        ("extension", Type::Num(Dim::LENGTH)),
        ("velocity", Type::Num(Dim::SPEED)),
        ("force", Type::Num(Dim::FORCE)),
        ("pressure_a", P),
        ("pressure_b", P),
        ("valve", N),
        ("mode", N),
        ("active", B),
        ("circuit_flow", VF),
        ("circuit_held", VOL),
        ("electric_power", Type::Num(Dim::POWER)),
        ("temperature", T),
        ("overheat", B),
        ("cavitating", B),
        ("stalled", B),
        ("relieving", B),
        ("lvdt_1", Type::Num(Dim::LENGTH)),
        ("lvdt_2", Type::Num(Dim::LENGTH)),
        ("lvdt_disagree", B),
        ("current", Type::Num(Dim::CURRENT)),
        ("seal_wear", N),
        ("line_drop", P),
        ("winding_temperature", T),
        ("failed", B),
    ],
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Control {
    Servo,
    Selector,
    Eha,
    Ebha,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Passive {
    Damping,
    Blocked,
    Check,
    Float,
}

/// What the mode valve is doing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Mode {
    Active,
    Passive,
    /// EBHA on its electric backup.
    Backup,
}

/// The electro-hydrostatic power pack: a variable-speed motor driving a fixed-displacement reversible pump.
#[derive(Clone, Copy, Debug)]
pub struct Pump {
    /// Displacement per radian.
    pub disp: f64,
    pub w_max: f64,
    pub t_max: f64,
    pub inertia: f64,
    pub vol_eff: f64,
    pub mech_eff: f64,
    pub motor_eff: f64,
    /// Speed loop gain, N*m per rad/s.
    pub kw: f64,
    /// Local reservoir (compensator) pressure.
    pub p_comp: f64,
}

/// Everything held fixed over one mechanism step.
#[derive(Clone, Copy, Debug)]
pub struct Io {
    pub ps: f64,
    pub pr: f64,
    pub oil: Oil,
    pub mu: f64,
    pub beta_t: f64,
    /// Mode valve: 1 active, 0 passive.
    pub mode: f64,
    pub backup: bool,
    /// Position the loop is closing on, share of stroke; or the selector command.
    pub cmd: f64,
    pub electric: bool,
    pub jammed: bool,
    pub disconnected: bool,
    pub seal_failed: bool,
    pub ext_leak: bool,
    /// What the position transducer adds to the true extension, which the loop then closes on.
    pub sensor_error: f64,
    /// Seal wear share: internal leakage and friction grow with it.
    pub wear: f64,
    /// Motor drive supply share of rated.
    pub volts: f64,
    /// Oscillatory failure current added to the servo valve input.
    pub dither: f64,
}

/// What an actuator gives back from one evaluation.
#[derive(Clone, Copy, Debug, Default)]
pub struct Out {
    pub force: f64,
    pub supply: f64,
    pub ret: f64,
    pub ext_leak: f64,
    pub heat: f64,
    pub electric: f64,
    pub relieving: bool,
}

#[derive(Clone, Debug)]
pub struct Linear {
    pub control: Control,
    pub passive: Passive,
    pub area_a: f64,
    pub area_b: f64,
    pub stroke: f64,
    dead_a: f64,
    dead_b: f64,
    pub stall: f64,
    pub rate: f64,
    pub servo: ServoValve,
    pub selector: Selector,
    pub pump: Option<Pump>,
    damp_k: f64,
    check_area: f64,
    seal: Leak,
    ext: Leak,
    relief: Relief,
    refill: Refill,
    fric_c: f64,
    fric_p: f64,
    gain: f64,
    switch_time: f64,
    pub fluid: Fluid,
    pub nominal: f64,
    pub heat_cap: f64,
    pub overheat: f64,
    pub mass: f64,
    cushion: f64,
    pub seal_life: f64,
}

/// Reference viscosity the seal leakage is rated at, 40 C phosphate ester.
fn mu_ref(f: &Fluid) -> f64 {
    f.dynamic(313.15, 0.0)
}

impl Linear {
    /// Sizes an actuator from its part, a nominal supply pressure and the stroke its mechanism needs.
    pub fn from_part(p: &CheckedPart, nominal: f64, stroke_needed: f64) -> Self {
        let control = match p.text("control").to_ascii_lowercase().as_str() {
            "selector" => Control::Selector,
            "eha" => Control::Eha,
            "ebha" => Control::Ebha,
            _ => Control::Servo,
        };
        let passive = match p.text("passive").to_ascii_lowercase().as_str() {
            "blocked" => Passive::Blocked,
            "check" => Passive::Check,
            "float" => Passive::Float,
            _ => Passive::Damping,
        };
        let fluid = Fluid::parse(p.text("fluid"));
        let rho = fluid.density(313.15, 0.0);
        let nominal = if nominal > 1e5 { nominal } else { 3000.0 * PSI };
        let pr = 50.0 * PSI + P_ATM;
        let stall = p.num("force").max(10.0);
        let area_a = stall / (nominal - pr).max(1e5);
        let rod = p.num("rod").clamp(0.0, 0.8);
        let area_b = area_a * (1.0 - rod);
        let stroke = if p.num("stroke") > 0.0 { p.num("stroke") } else { stroke_needed.max(0.005) };
        let rate = p.num("rate").max(1e-4);
        let swept = area_a * stroke;
        // Valve sized so the full-open no-load rate comes out at the rated rate across both metering edges.
        let q_noload = area_a * rate;
        let q_rated = q_noload / ((nominal - pr) / (1000.0 * PSI)).sqrt().max(0.1);
        let servo = ServoValve::rated(q_rated, p.num("null_leak"), nominal, rho, p.num("valve_bandwidth"));
        let switch_time = p.num("switch_time").max(0.005);
        let selector = Selector::rated(q_noload, 0.5 * (nominal - pr), rho, if passive == Passive::Float { Centre::Float } else { Centre::Closed }, switch_time);
        let pump = matches!(control, Control::Eha | Control::Ebha).then(|| {
            let w_max = 2.0 * std::f64::consts::PI * 12_000.0 / 60.0;
            let disp = q_noload / w_max / 0.92;
            let given = p.num("motor_power");
            let power = if given > 0.0 { given } else { stall * rate * 0.5 };
            let t_max = (disp * (nominal - pr) / 0.9).max(power / (0.3 * w_max));
            let inertia = 2e-4 * (t_max / 5.0).max(0.2);
            Pump { disp, w_max, t_max, inertia, vol_eff: 0.92, mech_eff: 0.9, motor_eff: 0.9, kw: t_max / (0.05 * w_max), p_comp: 3.5e5 }
        });
        // Damping restrictor: laminar, so it damps small flutter motions as well as large ones.
        let damp_v = if p.num("damping_rate") > 0.0 { p.num("damping_rate") } else { 6.0 * rate };
        let damp_k = area_a * area_a * damp_v / stall;
        let mu = mu_ref(&fluid);
        let relief = if p.num("relief") > 0.0 { p.num("relief") } else { 1.35 * nominal };
        let given_cap = p.num("heat_capacity");
        Linear {
            control,
            passive,
            area_a,
            area_b,
            stroke,
            dead_a: 0.15 * swept + 2e-5,
            dead_b: 0.15 * swept + 2e-5,
            stall,
            rate,
            servo,
            selector,
            pump,
            damp_k,
            check_area: super::fluid::area_for(CD, q_noload.max(1e-6), 0.2 * nominal, rho),
            seal: Leak::rated(p.num("seal_leak"), nominal - pr, mu),
            ext: Leak::rated(2e-3 / 60.0, nominal, mu),
            relief: Relief::sized(relief, q_noload.max(1e-5), rho),
            refill: Refill { area: 2.0 * selector.area.max(servo.area) },
            fric_c: 0.3 * p.num("friction").max(0.0) * stall,
            fric_p: 0.7 * p.num("friction").max(0.0) * stall / (2.0 * nominal),
            gain: p.num("bandwidth").max(0.5) * stroke / rate,
            switch_time,
            fluid,
            nominal,
            heat_cap: if given_cap > 0.0 { given_cap } else { 900.0 * (stall / 2000.0).max(2.0) },
            overheat: p.num("overheat_temperature"),
            mass: p.num("mass").max(0.1),
            cushion: if p.num("cushion") >= 0.0 { p.num("cushion") } else if control == Control::Selector { 0.05 } else { 0.0 },
            seal_life: p.num("seal_life").max(1.0),
        }
    }

    pub fn switch_time(&self) -> f64 {
        self.switch_time
    }

    /// Initial state: both chambers at return pressure plus what holds the piston still, valve at null.
    pub fn initial(&self, pr: f64, s: &mut [f64]) {
        s[0] = pr;
        s[1] = pr * self.area_a / self.area_b.max(1e-9);
        s[2] = 0.0;
        s[3] = 0.0;
        s[4] = 0.0;
    }

    /// Typical size of each state, for error control.
    pub fn scales(&self, s: &mut [f64]) {
        s[0] = 5_000.0;
        s[1] = 5_000.0;
        s[2] = 0.03;
        s[3] = 5.0;
        s[4] = self.pump.map_or(1.0, |p| 0.01 * p.w_max);
    }

    /// Force from the piston bottoming at either end of its stroke, `m` the mass it brings to rest: a stiff,
    /// critically damped contact, smooth at the touch so the stepper sees no corner.
    pub fn stroke_stop(&self, x: f64, v: f64, m: f64) -> f64 {
        let w = 1e-4 * self.stroke.max(1e-3);
        let k = 100.0 * self.stall / self.stroke.max(1e-3);
        let c = 1.4 * (k * m.max(1e-3)).sqrt();
        let pen_in = super::ode::soft_pos(-x, w);
        let pen_out = super::ode::soft_pos(x - self.stroke, w);
        let touch_in = super::ode::smooth_step(-x, 0.0, 2.0 * w);
        let touch_out = super::ode::smooth_step(x - self.stroke, 0.0, 2.0 * w);
        k * (pen_in - pen_out) - c * v * (touch_in + touch_out)
    }

    /// Fluid held in the chambers beyond a centred piston: what the circuit's reservoir gives up while it extends.
    pub fn held(&self, x: f64) -> f64 {
        let mid = 0.5 * self.stroke;
        (self.area_a - self.area_b) * (x - mid)
    }

    /// Chamber pressure rates, valve rates and the piston force with the piston at extension `x` moving at `v`.
    pub fn eval(&self, io: &Io, x: f64, v: f64, s: &[f64], d: &mut [f64]) -> Out {
        let (pa, pb) = (s[0].max(self.fluid.vapour()), s[1].max(self.fluid.vapour()));
        let (xv, vv, w) = (s[2], s[3], s[4]);
        let oil = io.oil;
        let mut out = Out::default();
        let mut qa = 0.0;
        let mut qb = 0.0;
        let err = io.cmd - (x + io.sensor_error) / self.stroke;
        let servo_on = matches!(self.control, Control::Servo | Control::Ebha) && !io.backup;
        // Valve.
        match self.control {
            Control::Selector => {
                d[2] = self.selector.rate(io.cmd, xv);
                d[3] = -vv * 50.0;
                if io.mode > 0.0 {
                    let f = self.selector.flows(xv, io.ps, io.pr, pa, pb, oil);
                    qa += f.a * io.mode;
                    qb += f.b * io.mode;
                    out.supply += f.supply * io.mode;
                    out.ret += f.ret * io.mode;
                }
            }
            _ if servo_on => {
                let u = (self.gain * err + io.dither).clamp(-1.0, 1.0);
                d[2] = vv;
                d[3] = self.servo.accel(u, xv, vv);
                if io.mode > 0.0 {
                    let f: PortFlows = self.servo.flows(xv, io.ps, io.pr, pa, pb, oil);
                    qa += f.a * io.mode;
                    qb += f.b * io.mode;
                    out.supply += f.supply * io.mode;
                    out.ret += f.ret * io.mode;
                }
            }
            _ => {
                d[2] = -xv * 20.0;
                d[3] = -vv * 50.0;
            }
        }
        // Electro-hydrostatic pump: speed loop on the position error, motor torque against the pump's pressure torque.
        if let Some(pm) = self.pump.filter(|_| matches!(self.control, Control::Eha) || io.backup) {
            let on = io.electric && io.mode > 0.0;
            let w_cmd = if on { (self.gain * err).clamp(-1.0, 1.0) * pm.w_max * io.volts.min(1.0) } else { 0.0 };
            let t_lim = pm.t_max * io.volts.clamp(0.0, 1.2);
            let t_motor = if on { (pm.kw * (w_cmd - w)).clamp(-t_lim, t_lim) } else { 0.0 };
            let dp = pa - pb;
            let t_pump = pm.disp * dp / pm.mech_eff;
            d[4] = (t_motor - t_pump - 1e-3 * pm.t_max * soft_sign(w, 5.0) - 2e-4 * w) / pm.inertia;
            let q = pm.disp * w;
            let slip = (1.0 - pm.vol_eff) * pm.disp * pm.w_max * dp / self.nominal.max(1.0);
            qa += q - slip;
            qb -= q - slip;
            // Compensator keeps both sides charged above its own pressure.
            qa += self.refill.flow(pa, pm.p_comp, oil);
            qb += self.refill.flow(pb, pm.p_comp, oil);
            let mech = (t_motor * w).abs();
            out.electric = if on { mech / pm.motor_eff + 0.05 * pm.t_max * pm.w_max * 0.02 } else { 0.0 };
            out.heat += out.electric - (q * dp).abs().min(out.electric);
        } else {
            d[4] = -w * 20.0;
        }
        // Passive mode share.
        let passive = 1.0 - io.mode;
        if passive > 0.0 && !io.backup {
            match self.passive {
                Passive::Damping => {
                    let q = self.damp_k * (pa - pb) * passive;
                    qa -= q;
                    qb += q;
                }
                Passive::Blocked => {}
                Passive::Check => {
                    qa -= super::valve::check(self.check_area, 20.0 * PSI, pa, io.pr, oil) * passive;
                }
                Passive::Float => {
                    qa -= oil.q(self.selector.area, pa - io.pr) * passive;
                    qb -= oil.q(self.selector.area, pb - io.pr) * passive;
                }
            }
        }
        let pr_local = self.pump.filter(|_| matches!(self.control, Control::Eha) || io.backup).map_or(io.pr, |p| p.p_comp);
        // Anti-cavitation refill and chamber relief, both to return.
        let ra = self.refill.flow(pa, pr_local, oil);
        let rb = self.refill.flow(pb, pr_local, oil);
        let la = self.relief.flow(pa, pr_local, oil);
        let lb = self.relief.flow(pb, pr_local, oil);
        qa += ra - la;
        qb += rb - lb;
        out.relieving = la > 1e-6 || lb > 1e-6;
        // Seal leakage across the piston, thinning as the fluid warms.
        let mut seal = self.seal.flow(pa - pb, io.mu) * (1.0 + 30.0 * io.wear * io.wear);
        if io.seal_failed {
            seal += oil.q(0.5 * self.check_area, pa - pb);
        }
        qa -= seal;
        qb += seal;
        if io.ext_leak {
            let l = self.ext.flow(pa.max(pb) - P_ATM, io.mu) * 100.0;
            qa -= l * 0.5;
            qb -= l * 0.5;
            out.ext_leak = l;
        }
        if !self.pump.is_some_and(|_| matches!(self.control, Control::Eha) || io.backup) {
            out.ret += la + lb - ra - rb;
        }
        let va = self.dead_a + self.area_a * x.clamp(0.0, self.stroke);
        let vb = self.dead_b + self.area_b * (self.stroke - x).clamp(0.0, self.stroke);
        let piston_v = if io.jammed { 0.0 } else { v };
        // Below air release the chamber is a froth that fills without pressure: hold the modulus at its value there.
        let release = self.fluid.air_release();
        d[0] = self.fluid.bulk(io.beta_t, pa.max(release)) / va * (qa - self.area_a * piston_v);
        d[1] = self.fluid.bulk(io.beta_t, pb.max(release)) / vb * (qb + self.area_b * piston_v);
        // Hold chambers off the vapour floor without a kink.
        let floor = self.fluid.vapour();
        if s[0] < floor * 2.0 && d[0] < 0.0 {
            d[0] *= smooth_step(s[0], floor * 1.5, floor);
        }
        if s[1] < floor * 2.0 && d[1] < 0.0 {
            d[1] *= smooth_step(s[1], floor * 1.5, floor);
        }
        // Seal friction rises with chamber pressure; a Stribeck peak at breakaway.
        let fc = (self.fric_c + self.fric_p * (pa + pb)) * (1.0 + 0.5 * io.wear);
        let fric = (fc + 0.5 * fc * (-(v / 0.002).powi(2)).exp()) * soft_sign(v, 5e-4) + 0.002 * self.stall / self.rate.max(1e-3) * v;
        let hydraulic = pa * self.area_a - pb * self.area_b;
        // Snubbers: the last part of the stroke meters the outgoing fluid through a small orifice.
        let mut snub = 0.0;
        if self.cushion > 0.0 {
            let len = self.cushion * self.stroke;
            let c = self.stall / (0.5 * self.rate);
            let near_in = ((len - x) / len).clamp(0.0, 1.0);
            let near_out = ((x - (self.stroke - len)) / len).clamp(0.0, 1.0);
            snub = c * (near_in * super::ode::soft_pos(-v, 1e-3) - near_out * super::ode::soft_pos(v, 1e-3));
        }
        out.force = if io.disconnected { 0.0 } else { hydraulic - fric + snub };
        // Throttling heat: hydraulic power taken less the mechanical work done on the load.
        out.heat += (out.supply * (io.ps - io.pr)).max(0.0) - (hydraulic * v).max(0.0).min((out.supply * (io.ps - io.pr)).max(0.0));
        out
    }
}

/// Mode valve and EBHA logic: which mode the actuator should be in, and the mode valve's share toward it.
#[derive(Clone, Debug)]
pub struct ModeLogic {
    pub share: f64,
    pub mode: Mode,
}

impl ModeLogic {
    pub fn update(&mut self, a: &Linear, active: bool, circuit_ok: bool, backup_ok: bool, dt: f64) {
        let hyd_active = active && circuit_ok;
        let backup = a.control == Control::Ebha && active && !circuit_ok && backup_ok;
        self.mode = if a.control == Control::Eha {
            if active && backup_ok {
                Mode::Active
            } else {
                Mode::Passive
            }
        } else if hyd_active {
            Mode::Active
        } else if backup {
            Mode::Backup
        } else {
            Mode::Passive
        };
        let want = if self.mode == Mode::Passive { 0.0 } else { 1.0 };
        let rate = dt / a.switch_time;
        self.share += (want - self.share).clamp(-rate, rate);
    }
}

/// A standalone actuator part: its configuration, expressions and the mode state it carries between steps.
pub struct Part {
    pub act: Linear,
    pub circuit: Option<(usize, usize, usize)>,
    pub e: [Option<Expr>; 11],
    pub ambient: Option<Expr>,
    pub mode: ModeLogic,
    pub temp: f64,
    pub index: usize,
    pub s: Slots,
    pub lost: f64,
    pub lvdt: super::sensors::Dual,
    sensor_error: f64,
    pub wear: f64,
    sensors: [Option<Expr>; 4],
    clock: f64,
    line: (f64, f64),
    line_drop: f64,
    last_supply: f64,
    winding: f64,
    winding_limit: f64,
    tripped: bool,
    rated_v: f64,
    volts: f64,
}

pub const EXPRS: [&str; 11] = ["command", "active_when", "powered_when", "backup_when", "load", "jam_when", "runaway_when", "valve_jam_when", "disconnect_when", "seal_failed_when", "external_leak_when"];

impl Part {
    pub fn new(c: &Checked, p: &CheckedPart, index: usize, stroke_needed: f64) -> Self {
        let circuit = p.part("circuit").and_then(|i| {
            let id = &c.parts[i].id;
            let g = |o: &str| c.slots.index.get(&format!("{id}.{o}")).copied();
            Some((g("pressure")?, g("temperature")?, g("return_pressure")?))
        });
        let nominal = p.part("circuit").map_or(0.0, |i| c.parts[i].num("nominal"));
        let act = Linear::from_part(p, nominal, stroke_needed);
        let stroke = act.stroke;
        Part {
            act,
            circuit,
            e: EXPRS.map(|k| expr_of(p, k)),
            ambient: expr_of(p, "ambient"),
            mode: ModeLogic { share: 0.0, mode: Mode::Passive },
            temp: f64::NAN,
            index,
            s: Slots::of(p, &["position", "extension", "velocity", "force", "pressure_a", "pressure_b", "valve", "mode", "active", "circuit_flow", "circuit_held", "electric_power", "temperature", "overheat", "cavitating", "stalled", "relieving", "lvdt_1", "lvdt_2", "lvdt_disagree", "current", "seal_wear", "line_drop", "winding_temperature", "failed"]),
            lost: 0.0,
            lvdt: super::sensors::Dual::new(0.0, stroke, (p.slot as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15)),
            sensor_error: 0.0,
            wear: p.num("seal_wear").clamp(0.0, 1.0),
            sensors: ["lvdt_1_fault", "lvdt_2_fault", "voltage", "oscillation_when"].map(|k| expr_of(p, k)),
            clock: 0.0,
            line: (p.num("line_length").max(0.0), p.num("line_bore").max(1e-3)),
            line_drop: 0.0,
            last_supply: 0.0,
            winding: f64::NAN,
            winding_limit: p.num("winding_limit"),
            tripped: false,
            rated_v: p.num("rated_voltage").max(1.0),
            volts: 1.0,
        }
    }

    /// Re-sizes the actuator for the stroke its mechanism needs, transducers included.
    pub fn resize(&mut self, p: &CheckedPart, stroke: f64) {
        self.act = Linear::from_part(p, self.act.nominal, stroke);
        self.lvdt = super::sensors::Dual::new(0.0, self.act.stroke, (p.slot as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15));
    }

    /// Supply and return pressures, the oil state at the fluid's temperature, and whether the circuit is up.
    pub fn supply(&self, slots: &[f64]) -> (f64, f64, f64, bool) {
        match self.circuit {
            Some((p, t, r)) => {
                let pr = slots[r].max(P_ATM);
                let ps = slots[p].max(0.0) + pr;
                (ps, pr, slots[t].max(150.0), slots[p] > 0.5 * self.act.nominal)
            }
            None => (P_ATM, P_ATM, if self.temp.is_finite() { self.temp } else { 288.15 }, false),
        }
    }

    pub fn ev(&mut self, k: usize, slots: &[f64], dt: f64, or: f64) -> f64 {
        self.e[k].as_mut().map_or(or, |x| x.eval(slots, dt))
    }

    pub fn yes(&mut self, k: usize, slots: &[f64], dt: f64, or: bool) -> bool {
        self.e[k].as_mut().map_or(or, |x| x.yes(slots, dt))
    }

    /// Reads the expressions, moves the mode valve and builds the fixed inputs for this step; `cmd` overrides
    /// the actuator's own command when a mechanism closes the loop.
    pub fn prepare(&mut self, slots: &[f64], dt: f64, cmd: Option<f64>) -> Io {
        let (ps0, pr0, temp, ok) = self.supply(slots);
        // Line losses on last step's flow: supply arrives low, return backs up; both share the run.
        let (ps, pr) = if self.line.0 > 0.0 && self.circuit.is_some() {
            let rho = self.act.fluid.density(temp, ps0);
            let nu = self.act.fluid.kinematic(temp);
            let drop = super::fluid::pipe_drop(self.last_supply, self.line.0, self.line.1, rho, nu).clamp(0.0, 0.45 * (ps0 - pr0).max(0.0));
            let filtered = self.line_drop + (drop - self.line_drop) * (dt / 0.02).min(1.0);
            self.line_drop = filtered;
            (ps0 - filtered, pr0 + filtered)
        } else {
            (ps0, pr0)
        };
        if !self.temp.is_finite() {
            self.temp = temp;
        }
        let own = self.ev(0, slots, dt, 0.5);
        let active = self.yes(1, slots, dt, true) && !self.s.on(slots, "failed");
        let v = self.sensors[2].as_mut().map_or(0.0, |e| e.eval(slots, dt));
        self.volts = if v <= 0.0 && self.sensors[2].as_ref().is_some_and(|e| e.program.constant().is_some()) { 1.0 } else { (v / self.rated_v).max(0.0) };
        // A drive whose supply has sagged below a third of rated trips off.
        // The drive cuts out on a hot winding and stays out until it has cooled 30 K.
        if self.winding > self.winding_limit {
            self.tripped = true;
        } else if self.winding < self.winding_limit - 30.0 {
            self.tripped = false;
        }
        let powered = self.yes(2, slots, dt, true) && !(self.act.pump.is_some() && (self.temp > self.act.overheat + 15.0 || self.tripped)) && (self.act.pump.is_none() || self.volts > 0.3);
        if !powered {
            self.volts = 0.0;
        }
        let backup_ok = self.yes(3, slots, dt, true) && powered;
        self.clock += dt;
        let osc = self.sensors[3].as_mut().is_some_and(|e| e.yes(slots, dt));
        let dither = if osc { 0.3 * (std::f64::consts::TAU * 3.0 * self.clock).sin() } else { 0.0 };
        let jammed = self.yes(5, slots, dt, false);
        let runaway = self.yes(6, slots, dt, false);
        let valve_jam = self.yes(7, slots, dt, false);
        let disconnected = self.yes(8, slots, dt, false);
        let seal_failed = self.yes(9, slots, dt, false);
        let ext_leak = self.yes(10, slots, dt, false);
        self.act.servo.fault = if runaway {
            SpoolFault::HardOver(1.0)
        } else if valve_jam {
            SpoolFault::Jammed
        } else {
            SpoolFault::None
        };
        self.mode.update(&self.act, active, ok, backup_ok, dt);
        // The fluid in the cylinder is at the body's temperature: a cold-soaked actuator with little flow through it
        // stays thick until circulation and its own losses warm it.
        let oil_t = if self.temp.is_finite() { self.temp } else { temp };
        let rho = self.act.fluid.density(oil_t, 0.5 * (ps + pr));
        let mu = self.act.fluid.dynamic(oil_t, 0.5 * (ps + pr));
        Io {
            ps,
            pr,
            oil: Oil { rho, nu: mu / rho },
            mu,
            beta_t: oil_t,
            mode: self.mode.share,
            backup: self.mode.mode == Mode::Backup,
            cmd: cmd.unwrap_or(own),
            electric: powered,
            jammed,
            disconnected,
            seal_failed,
            ext_leak,
            sensor_error: self.sensor_error,
            wear: self.wear,
            volts: self.volts,
            dither,
        }
    }

    /// Writes the outputs and heats the body after a mechanism step.
    pub fn finish(&mut self, slots: &mut [f64], io: &Io, x: f64, v: f64, st: &[f64], out: &Out, dt: f64) {
        let amb = self.ambient.as_mut().map_or(288.15, |e| e.eval(slots, dt));
        if !self.temp.is_finite() {
            self.temp = amb;
        }
        if !self.winding.is_finite() {
            self.winding = self.temp;
        }
        let ua = self.act.heat_cap / 600.0;
        // Windings: copper and iron losses heat a small mass that sheds into the body.
        let motor_loss = self.act.pump.map_or(0.0, |p| out.electric * (1.0 - p.motor_eff));
        let w_cap = 0.08 * self.act.heat_cap;
        let w_ua = w_cap / 40.0;
        self.winding += (motor_loss - w_ua * (self.winding - self.temp)) / w_cap * dt;
        let oil_t = self.circuit.map_or(amb, |(_, t, _)| slots[t]);
        // Body warms with throttling and motor losses, and toward the circulating fluid.
        let circ = if self.circuit.is_some() { out.supply.abs() * io.oil.rho * self.act.fluid.cp * 0.3 } else { 0.0 };
        self.temp += (out.heat - motor_loss + w_ua * (self.winding - self.temp) - ua * (self.temp - amb) - circ * (self.temp - oil_t)) / self.act.heat_cap * dt;
        self.lost += out.ext_leak * dt;
        self.last_supply = out.supply.max(0.0);
        self.wear = (self.wear + v.abs() * dt / self.act.seal_life).min(1.0);
        let f1 = super::sensors::fault_code(self.sensors[0].as_mut().map_or(0.0, |e| e.eval(slots, dt)));
        let f2 = super::sensors::fault_code(self.sensors[1].as_mut().map_or(0.0, |e| e.eval(slots, dt)));
        let (l1, l2, used, disagree) = self.lvdt.read(x, f1, f2, dt);
        self.sensor_error = used - x;
        let a = &self.act;
        let stalled = (io.cmd - x / a.stroke).abs() > 0.02 && v.abs() < 0.02 * a.rate && io.mode > 0.5;
        let s = &self.s;
        s.put(slots, "position", (x / a.stroke).clamp(-0.05, 1.05));
        s.put(slots, "extension", x);
        s.put(slots, "velocity", v);
        s.put(slots, "force", out.force);
        s.put(slots, "pressure_a", st[0] - P_ATM);
        s.put(slots, "pressure_b", st[1] - P_ATM);
        s.put(slots, "valve", if a.control == Control::Selector { st[2] } else { st[2].clamp(-1.0, 1.0) });
        s.put(slots, "mode", match self.mode.mode {
            Mode::Active => 0.0,
            Mode::Passive => 1.0,
            Mode::Backup => 2.0,
        });
        s.flag(slots, "active", self.mode.mode != Mode::Passive && self.mode.share > 0.5);
        s.put(slots, "circuit_flow", if self.circuit.is_some() { out.supply.max(0.0) + out.ext_leak } else { 0.0 });
        s.put(slots, "circuit_held", if self.circuit.is_some() { a.held(x) + self.lost } else { 0.0 });
        s.put(slots, "electric_power", out.electric);
        s.put(slots, "temperature", self.temp);
        s.flag(slots, "overheat", self.temp > a.overheat);
        s.flag(slots, "cavitating", st[0] < self.act.fluid.air_release() || st[1] < self.act.fluid.air_release());
        s.flag(slots, "stalled", stalled);
        s.flag(slots, "relieving", out.relieving);
        s.put(slots, "lvdt_1", l1);
        s.put(slots, "lvdt_2", l2);
        s.flag(slots, "lvdt_disagree", disagree);
        let volts = self.volts * self.rated_v;
        s.put(slots, "current", if volts > 1.0 { out.electric / volts } else { 0.0 });
        s.put(slots, "seal_wear", self.wear);
        s.put(slots, "line_drop", self.line_drop);
        s.put(slots, "winding_temperature", self.winding);
    }

    pub fn state(&self, v: &mut Vec<f64>) {
        v.extend([self.mode.share, self.temp, self.lost, self.wear, self.sensor_error, self.lvdt.a.value, self.lvdt.b.value, self.winding, self.tripped as u8 as f64, self.line_drop, self.last_supply]);
    }

    pub fn restore(&mut self, it: &mut impl Iterator<Item = f64>) {
        self.mode.share = it.next().unwrap_or(0.0);
        self.temp = it.next().unwrap_or(f64::NAN);
        self.lost = it.next().unwrap_or(0.0);
        self.wear = it.next().unwrap_or(self.wear);
        self.sensor_error = it.next().unwrap_or(0.0);
        self.lvdt.a.value = it.next().unwrap_or(self.lvdt.a.value);
        self.lvdt.b.value = it.next().unwrap_or(self.lvdt.b.value);
        self.winding = it.next().unwrap_or(f64::NAN);
        self.tripped = it.next().unwrap_or(0.0) != 0.0;
        self.line_drop = it.next().unwrap_or(0.0);
        self.last_supply = it.next().unwrap_or(0.0);
    }
}
