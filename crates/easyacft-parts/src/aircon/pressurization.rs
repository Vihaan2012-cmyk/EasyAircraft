//! Pressurisation: the fuselage as a pressure vessel (mass and energy from the mixer, zone exhausts, outflow
//! valves, safety valves, leaks and breaches), outflow valves driven by two automatic motors and a manual one,
//! pneumatic safety and negative relief valves, and the cabin pressure controller with its flight-phase
//! schedule, landing elevation, prepressurisation, abort, ditching and cabin altitude limiting.

use super::thermo::{self, isa_altitude, isa_pressure, lag, Actuator, Confirm, Pi, CP, CV, P_SL, PSI, R};
use super::{eval, slot_of, yes};
use crate::util::Slots;
use crate::Expr;
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
const FUSELAGE: &[&str] = &["pressurization.fuselage"];

pub const FUSELAGE_SPEC: PartSpec = PartSpec {
    domain: "pressurization",
    kind: "fuselage",
    help: "The pressure vessel: cabin air mass and temperature from what the mixer and other inflows bring, what the outflow and safety valves, leaks and any breach let out.",
    fields: &[
        req("air", Kind::Ref(AIR), "outside air"),
        req("volume", Kind::Quantity(Dim::VOLUME), "pressurised volume"),
        opt("mixers", Kind::Refs(&["aircon.mixer"]), "", "mixer units that supply it"),
        opt("inflow", Kind::Expr(MF), "0 kg/s", "other inflow, such as packs not modelled through a mixer"),
        opt("inflow_temperature", Kind::Expr(T), "295 K", "temperature of that inflow"),
        opt("leak_area", Kind::Number, "0.003", "door seals and fuselage leakage, m²"),
        opt("breach_area", Kind::Expr(N), "0", "a hole in the fuselage, m²"),
        opt("temperature", Kind::Quantity(Dim::TEMPERATURE), "0 K", "cabin temperature at start; 0 K means the outside air"),
        opt("structure_ua", Kind::Number, "1500", "used only without zones: heat lost through the fuselage, W/K"),
        opt("warning_altitude", Kind::Quantity(Dim::LENGTH), "9550 ft", "excessive cabin altitude warning"),
        opt("max_differential", Kind::Quantity(Dim::PRESSURE), "9.25 psi", "structural limit, for the excess differential flag"),
    ],
    outputs: &[
        ("pressure", P),
        ("altitude", LEN),
        ("differential", P),
        ("vertical_speed", SPD),
        ("temperature", T),
        ("mass", M),
        ("inflow", MF),
        ("outflow", MF),
        ("leak_flow", MF),
        ("excess_altitude", B),
        ("excess_differential", B),
        ("negative_differential", B),
        ("rapid_decompression", B),
        ("residual_pressure", B),
        ("failed", B),
    ],
};

pub const VALVE_SPEC: PartSpec = PartSpec {
    domain: "pressurization",
    kind: "outflow_valve",
    help: "An outflow valve: a door or butterfly driven by two automatic motors (one per controller channel) and a manual motor, each with its own power.",
    fields: &[
        req("fuselage", Kind::Ref(FUSELAGE), "the fuselage it vents"),
        opt("area", Kind::Number, "0.08", "flow area fully open, m²"),
        opt("auto_time", Kind::Quantity(Dim::TIME), "12 s", "closed to open on an automatic motor"),
        opt("manual_time", Kind::Quantity(Dim::TIME), "30 s", "closed to open on the manual motor"),
        opt("motor_1_when", WHEN, "true", "automatic motor 1 has power"),
        opt("motor_2_when", WHEN, "true", "automatic motor 2 has power"),
        opt("manual_motor_when", WHEN, "true", "manual motor has power"),
        opt("jammed_when", WHEN, "false", "the door is jammed"),
        opt("shape", Kind::Text, "door", "door (area follows opening) or butterfly (area follows 1 - cos)"),
        opt("thrust_recovery", Kind::Number, "0.6", "share of the jet momentum recovered as thrust, for drag"),
    ],
    outputs: &[("position", N), ("area", N), ("flow", MF), ("motor", N), ("thrust", Type::Num(Dim::FORCE)), ("jammed", B), ("failed", B)],
};

pub const SAFETY_SPEC: PartSpec = PartSpec {
    domain: "pressurization",
    kind: "safety_valve",
    help: "A pneumatic safety valve: opens on positive differential, and on negative differential as a negative relief valve.",
    fields: &[
        req("fuselage", Kind::Ref(FUSELAGE), "the fuselage it protects"),
        opt("opening", Kind::Quantity(Dim::PRESSURE), "8.85 psi", "differential at which it cracks open"),
        opt("full_open", Kind::Quantity(Dim::PRESSURE), "0.3 psi", "further differential to full open"),
        opt("negative", Kind::Quantity(Dim::PRESSURE), "0.5 psi", "outside-over-cabin differential at which it relieves inward"),
        opt("area", Kind::Number, "0.035", "flow area fully open, m²"),
        opt("stuck_closed_when", WHEN, "false", "stuck shut"),
        opt("stuck_open_when", WHEN, "false", "stuck open"),
    ],
    outputs: &[("open", N), ("negative_relief", B), ("flow", MF), ("failed", B)],
};

pub const CONTROLLER_SPEC: PartSpec = PartSpec {
    domain: "pressurization",
    kind: "controller",
    help: "The cabin pressure controller: schedules cabin altitude through ground, takeoff, climb, cruise, descent and abort, drives the outflow valves through either channel, and hands over to manual.",
    fields: &[
        req("fuselage", Kind::Ref(FUSELAGE), "the fuselage it controls"),
        req("air", Kind::Ref(AIR), "air data: pressure altitude, vertical speed, on ground"),
        req("valves", Kind::Refs(&["pressurization.outflow_valve"]), "outflow valves it drives"),
        opt("landing_elevation", Kind::Expr(LEN), "0 m", "destination elevation from the FMS"),
        opt("manual_elevation_when", WHEN, "false", "landing elevation selected manually"),
        opt("manual_elevation", Kind::Expr(LEN), "0 m", "that manual elevation"),
        opt("takeoff_when", WHEN, "false", "takeoff power set: prepressurise on the runway"),
        opt("manual_when", WHEN, "false", "mode selector in manual: the manual motor follows the cockpit switch"),
        opt("manual_rate", Kind::Expr(N), "0", "manual outflow valve switch: -1 close, +1 open"),
        opt("manual_vs_when", WHEN, "false", "manual cabin vertical speed"),
        opt("manual_vs", Kind::Expr(SPD), "0 m/s", "that cabin vertical speed"),
        opt("ditching_when", WHEN, "false", "ditching: every valve shuts"),
        opt("channel_1_failed_when", WHEN, "false", "channel 1 lost"),
        opt("channel_2_failed_when", WHEN, "false", "channel 2 lost"),
        opt("max_differential", Kind::Quantity(Dim::PRESSURE), "8.6 psi", "highest differential it schedules"),
        opt("cabin_ceiling", Kind::Quantity(Dim::LENGTH), "8000 ft", "highest cabin altitude it schedules"),
        opt("climb_rate", Kind::Quantity(Dim::SPEED), "500 ft/min", "fastest cabin climb"),
        opt("descent_rate", Kind::Quantity(Dim::SPEED), "350 ft/min", "fastest cabin descent"),
        opt("prepressurisation", Kind::Quantity(Dim::PRESSURE), "0.1 psi", "differential held on the takeoff roll"),
        opt("landing_differential", Kind::Quantity(Dim::PRESSURE), "0.1 psi", "differential it arrives at the field with"),
        opt("altitude_limit", Kind::Quantity(Dim::LENGTH), "14000 ft", "cabin altitude above which it shuts the valves"),
        opt("ground_depressurisation", Kind::Quantity(Dim::TIME), "55 s", "after landing, time to drive the valves fully open"),
        opt("sensor_1_bias", Kind::Expr(P), "0 Pa", "channel 1 cabin pressure sensor error"),
        opt("sensor_2_bias", Kind::Expr(P), "0 Pa", "channel 2 cabin pressure sensor error"),
        opt("sensor_disagree", Kind::Quantity(Dim::PRESSURE), "0.3 psi", "difference between its two sensors that it treats as a sensor fault"),
        opt("vs_alert", Kind::Quantity(Dim::SPEED), "1800 ft/min", "cabin climb or descent rate that raises the alert"),
    ],
    outputs: &[
        ("target_altitude", LEN),
        ("target_rate", SPD),
        ("landing_elevation", LEN),
        ("mode", N),
        ("channel", N),
        ("command", N),
        ("manual", B),
        ("vs_high", B),
        ("elevation_fault", B),
        ("sensor_fault", B),
        ("fault", B),
        ("failed", B),
    ],
};

pub mod mode {
    pub const GROUND: f64 = 0.0;
    pub const TAKEOFF: f64 = 1.0;
    pub const CLIMB: f64 = 2.0;
    pub const CRUISE: f64 = 3.0;
    pub const DESCENT: f64 = 4.0;
    pub const ABORT: f64 = 5.0;
    pub const MANUAL: f64 = 6.0;
    pub const DITCHING: f64 = 7.0;
    pub const LIMIT: f64 = 8.0;
}

pub struct Fuselage {
    pub mixers: Vec<usize>,
    air: [usize; 3],
    inflow: Option<Expr>,
    inflow_t: Option<Expr>,
    leak_cda: f64,
    breach: Option<Expr>,
    volume: f64,
    structure_ua: f64,
    warning: f64,
    limit: f64,
    pub m: f64,
    pub t: f64,
    pub p: f64,
    last_h: f64,
    pub vs: f64,
    rapid: Confirm,
    pub outflow: f64,
    s: Slots,
}

impl Fuselage {
    pub fn new(c: &Checked, p: &CheckedPart, mixers: Vec<usize>) -> Option<Self> {
        let air = [slot_of(c, p, "air", "pressure")?, slot_of(c, p, "air", "temperature")?, slot_of(c, p, "air", "on_ground")?];
        let volume = p.num("volume").max(1.0);
        let t0 = p.num("temperature");
        let t0 = if t0 > 100.0 { t0 } else { 288.15 };
        Some(Fuselage {
            mixers,
            air,
            inflow: crate::expr_of(p, "inflow"),
            inflow_t: crate::expr_of(p, "inflow_temperature"),
            leak_cda: 0.7 * p.num("leak_area").max(0.0),
            breach: crate::expr_of(p, "breach_area"),
            volume,
            structure_ua: p.num("structure_ua").max(0.0),
            warning: p.num("warning_altitude"),
            limit: p.num("max_differential"),
            m: P_SL * volume / (R * t0),
            t: t0,
            p: P_SL,
            last_h: 0.0,
            vs: 0.0,
            rapid: Confirm::default(),
            outflow: 0.0,
            s: Slots::of(p, &["pressure", "altitude", "differential", "vertical_speed", "temperature", "mass", "inflow", "outflow", "leak_flow", "excess_altitude", "excess_differential", "negative_differential", "rapid_decompression", "residual_pressure"]),
        })
    }

    pub fn ambient(&self, slots: &[f64]) -> (f64, f64) {
        (slots[self.air[0]].max(100.0), slots[self.air[1]].max(150.0))
    }

    /// Before the vessel steps, set its first pressure from the outside on the very first call.
    pub fn prime(&mut self, slots: &[f64]) {
        if self.last_h == 0.0 && self.m > 0.0 {
            let (pa, ta) = self.ambient(slots);
            if (self.p - P_SL).abs() < 1.0 && (pa - P_SL).abs() > 1.0 {
                self.t = ta.max(200.0);
                self.m = pa * self.volume / (R * self.t);
                self.p = pa;
            }
            self.last_h = isa_altitude(self.p).max(1e-6);
        }
    }

    /// `mixed` is the conditioned air (flow, temperature); `zone_t` the zones' mass-weighted temperature if any;
    /// `valve_out` the flow the outflow and safety valves passed (signed, positive out).
    pub fn step(&mut self, slots: &mut [f64], dt: f64, mixed: (f64, f64), zone_t: Option<f64>, valve_out: f64) {
        let (pa, ta) = self.ambient(slots);
        let extra = eval(&mut self.inflow, slots, dt, 0.0).max(0.0);
        let extra_t = eval(&mut self.inflow_t, slots, dt, 295.0);
        let breach = eval(&mut self.breach, slots, dt, 0.0).max(0.0);
        let leak = thermo::orifice_signed(self.leak_cda, self.p, self.t, pa, ta);
        let hole = thermo::orifice_signed(0.65 * breach, self.p, self.t, pa, ta);
        let inflow = mixed.0 + extra;
        let out = valve_out + leak + hole;
        let inward = (-valve_out).max(0.0) + (-leak).max(0.0) + (-hole).max(0.0);
        let outward = out + inward;
        // Energy: zones own the cabin temperature when there are any; otherwise the vessel carries its own.
        let m_new = (self.m + (inflow - out) * dt).max(1e-3);
        if let Some(tz) = zone_t {
            let t_target = (tz * self.m + inward * dt * ta) / (self.m + inward * dt);
            self.t = lag(self.t, t_target, 2.0, dt);
        } else {
            let h_in = mixed.0 * CP * mixed.1 + extra * CP * extra_t + inward * CP * ta;
            let h_out = outward * CP * self.t;
            let wall = self.structure_ua * (self.t - ta);
            let energy = self.m * CV * self.t + (h_in - h_out - wall) * dt;
            self.t = (energy / (m_new * CV)).clamp(180.0, 360.0);
        }
        self.m = m_new;
        self.p = self.m * R * self.t / self.volume;
        let h = isa_altitude(self.p);
        let raw = (h - self.last_h) / dt.max(1e-6);
        self.vs = lag(self.vs, raw, 1.0, dt);
        self.last_h = h;
        self.outflow = out;
        let rapid = self.rapid.hold(self.vs > 10.0, 0.5, dt);
        let s = &self.s;
        s.put(slots, "pressure", self.p);
        s.put(slots, "altitude", h);
        s.put(slots, "differential", self.p - pa);
        s.put(slots, "vertical_speed", self.vs);
        s.put(slots, "temperature", self.t);
        s.put(slots, "mass", self.m);
        s.put(slots, "inflow", inflow);
        s.put(slots, "outflow", valve_out);
        s.put(slots, "leak_flow", leak + hole);
        s.flag(slots, "excess_altitude", h > self.warning);
        s.flag(slots, "excess_differential", self.p - pa > self.limit);
        s.flag(slots, "negative_differential", self.p < pa - 0.3 * PSI);
        s.flag(slots, "rapid_decompression", rapid);
        // A door opened against more than 2.5 hPa of cabin pressure can injure whoever opens it.
        s.flag(slots, "residual_pressure", slots[self.air[2]] != 0.0 && self.p - pa > 250.0);
    }

    pub fn state(&self, v: &mut Vec<f64>) {
        v.extend([self.m, self.t, self.p, self.last_h, self.vs, self.rapid.elapsed, self.outflow]);
    }

    pub fn restore(&mut self, it: &mut impl Iterator<Item = f64>) {
        let mut n = || it.next().unwrap_or(0.0);
        self.m = n();
        self.t = n();
        self.p = n();
        self.last_h = n();
        self.vs = n();
        self.rapid.elapsed = n();
        self.outflow = n();
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Shape {
    Door,
    Butterfly,
}

pub struct OutflowValve {
    pub fuselage: usize,
    cda: f64,
    auto: Actuator,
    manual_time: f64,
    m1: Option<Expr>,
    m2: Option<Expr>,
    mm: Option<Expr>,
    jammed: Option<Expr>,
    shape: Shape,
    recovery: f64,
    /// Command from the controller: target position, which channel (1, 2) or 3 for manual rate.
    pub target: f64,
    pub channel: u8,
    pub manual_rate: f64,
    pub flow: f64,
    s: Slots,
}

impl OutflowValve {
    pub fn new(p: &CheckedPart, fuselage: usize) -> Self {
        OutflowValve {
            fuselage,
            cda: 0.75 * p.num("area").max(0.0),
            auto: Actuator::new(1.0, p.num("auto_time")),
            manual_time: p.num("manual_time").max(1.0),
            m1: crate::expr_of(p, "motor_1_when"),
            m2: crate::expr_of(p, "motor_2_when"),
            mm: crate::expr_of(p, "manual_motor_when"),
            jammed: crate::expr_of(p, "jammed_when"),
            shape: if p.text("shape") == "butterfly" { Shape::Butterfly } else { Shape::Door },
            recovery: p.num("thrust_recovery").clamp(0.0, 1.0),
            target: 1.0,
            channel: 1,
            manual_rate: 0.0,
            flow: 0.0,
            s: Slots::of(p, &["position", "area", "flow", "motor", "thrust", "jammed", "failed"]),
        }
    }

    pub fn area(&self) -> f64 {
        let x = self.auto.position;
        self.cda * match self.shape {
            Shape::Door => x,
            Shape::Butterfly => 1.0 - (x * std::f64::consts::FRAC_PI_2).cos(),
        }
    }

    /// Move on whichever motor the command names; returns the flow it passes given cabin and outside states.
    pub fn step(&mut self, slots: &mut [f64], dt: f64, cabin: (f64, f64), outside: (f64, f64)) -> f64 {
        let failed = self.s.on(slots, "failed");
        self.auto.jammed = failed || yes(&mut self.jammed, slots, dt, false);
        let m1 = yes(&mut self.m1, slots, dt, true);
        let m2 = yes(&mut self.m2, slots, dt, true);
        let mm = yes(&mut self.mm, slots, dt, true);
        let motor = match self.channel {
            1 if m1 => 1.0,
            2 if m2 => 2.0,
            1 if m2 => 2.0,
            2 if m1 => 1.0,
            3 if mm => 3.0,
            _ => 0.0,
        };
        if motor == 3.0 {
            if !self.auto.jammed {
                let step = self.manual_rate.clamp(-1.0, 1.0) * dt / self.manual_time;
                self.auto.position = (self.auto.position + step).clamp(0.0, 1.0);
            }
        } else if motor > 0.0 {
            self.auto.drive(self.target, 1.0, dt);
        }
        let (pc, tc) = cabin;
        let (pa, ta) = outside;
        self.flow = thermo::orifice_signed(self.area(), pc, tc, pa, ta);
        let v_jet = if self.flow > 0.0 { (2.0 * (pc - pa).max(0.0) / (pc / (R * tc))).sqrt().min(340.0) } else { 0.0 };
        let s = &self.s;
        s.put(slots, "position", self.auto.position);
        s.put(slots, "area", self.area());
        s.put(slots, "flow", self.flow);
        s.put(slots, "motor", motor);
        s.put(slots, "thrust", self.flow.max(0.0) * v_jet * self.recovery);
        s.flag(slots, "jammed", self.auto.jammed);
        self.flow
    }

    pub fn state(&self, v: &mut Vec<f64>) {
        v.extend(self.auto.state());
        v.extend([self.target, self.channel as f64, self.manual_rate, self.flow]);
    }

    pub fn restore(&mut self, it: &mut impl Iterator<Item = f64>) {
        let mut n = || it.next().unwrap_or(0.0);
        self.auto.restore([n(), n()]);
        self.target = n();
        self.channel = n() as u8;
        self.manual_rate = n();
        self.flow = n();
    }
}

pub struct SafetyValve {
    pub fuselage: usize,
    opening: f64,
    full: f64,
    negative: f64,
    cda: f64,
    closed: Option<Expr>,
    open: Option<Expr>,
    position: f64,
    pub flow: f64,
    s: Slots,
}

impl SafetyValve {
    pub fn new(p: &CheckedPart, fuselage: usize) -> Self {
        SafetyValve {
            fuselage,
            opening: p.num("opening"),
            full: p.num("full_open").max(1.0),
            negative: p.num("negative"),
            cda: 0.7 * p.num("area").max(0.0),
            closed: crate::expr_of(p, "stuck_closed_when"),
            open: crate::expr_of(p, "stuck_open_when"),
            position: 0.0,
            flow: 0.0,
            s: Slots::of(p, &["open", "negative_relief", "flow", "failed"]),
        }
    }

    pub fn step(&mut self, slots: &mut [f64], dt: f64, cabin: (f64, f64), outside: (f64, f64)) -> f64 {
        let (pc, tc) = cabin;
        let (pa, ta) = outside;
        let diff = pc - pa;
        let stuck_closed = yes(&mut self.closed, slots, dt, false) || self.s.on(slots, "failed");
        let stuck_open = yes(&mut self.open, slots, dt, false);
        let negative = -diff > self.negative;
        let target = if stuck_open {
            1.0
        } else if stuck_closed {
            0.0
        } else if negative {
            ((-diff - self.negative) / self.full + 0.2).clamp(0.0, 1.0)
        } else {
            ((diff - self.opening) / self.full).clamp(0.0, 1.0)
        };
        // A poppet reacts in well under a second.
        self.position = lag(self.position, target, 0.15, dt);
        self.flow = thermo::orifice_signed(self.cda * self.position, pc, tc, pa, ta);
        let s = &self.s;
        s.put(slots, "open", self.position);
        s.flag(slots, "negative_relief", negative && !stuck_closed);
        s.put(slots, "flow", self.flow);
        self.flow
    }

    pub fn state(&self, v: &mut Vec<f64>) {
        v.extend([self.position, self.flow]);
    }

    pub fn restore(&mut self, it: &mut impl Iterator<Item = f64>) {
        let mut n = || it.next().unwrap_or(0.0);
        self.position = n();
        self.flow = n();
    }
}

pub struct Controller {
    pub fuselage: usize,
    pub valves: Vec<usize>,
    air: [usize; 4],
    landing: Option<Expr>,
    man_elev_when: Option<Expr>,
    man_elev: Option<Expr>,
    takeoff: Option<Expr>,
    manual: Option<Expr>,
    manual_rate: Option<Expr>,
    man_vs_when: Option<Expr>,
    man_vs: Option<Expr>,
    ditch: Option<Expr>,
    ch1: Option<Expr>,
    ch2: Option<Expr>,
    max_diff: f64,
    ceiling: f64,
    climb: f64,
    descend: f64,
    prepress: f64,
    landing_diff: f64,
    alt_limit: f64,
    ground_time: f64,
    target: f64,
    mode: f64,
    departure: f64,
    max_alt: f64,
    since_landing: f64,
    pi: Pi,
    was_ground: bool,
    /// What it asks of its valves this step.
    pub command: f64,
    pub channel: u8,
    pub manual_now: bool,
    pub manual_switch: f64,
    bias: [Option<Expr>; 2],
    disagree: f64,
    vs_alert: f64,
    vs_high: Confirm,
    sensors: [f64; 2],
    s: Slots,
}

impl Controller {
    pub fn new(c: &Checked, p: &CheckedPart, fuselage: usize, valves: Vec<usize>) -> Option<Self> {
        Some(Controller {
            fuselage,
            valves,
            air: [slot_of(c, p, "air", "pressure")?, slot_of(c, p, "air", "on_ground")?, slot_of(c, p, "air", "vertical_speed")?, slot_of(c, p, "air", "altitude")?],
            landing: crate::expr_of(p, "landing_elevation"),
            man_elev_when: crate::expr_of(p, "manual_elevation_when"),
            man_elev: crate::expr_of(p, "manual_elevation"),
            takeoff: crate::expr_of(p, "takeoff_when"),
            manual: crate::expr_of(p, "manual_when"),
            manual_rate: crate::expr_of(p, "manual_rate"),
            man_vs_when: crate::expr_of(p, "manual_vs_when"),
            man_vs: crate::expr_of(p, "manual_vs"),
            ditch: crate::expr_of(p, "ditching_when"),
            ch1: crate::expr_of(p, "channel_1_failed_when"),
            ch2: crate::expr_of(p, "channel_2_failed_when"),
            max_diff: p.num("max_differential"),
            ceiling: p.num("cabin_ceiling"),
            climb: p.num("climb_rate").max(0.1),
            descend: p.num("descent_rate").max(0.1),
            prepress: p.num("prepressurisation"),
            landing_diff: p.num("landing_differential"),
            alt_limit: p.num("altitude_limit"),
            ground_time: p.num("ground_depressurisation").max(1.0),
            target: 0.0,
            mode: mode::GROUND,
            departure: f64::NAN,
            max_alt: 0.0,
            since_landing: f64::INFINITY,
            pi: Pi::new(0.25, 0.02, 0.0, 1.0).starting_at(1.0),
            was_ground: true,
            command: 1.0,
            channel: 1,
            manual_now: false,
            manual_switch: 0.0,
            bias: [crate::expr_of(p, "sensor_1_bias"), crate::expr_of(p, "sensor_2_bias")],
            disagree: p.num("sensor_disagree").max(1.0),
            vs_alert: p.num("vs_alert").max(0.1),
            vs_high: Confirm::default(),
            sensors: [P_SL, P_SL],
            s: Slots::of(p, &["target_altitude", "target_rate", "landing_elevation", "mode", "channel", "command", "manual", "vs_high", "elevation_fault", "sensor_fault", "fault", "failed"]),
        })
    }

    /// `cabin` is the fuselage pressure, altitude and vertical speed.
    pub fn step(&mut self, slots: &mut [f64], dt: f64, cabin: (f64, f64, f64)) {
        let failed = self.s.on(slots, "failed");
        let (pa, ground, ac_vs, ac_alt) = (slots[self.air[0]].max(100.0), slots[self.air[1]] != 0.0, slots[self.air[2]], slots[self.air[3]]);
        let (pc_true, _, cabin_vs) = cabin;
        // Each channel reads its own cabin pressure sensor; a large disagreement takes the suspect channel out.
        for (k, b) in self.bias.iter_mut().enumerate() {
            let reading = pc_true + eval(b, slots, dt, 0.0);
            self.sensors[k] = lag(self.sensors[k], reading, 0.3, dt);
        }
        let disagree = (self.sensors[0] - self.sensors[1]).abs() > self.disagree;
        let pc = if self.channel == 2 { self.sensors[1] } else { self.sensors[0] };
        let cabin_alt = isa_altitude(pc);
        let h_out = isa_altitude(pa);
        if self.departure.is_nan() || (ground && self.was_ground) {
            self.departure = h_out;
        }
        let fms = eval(&mut self.landing, slots, dt, 0.0);
        let manual_elev = yes(&mut self.man_elev_when, slots, dt, false);
        let elevation_fault = !manual_elev && !(-600.0..=5000.0).contains(&fms);
        let landing = if manual_elev { eval(&mut self.man_elev, slots, dt, 0.0) } else if elevation_fault { 0.0 } else { fms };
        // With the sensors disagreeing, trust the one nearer what the outflow valve and air data imply.
        let suspect = if disagree { if (self.sensors[0] - pc_true).abs() > (self.sensors[1] - pc_true).abs() { 1 } else { 2 } } else { 0 };
        let ch1 = !failed && !yes(&mut self.ch1, slots, dt, false) && suspect != 1;
        let ch2 = !failed && !yes(&mut self.ch2, slots, dt, false) && suspect != 2;
        self.channel = if ch1 { 1 } else if ch2 { 2 } else { 0 };
        let manual = yes(&mut self.manual, slots, dt, false) || self.channel == 0;
        let ditching = yes(&mut self.ditch, slots, dt, false);
        let takeoff = yes(&mut self.takeoff, slots, dt, false);
        let manual_vs = yes(&mut self.man_vs_when, slots, dt, false);
        let vs_cmd = eval(&mut self.man_vs, slots, dt, 0.0);
        self.manual_switch = eval(&mut self.manual_rate, slots, dt, 0.0);
        if ground && !self.was_ground {
            self.since_landing = 0.0;
        }
        self.since_landing += dt;
        if !ground {
            self.max_alt = self.max_alt.max(ac_alt);
        } else if self.since_landing > self.ground_time {
            self.max_alt = 0.0;
        }
        self.was_ground = ground;

        // Lowest cabin altitude the structure allows right now.
        let h_floor = isa_altitude(pa + self.max_diff);
        let schedule = |alt: f64| -> f64 {
            // Cabin altitude rises in proportion to aircraft altitude, reaching the ceiling at maximum differential.
            let at_max = isa_altitude(isa_pressure(self.ceiling) - self.max_diff).max(1.0);
            (alt / at_max).clamp(0.0, 1.0) * self.ceiling
        };
        let (goal, rate_up, rate_down, m) = if ditching {
            (cabin_alt, 0.0, 0.0, mode::DITCHING)
        } else if manual {
            (cabin_alt, 0.0, 0.0, mode::MANUAL)
        } else if cabin_alt > self.alt_limit {
            (h_floor, self.climb, self.descend * 3.0, mode::LIMIT)
        } else if ground && takeoff {
            (isa_altitude(pa + self.prepress), self.climb, self.descend, mode::TAKEOFF)
        } else if ground {
            (h_out, self.climb * 2.0, self.descend * 2.0, mode::GROUND)
        } else if self.max_alt < self.departure + 1500.0 && ac_vs < -1.0 && ac_alt < self.departure + 1500.0 {
            (self.departure, self.climb, self.descend, mode::ABORT)
        } else if ac_vs < -2.5 {
            // Descent: arrive at the field just below its pressure, at a rate matched to the aircraft's descent.
            let land_target = isa_altitude(isa_pressure(landing) + self.landing_diff);
            let remaining = (ac_alt - landing).max(100.0);
            let needed = ((self.target - land_target).max(0.0) / remaining * (-ac_vs)).clamp(0.3, self.descend);
            (land_target, self.climb, needed, mode::DESCENT)
        } else if ac_vs > 2.5 {
            (schedule(ac_alt), self.climb, self.descend, mode::CLIMB)
        } else {
            (schedule(self.max_alt.max(ac_alt)), self.climb, self.descend, mode::CRUISE)
        };
        let goal = if !ground && !manual && !ditching { goal.max(h_floor) } else { goal };
        if manual_vs && !manual {
            self.target += vs_cmd * dt;
        } else if m != mode::MANUAL && m != mode::DITCHING {
            let step = if goal > self.target { rate_up * dt } else { rate_down * dt };
            self.target += (goal - self.target).clamp(-step, step);
        } else {
            self.target = cabin_alt;
        }
        if !ground && !manual {
            self.target = self.target.max(h_floor);
        }
        self.mode = m;
        let p_target = isa_pressure(self.target);
        // Valve command: integral on cabin pressure error, with a direct term.
        self.manual_now = manual;
        self.command = if ditching {
            0.0
        } else if manual {
            self.command
        } else if m == mode::GROUND && self.since_landing > self.ground_time {
            self.pi.reset(1.0);
            1.0
        } else if m == mode::LIMIT {
            self.pi.reset(0.0);
            0.0
        } else {
            self.pi.step((pc - p_target) / 2000.0, dt)
        };
        let s = &self.s;
        s.put(slots, "target_altitude", self.target);
        s.put(slots, "target_rate", if m == mode::DESCENT { -rate_down } else { rate_up });
        s.put(slots, "landing_elevation", landing);
        s.put(slots, "mode", m);
        s.put(slots, "channel", self.channel as f64);
        s.put(slots, "command", self.command);
        s.flag(slots, "manual", manual);
        s.flag(slots, "vs_high", self.vs_high.hold(cabin_vs.abs() > self.vs_alert, 5.0, dt));
        s.flag(slots, "elevation_fault", elevation_fault);
        s.flag(slots, "sensor_fault", disagree);
        s.flag(slots, "fault", !ch1 || !ch2);
    }

    pub fn state(&self, v: &mut Vec<f64>) {
        v.extend([self.sensors[0], self.sensors[1], self.vs_high.elapsed]);
        v.extend([self.target, self.mode, if self.departure.is_nan() { -1e9 } else { self.departure }, self.max_alt, self.since_landing.min(1e9), self.pi.integral, self.was_ground as u8 as f64, self.command, self.channel as f64]);
    }

    pub fn restore(&mut self, it: &mut impl Iterator<Item = f64>) {
        let mut n = || it.next().unwrap_or(0.0);
        self.sensors = [n(), n()];
        self.vs_high.elapsed = n();
        self.target = n();
        self.mode = n();
        let d = n();
        self.departure = if d <= -1e8 { f64::NAN } else { d };
        self.max_alt = n();
        self.since_landing = n();
        self.pi.integral = n();
        self.was_ground = n() != 0.0;
        self.command = n();
        self.channel = n() as u8;
    }
}
