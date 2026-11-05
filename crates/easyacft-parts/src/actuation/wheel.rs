//! A braked wheel at component level. Hydraulics: a normal brake servo valve and an alternate metering valve, each
//! a pressure-control valve on its own circuit, a shuttle valve choosing the higher, the parking brake trapping
//! alternate pressure, and the piston housing whose pressure first takes up the running clearance against the
//! retraction springs then rises steeply on the stack. Wheel: spins up and down under the tyre's slip-dependent
//! grip (Pacejka) on the runway's friction against brake torque, so a hard brake on ice locks it. Heat: a
//! five-node model (heat stack, piston housing, wheel rim, tyre carcass, tyre gas) with conduction, convection that
//! fans and airflow force, radiation from the stack, carbon heat capacity rising with temperature and oxidation
//! eating hot carbon. Carbon friction that is weak cold and damp and fades very hot; wear per application and per
//! energy; a lagging thermocouple; fuse plugs; tyre pressure from its gas; flat spots and bursts; brake fires.

use super::fluid::{Fluid, P_ATM};
use super::ode::{soft_sign, Outcome, Stepper};
use super::valve::{Oil, PressureControl, Shuttle};
use crate::util::Slots;
use crate::{expr_of, Expr};
use easyacft_core::Dim;
use easyacft_lang::schema::{opt, Kind, PartSpec};
use easyacft_lang::{Checked, CheckedPart, Type};

const B: Type = Type::Bool;
const N: Type = Type::Num(Dim::NONE);
const P: Type = Type::Num(Dim::PRESSURE);
const T: Type = Type::Num(Dim::TEMPERATURE);
const WHEN: Kind = Kind::Expr(Type::Bool);
const CIRCUIT: &[&str] = &["hydraulic.circuit"];
const PSI: f64 = 6_894.757;
const SIGMA: f64 = 5.670e-8;
const KT: f64 = 1852.0 / 3600.0;

pub static SPEC: PartSpec = PartSpec {
    domain: "brakes",
    kind: "wheel",
    help: "A braked wheel: normal servo valve and alternate metering valve through a shuttle, parking brake, piston housing take-up, wheel spin under tyre slip and runway friction, a five-node heat model with carbon oxidation, carbon friction, wear, thermocouple, fuse plugs, tyre gas, flat spots, bursts and brake fires.",
    fields: &[
        opt("circuit", Kind::Ref(CIRCUIT), "", "normal brake circuit"),
        opt("alternate_circuit", Kind::Ref(CIRCUIT), "", "alternate brake circuit or the brake accumulator's circuit"),
        opt("pedal", Kind::Expr(N), "0", "pilot's pedal 0 to 1, for the alternate metering valve"),
        opt("normal_command", Kind::Expr(N), "-1", "normal servo valve input 0 to 1 from the brake computer; -1 follows the pedal"),
        opt("normal_when", WHEN, "true", "normal brake selector valve open"),
        opt("alternate_command", Kind::Expr(N), "-1", "alternate valve input 0 to 1; -1 follows the pedal"),
        opt("alternate_when", WHEN, "true", "alternate brake line open"),
        opt("parking_when", WHEN, "false", "parking brake set: full alternate pressure, trapped"),
        opt("max_pressure", Kind::Quantity(Dim::PRESSURE), "3000 psi", "brake pressure at full input"),
        opt("air", Kind::Ref(&["environment.air"]), "", "for groundspeed, airspeed and outside temperature"),
        opt("load", Kind::Expr(Type::Num(Dim::FORCE)), "0 N", "vertical load on this wheel; 0 uses static_load on the ground"),
        opt("static_load", Kind::Quantity(Dim::FORCE), "120 kN", "wheel load at rest"),
        opt("runway_friction", Kind::Expr(N), "0.8", "tyre peak friction: 0.8 dry, 0.5 wet, 0.3 snow, 0.1 ice"),
        opt("max_torque", Kind::Quantity(Dim::TORQUE), "40000 N*m", "brake torque at full pressure and nominal friction"),
        opt("heat_sink", Kind::Quantity(Dim::MASS), "90 kg", "heat stack mass of this brake"),
        opt("material", Kind::Text, "carbon", "carbon or steel"),
        opt("wheel_mass", Kind::Quantity(Dim::MASS), "70 kg", "wheel rim and hub"),
        opt("tyre_mass", Kind::Quantity(Dim::MASS), "110 kg", "tyre carcass"),
        opt("tyre_radius", Kind::Quantity(Dim::LENGTH), "0.62 m", "loaded rolling radius"),
        opt("tyre_pressure", Kind::Quantity(Dim::PRESSURE), "210 psi", "cold inflation pressure, gauge"),
        opt("rated_load", Kind::Quantity(Dim::FORCE), "0 N", "tyre rated load; 0 means 1.3 times static load"),
        opt("fan_when", WHEN, "false", "brake cooling fan running"),
        opt("fuse_temperature", Kind::Quantity(Dim::TEMPERATURE), "450 K", "fuse plugs in the rim melt and deflate the tyre"),
        opt("hot_temperature", Kind::Quantity(Dim::TEMPERATURE), "573.15 K", "thermocouple reading that counts as hot"),
        opt("wear_pin", Kind::Number, "1", "wear pin left at start, share of a new brake"),
        opt("hydraulic_leak_when", WHEN, "false", "fluid spraying onto the brake"),
        opt("burst_when", WHEN, "false", "tyre burst from debris"),
        opt("seized_when", WHEN, "false", "brake dragging: stack seized on"),
        opt("thermocouple_failed_when", WHEN, "false", "thermocouple reads open circuit"),
    ],
    outputs: &[
        ("pressure", P),
        ("normal_pressure", P),
        ("alternate_pressure", P),
        ("torque", Type::Num(Dim::TORQUE)),
        ("force", Type::Num(Dim::FORCE)),
        ("friction", N),
        ("wheel_speed", Type::Num(Dim::SPEED)),
        ("slip", N),
        ("skidding", B),
        ("locked", B),
        ("temperature", T),
        ("stack_temperature", T),
        ("rim_temperature", T),
        ("tyre_temperature", T),
        ("hot", B),
        ("energy", Type::Num(Dim::ENERGY)),
        ("wear", N),
        ("tyre_pressure", P),
        ("tyre_low", B),
        ("fuse_melted", B),
        ("flat_spot", N),
        ("burst", B),
        ("fire", B),
        ("parking", B),
        ("circuit_flow", Type::Num(Dim::VOLUME_FLOW)),
        ("circuit_held", Type::Num(Dim::VOLUME)),
        ("alternate_circuit_flow", Type::Num(Dim::VOLUME_FLOW)),
        ("alternate_circuit_held", Type::Num(Dim::VOLUME)),
        ("failed", B),
    ],
};

/// Pacejka longitudinal force coefficient at slip ratio `s`, scaled to peak `mu`.
pub fn tyre_mu(s: f64, mu: f64) -> f64 {
    let (b, c, e) = (10.0, 1.9, 0.97);
    let bs = b * s;
    mu * (c * (bs - e * (bs - bs.atan())).atan()).sin()
}

/// Carbon or steel friction against stack temperature (K), sliding speed share and moisture.
pub fn pad_mu(carbon: bool, t: f64, wet: f64) -> f64 {
    let c = t - 273.15;
    if carbon {
        // Cold, damp carbon is weak until the first stops dry and warm it; strong 200 to 900 C; fades past 1100.
        let warm = (c / 250.0).clamp(0.0, 1.0);
        let base = 0.22 + 0.13 * warm;
        let damp = 1.0 - 0.35 * wet.clamp(0.0, 1.0) * (1.0 - warm);
        (base * damp * (1.0 - 0.5 * ((c - 1100.0) / 900.0).clamp(0.0, 1.0))).max(0.05)
    } else {
        (0.38 * (1.0 - 0.6 * ((c - 350.0) / 450.0).clamp(0.0, 1.0))).max(0.08)
    }
}

/// Carbon specific heat rises steeply with temperature.
fn cp_stack(carbon: bool, t: f64) -> f64 {
    if carbon {
        (720.0 + 2.2 * (t - 273.15)).clamp(600.0, 1900.0)
    } else {
        500.0 + 0.25 * (t - 273.15).max(0.0)
    }
}

/// Nodes: stack, housing, rim, tyre carcass, tyre gas.
const NODES: usize = 5;

pub struct Wheel {
    circuits: [Option<(usize, usize, usize)>; 2],
    e: [Option<Expr>; 13],
    air: [Option<usize>; 4],
    normal: PressureControl,
    alternate: PressureControl,
    shuttle: Shuttle,
    fluid: Fluid,
    p_max: f64,
    take_up: f64,
    p_spring: f64,
    stack_c: f64,
    t_max: f64,
    carbon: bool,
    mass0: f64,
    mass: f64,
    m: [f64; NODES],
    temp: [f64; NODES],
    radius: f64,
    inertia: f64,
    static_load: f64,
    rated: f64,
    cold_p: f64,
    gas: f64,
    fuse: f64,
    hot: f64,
    pub x: [f64; 4],
    stepper: Stepper,
    energy: f64,
    wear: f64,
    applied: bool,
    moisture: f64,
    thermo: f64,
    fused: bool,
    burst: bool,
    flat: f64,
    locked_time: f64,
    parked: bool,
    park_p: f64,
    park_t: f64,
    s: Slots,
}

pub const EXPRS: [&str; 13] = ["pedal", "normal_command", "normal_when", "alternate_command", "alternate_when", "parking_when", "load", "runway_friction", "fan_when", "hydraulic_leak_when", "burst_when", "seized_when", "thermocouple_failed_when"];

impl Wheel {
    pub fn new(c: &Checked, p: &CheckedPart) -> Self {
        let circ = |field: &str| {
            p.part(field).and_then(|i| {
                let id = &c.parts[i].id;
                let g = |o: &str| c.slots.index.get(&format!("{id}.{o}")).copied();
                Some((g("pressure")?, g("temperature")?, g("return_pressure")?))
            })
        };
        let air = |o: &str| p.part("air").and_then(|i| c.slots.index.get(&format!("{}.{o}", c.parts[i].id)).copied());
        let fluid = Fluid::new(super::fluid::Kind::PhosphateEster);
        let rho = fluid.density(313.15, 0.0);
        let p_max = p.num("max_pressure").max(1e5);
        let carbon = !p.text("material").eq_ignore_ascii_case("steel");
        let mass = p.num("heat_sink").max(1.0);
        let radius = p.num("tyre_radius").max(0.1);
        let m_rim = p.num("wheel_mass").max(1.0);
        let m_tyre = p.num("tyre_mass").max(1.0);
        let static_load = p.num("static_load").max(1.0);
        let rated = if p.num("rated_load") > 0.0 { p.num("rated_load") } else { 1.3 * static_load };
        // Piston housing: 8 cm3 of running clearance at 50 psi, then a stiff stack: full pressure 12 cm3 later.
        let take_up = 8e-6;
        let stack_c = 12e-6 / p_max;
        Wheel {
            circuits: [circ("circuit"), circ("alternate_circuit")],
            e: EXPRS.map(|k| expr_of(p, k)),
            air: [air("groundspeed"), air("airspeed"), air("temperature"), air("on_ground")],
            normal: PressureControl::rated(12e-3 / 60.0, p_max, rho),
            alternate: PressureControl { gain: 4.0, tau: 0.03, ..PressureControl::rated(8e-3 / 60.0, p_max, rho) },
            shuttle: Shuttle::default(),
            fluid,
            p_max,
            take_up,
            p_spring: 50.0 * PSI,
            stack_c,
            t_max: p.num("max_torque").max(1.0),
            carbon,
            mass0: mass,
            mass,
            m: [mass, 0.25 * mass, m_rim, m_tyre, 0.0],
            temp: [f64::NAN; NODES],
            radius,
            inertia: 0.5 * (m_rim + m_tyre) * radius * radius * 0.7 + 0.25 * mass * 0.25 * 0.25,
            static_load,
            rated,
            cold_p: p.num("tyre_pressure").max(1e4),
            gas: 1.0,
            fuse: p.num("fuse_temperature"),
            hot: p.num("hot_temperature"),
            x: [0.0, 0.0, 0.0, 0.0],
            stepper: Stepper::new(4),
            energy: 0.0,
            wear: p.num("wear_pin").clamp(0.0, 1.0),
            applied: false,
            moisture: 0.5,
            thermo: f64::NAN,
            fused: false,
            burst: false,
            flat: 0.0,
            locked_time: 0.0,
            parked: false,
            park_p: 0.0,
            park_t: 0.0,
            s: Slots::of(p, &[
                "pressure", "normal_pressure", "alternate_pressure", "torque", "force", "friction", "wheel_speed", "slip", "skidding", "locked", "temperature", "stack_temperature", "rim_temperature", "tyre_temperature", "hot", "energy", "wear", "tyre_pressure", "tyre_low", "fuse_melted", "flat_spot", "burst", "fire", "parking", "circuit_flow", "circuit_held", "alternate_circuit_flow", "alternate_circuit_held", "failed",
            ]),
        }
    }

    /// Brake housing pressure from the fluid it holds: clearance take-up against the springs, then the stack.
    fn housing_p(&self, v: f64) -> f64 {
        if v <= self.take_up {
            self.p_spring * (v / self.take_up).max(0.0)
        } else {
            self.p_spring + (v - self.take_up) / self.stack_c
        }
    }

    fn supply(&self, k: usize, slots: &[f64]) -> (f64, f64, f64) {
        match self.circuits[k] {
            Some((p, t, r)) => {
                let pr = slots[r].max(P_ATM);
                (slots[p].max(0.0) + pr, pr, slots[t])
            }
            None => (P_ATM, P_ATM, 288.15),
        }
    }

    pub fn step(&mut self, slots: &mut [f64], dt: f64) {
        let ev = |e: &mut Option<Expr>, s: &[f64], or: f64| e.as_mut().map_or(or, |x| x.eval(s, dt));
        let pedal = ev(&mut self.e[0], slots, 0.0).clamp(0.0, 1.0);
        let n_cmd = ev(&mut self.e[1], slots, -1.0);
        let n_on = ev(&mut self.e[2], slots, 1.0) != 0.0;
        let a_cmd = ev(&mut self.e[3], slots, -1.0);
        let a_on = ev(&mut self.e[4], slots, 1.0) != 0.0;
        let park = ev(&mut self.e[5], slots, 0.0) != 0.0;
        let load_in = ev(&mut self.e[6], slots, 0.0);
        let mu_rwy = ev(&mut self.e[7], slots, 0.8).clamp(0.02, 1.2);
        let fans = ev(&mut self.e[8], slots, 0.0) != 0.0;
        let leak = ev(&mut self.e[9], slots, 0.0) != 0.0;
        let burst_cmd = ev(&mut self.e[10], slots, 0.0) != 0.0;
        let seized = ev(&mut self.e[11], slots, 0.0) != 0.0;
        let tc_failed = ev(&mut self.e[12], slots, 0.0) != 0.0;
        let failed = self.s.on(slots, "failed");
        let gs = self.air[0].map_or(0.0, |i| slots[i]).abs();
        let tas = self.air[1].map_or(0.0, |i| slots[i]).abs();
        let t_air = self.air[2].map_or(288.15, |i| slots[i]).max(150.0);
        if self.temp[0].is_nan() {
            self.temp = [t_air; NODES];
            self.thermo = t_air;
            // Spun up at start if rolling.
            self.x[3] = gs / self.radius;
        }
        let (ps_n, pr_n, t_n) = self.supply(0, slots);
        let (ps_a, pr_a, _) = self.supply(1, slots);
        let pr = pr_n.max(pr_a);
        if self.x[1] <= 0.0 {
            self.x[1] = pr_n;
            self.x[2] = pr_a;
        }
        let normal_in = if n_cmd < 0.0 { pedal } else { n_cmd.clamp(0.0, 1.0) };
        let mut alt_in = if a_cmd < 0.0 { pedal } else { a_cmd.clamp(0.0, 1.0) };
        if park {
            alt_in = 1.0;
        }
        let n_open = n_on && self.circuits[0].is_some() && !failed;
        let a_open = a_on && self.circuits[1].is_some() && !failed;
        let rho = self.fluid.density(t_n, 1e7);
        let mu_oil = self.fluid.dynamic(t_n, 1e7);
        let oil = Oil { rho, nu: mu_oil / rho };
        self.shuttle.update(self.x[1], self.x[2], dt);
        let sh = self.shuttle.pos;
        // Parking: the alternate line is locked off once pressurised, so the brake keeps what it had.
        self.park_t = if park { self.park_t + dt } else { 0.0 };
        if park && !self.parked && self.park_t > 1.5 {
            self.parked = true;
            self.park_p = self.x[2];
        }
        if !park {
            self.parked = false;
        }
        let parked = self.parked;
        let load = if self.e[6].is_some() && load_in > 0.0 {
            load_in
        } else if self.air[3].is_some_and(|i| slots[i] != 0.0) {
            self.static_load
        } else {
            0.0
        };
        let mu_pad = pad_mu(self.carbon, self.temp[0], self.moisture);
        let t_gain = self.t_max * mu_pad / 0.32;
        let (r, inertia) = (self.radius * (1.0 - 0.02 * self.flat), self.inertia);
        let deflated = self.burst || self.fused;
        let peak = mu_rwy * if deflated { 0.5 } else { 1.0 };
        let (normal, alternate) = (self.normal, self.alternate);
        let p_max = self.p_max;
        let mut stepper = std::mem::take(&mut self.stepper);
        let me = &*self;
        let rolling = 0.012 * load * r;
        let f = |s: &[f64], d: &mut [f64]| {
            let (v_brake, pn, pa, w) = (s[0], s[1], s[2], s[3]);
            let p = me.housing_p(v_brake);
            // Each valve's output line: its own pressure, flowing into the brake through the shuttle.
            let xn = normal.target(normal_in, pn);
            let (qn, _, _) = if n_open { normal.flow(xn, ps_n, pr_n, pn, oil) } else { (0.0, 0.0, 0.0) };
            let xa = alternate.target(alt_in, pa);
            let (qa, _, _) = if parked { (0.0, 0.0, 0.0) } else if a_open { alternate.flow(xa, ps_a, pr_a, pa, oil) } else { (0.0, 0.0, 0.0) };
            let line_c = 4e-6 / p_max;
            let into_brake_n = oil.q(3e-6, pn - p) * (1.0 - sh);
            let into_brake_a = oil.q(3e-6, pa - p) * sh;
            d[1] = (qn - into_brake_n) / line_c;
            d[2] = (qa - into_brake_a) / line_c;
            if !n_open && pn > pr_n {
                d[1] -= (pn - pr_n) / 0.2;
            }
            if !a_open && !parked && pa > pr_a {
                d[2] -= (pa - pr_a) / 0.2;
            }
            d[0] = into_brake_n + into_brake_a;
            // Wheel: tyre grip from slip, brake torque against rotation, rolling resistance.
            let v = gs.max(0.0);
            let slip = (v - w * r) / v.max(2.0);
            let fx = tyre_mu(slip, peak) * load;
            let clamp = (p - me.p_spring).max(0.0) / p_max;
            let tb = if seized { 2.0 * me.t_max } else { t_gain * clamp };
            d[3] = (fx * r - tb * soft_sign(w, 0.2) - rolling * soft_sign(w, 0.5)) / inertia;
            if load <= 0.0 {
                // In the air the wheel just spins down under its brake and bearing drag.
                d[3] = (-tb * soft_sign(w, 0.2) - 0.5 * soft_sign(w, 0.5) - 0.02 * w) / inertia;
            }
        };
        let mut x = self.x;
        let scale = [1e-8, 0.002 * self.p_max, 0.002 * self.p_max, 0.05];
        let restored = stepper.step(&mut x, dt, &scale, 1.0, f) == Outcome::Restored;
        self.stepper = stepper;
        if restored {
            x = [x[0].max(0.0), pr, pr, gs / r];
        }
        x[0] = x[0].max(0.0);
        x[1] = x[1].max(pr.min(P_ATM));
        x[2] = if parked { self.park_p } else { x[2].max(pr.min(P_ATM)) };
        self.x = x;
        let p = self.housing_p(x[0]);
        let clamp = (p - self.p_spring).max(0.0) / self.p_max;
        let tb = if seized { 2.0 * self.t_max } else { t_gain * clamp };
        let w = x[3];
        let v = gs;
        let slip = if v > 0.5 { ((v - w * r) / v).clamp(-1.0, 1.0) } else { 0.0 };
        let power = tb * w.abs();
        // Flows the circuits supply: what fills the line, from whichever valve is metering.
        let q_n = if n_open { normal.flow(normal.target(normal_in, x[1]), ps_n, pr_n, x[1], oil).1 } else { 0.0 };
        let q_a = if a_open && !parked { alternate.flow(alternate.target(alt_in, x[2]), ps_a, pr_a, x[2], oil).1 } else { 0.0 };
        // Applications: each new squeeze wears carbon, more so cold.
        let pressing = p > 2.0 * self.p_spring;
        if pressing && !self.applied && w.abs() > 1.0 {
            let cold = if self.temp[0] < 473.15 { 1.0 } else { 0.4 };
            self.wear -= 1.5e-5 * cold * if self.carbon { 1.0 } else { 0.5 };
        }
        self.applied = pressing;
        self.energy += power * dt;
        self.wear = (self.wear - power * dt / 1e6 * if self.carbon { 2e-6 } else { 6e-6 }).max(0.0);
        // Moisture: dried by heat, picked up again slowly cold.
        let t0 = self.temp[0];
        self.moisture = (self.moisture + dt * if t0 > 373.15 { -(t0 - 373.15) / 3000.0 } else { 1.0 / 7200.0 }).clamp(0.0, 1.0);
        self.heat(power, gs.max(tas), t_air, fans, load, w, dt);
        // Thermocouple: a 15 s lag on the stack.
        self.thermo += (self.temp[0] - self.thermo) * (dt / 15.0).min(1.0);
        if self.temp[2] > self.fuse {
            self.fused = true;
        }
        // Skid: a wheel stopped on a moving runway grinds a flat spot through the tread and bursts it.
        let locked = v > 10.0 * KT && w.abs() * r < 0.1 * v;
        self.locked_time = if locked { self.locked_time + dt } else { 0.0 };
        if locked {
            self.flat = (self.flat + dt * v / 40.0 / 3.0).min(1.0);
        }
        let tyre_p = self.tyre_pressure();
        if burst_cmd || self.flat >= 1.0 || tyre_p > 4.0 * self.cold_p {
            self.burst = true;
        }
        let deflate = if self.burst { dt / 0.5 } else if self.fused { dt / 30.0 } else { 0.02 / 86400.0 * dt };
        self.gas = (self.gas - deflate * self.gas.max(0.05)).max(0.0);
        let fire = leak && self.temp[0] > 750.0;
        let s = &self.s;
        s.put(slots, "pressure", p - P_ATM.min(p));
        s.put(slots, "normal_pressure", x[1] - pr_n.min(x[1]));
        s.put(slots, "alternate_pressure", x[2] - pr_a.min(x[2]));
        s.put(slots, "torque", tb * soft_sign(w, 0.2));
        s.put(slots, "force", tyre_mu(slip, peak) * load);
        s.put(slots, "friction", mu_pad);
        s.put(slots, "wheel_speed", w * r);
        s.put(slots, "slip", slip);
        s.flag(slots, "skidding", slip > 0.3 && v > 5.0 * KT);
        s.flag(slots, "locked", locked);
        s.put(slots, "temperature", if tc_failed { 0.0 } else { self.thermo });
        s.put(slots, "stack_temperature", self.temp[0]);
        s.put(slots, "rim_temperature", self.temp[2]);
        s.put(slots, "tyre_temperature", self.temp[3]);
        s.flag(slots, "hot", !tc_failed && self.thermo > self.hot);
        s.put(slots, "energy", self.energy);
        s.put(slots, "wear", self.wear);
        s.put(slots, "tyre_pressure", tyre_p);
        s.flag(slots, "tyre_low", tyre_p < 0.9 * self.cold_p);
        s.flag(slots, "fuse_melted", self.fused);
        s.put(slots, "flat_spot", self.flat);
        s.flag(slots, "burst", self.burst);
        s.flag(slots, "fire", fire);
        s.flag(slots, "parking", parked || park);
        s.put(slots, "circuit_flow", if self.circuits[0].is_some() { q_n } else { 0.0 });
        s.put(slots, "circuit_held", if self.circuits[0].is_some() { x[0] * (1.0 - sh) } else { 0.0 });
        s.put(slots, "alternate_circuit_flow", if self.circuits[1].is_some() { q_a } else { 0.0 });
        s.put(slots, "alternate_circuit_held", if self.circuits[1].is_some() { x[0] * sh } else { 0.0 });
    }

    fn tyre_pressure(&self) -> f64 {
        let abs = (self.cold_p + P_ATM) * self.gas * self.temp[4] / 288.15;
        (abs - P_ATM).max(0.0)
    }

    /// Heat flows between the nodes and out to the air, with substeps where the fans make a node fast.
    fn heat(&mut self, power: f64, airflow: f64, t_air: f64, fans: bool, load: f64, w: f64, dt: f64) {
        let area = 0.05 * self.mass0.powf(2.0 / 3.0);
        let h_stack = 12.0 + 4.5 * airflow.sqrt() + if fans { 90.0 } else { 0.0 };
        let h_rim = 10.0 + 6.0 * airflow.sqrt() + if fans { 30.0 } else { 0.0 };
        let h_tyre = 8.0 + 5.0 * airflow.sqrt();
        let a = [area, 0.4 * area, 1.2 * area, 3.0 * area, 0.0];
        let g_sh = 0.6 * area * 40.0;
        let g_sr = 0.5 * area * 25.0;
        let g_rt = 0.9 * area * 30.0;
        let g_tg = 2.5 * area * 20.0;
        let g_rg = 1.5 * area * 20.0;
        // Rolling heat in the carcass, more on an underinflated or overloaded tyre.
        let defl = (load / self.rated) / (self.tyre_pressure() / self.cold_p).max(0.2);
        let rolling = 0.004 * load * (w.abs() * self.radius) * defl.clamp(0.2, 3.0);
        let mut cap = [0.0; NODES];
        for k in 0..NODES {
            cap[k] = match k {
                0 => self.mass * cp_stack(self.carbon, self.temp[0]),
                1 => self.m[1] * 900.0,
                2 => self.m[2] * 900.0,
                3 => self.m[3] * 1700.0,
                _ => 2000.0,
            };
        }
        let g_total = [g_sh + g_sr + h_stack * a[0] + 40.0, g_sh + h_rim * a[1], g_sr + g_rt + g_rg + h_rim * a[2], g_rt + g_tg + h_tyre * a[3], g_tg + g_rg];
        let tau_min = (0..NODES).map(|k| cap[k] / g_total[k].max(1e-6)).fold(f64::INFINITY, f64::min);
        let n = ((dt / (0.4 * tau_min)).ceil() as usize).clamp(1, 200);
        let h = dt / n as f64;
        for _ in 0..n {
            let t = self.temp;
            let rad = 0.8 * SIGMA * a[0] * (t[0].powi(4) - t_air.powi(4));
            let q = [
                0.95 * power - g_sh * (t[0] - t[1]) - g_sr * (t[0] - t[2]) - h_stack * a[0] * (t[0] - t_air) - rad,
                0.05 * power + g_sh * (t[0] - t[1]) - h_rim * a[1] * (t[1] - t_air),
                g_sr * (t[0] - t[2]) - g_rt * (t[2] - t[3]) - g_rg * (t[2] - t[4]) - h_rim * a[2] * (t[2] - t_air),
                rolling + g_rt * (t[2] - t[3]) - g_tg * (t[3] - t[4]) - h_tyre * a[3] * (t[3] - t_air),
                g_tg * (t[3] - t[4]) + g_rg * (t[2] - t[4]),
            ];
            for k in 0..NODES {
                self.temp[k] = (t[k] + q[k] / cap[k] * h).clamp(150.0, 3500.0);
            }
        }
        // Hot carbon oxidises: Arrhenius mass loss past about 500 C.
        if self.carbon && self.temp[0] > 773.15 {
            let k = 3e-6 * (-12_000.0 * (1.0 / self.temp[0] - 1.0 / 873.15)).exp();
            let lost = k * self.mass * dt;
            self.mass = (self.mass - lost).max(0.3 * self.mass0);
            self.wear = (self.wear - lost / self.mass0).max(0.0);
        }
    }

    pub fn state(&self, v: &mut Vec<f64>) {
        v.extend(self.x);
        v.extend(self.temp);
        v.extend([self.mass, self.energy, self.wear, self.moisture, self.thermo, self.fused as u8 as f64, self.burst as u8 as f64, self.flat, self.gas, self.shuttle.pos, self.parked as u8 as f64, self.park_p]);
    }

    pub fn restore(&mut self, it: &mut impl Iterator<Item = f64>) {
        for v in &mut self.x {
            *v = it.next().unwrap_or(*v);
        }
        for v in &mut self.temp {
            *v = it.next().unwrap_or(*v);
        }
        let mut n = |or: f64| it.next().unwrap_or(or);
        self.mass = n(self.mass0);
        self.energy = n(0.0);
        self.wear = n(1.0);
        self.moisture = n(0.5);
        self.thermo = n(f64::NAN);
        self.fused = n(0.0) != 0.0;
        self.burst = n(0.0) != 0.0;
        self.flat = n(0.0);
        self.gas = n(1.0);
        self.shuttle.pos = n(0.0);
        self.parked = n(0.0) != 0.0;
        self.park_p = n(0.0);
    }
}
