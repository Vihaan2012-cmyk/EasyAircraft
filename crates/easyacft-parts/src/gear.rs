//! Landing gear: legs with door sequencing, locks, airload and hydraulic-pressure dependent travel, gravity
//! extension and weight-on-wheels; carbon or steel brakes with friction, a heat sink, convective and radiative
//! cooling, fuse plugs, fans and wear; tyres whose pressure follows their temperature; nosewheel steering
//! scheduled with speed.

use crate::util::Slots;
use crate::{expr_of, Expr};
use easyacft_core::Dim;
use easyacft_lang::schema::{opt, req, Kind, PartSpec};
use easyacft_lang::{Checked, CheckedPart, Type};

const B: Type = Type::Bool;
const N: Type = Type::Num(Dim::NONE);
const P: Type = Type::Num(Dim::PRESSURE);
const T: Type = Type::Num(Dim::TEMPERATURE);
const F: Type = Type::Num(Dim::FORCE);
const E: Type = Type::Num(Dim::ENERGY);
const VF: Type = Type::Num(Dim::VOLUME_FLOW);
const WHEN: Kind = Kind::Expr(Type::Bool);
const AIR: &[&str] = &["environment.air"];
const CIRCUIT: &[&str] = &["hydraulic.circuit"];

pub static SPECS: &[PartSpec] = &[
    PartSpec {
        domain: "gear",
        kind: "leg",
        help: "A landing gear leg: doors open, the leg travels, doors close; locks, airload, gravity extension, weight on wheels.",
        fields: &[
            opt("circuit", Kind::Ref(CIRCUIT), "", "hydraulic circuit that moves it; none for electric"),
            req("down_when", WHEN, "the gear lever, for example \"gear_lever.on\""),
            opt("free_fall_when", WHEN, "false", "alternate gravity extension"),
            opt("time", Kind::Quantity(Dim::TIME), "10 s", "leg travel time at nominal pressure, no airload"),
            opt("door_time", Kind::Quantity(Dim::TIME), "0 s", "door travel time before and after the leg moves; 0 for no doors"),
            opt("free_fall_time", Kind::Quantity(Dim::TIME), "0 s", "gravity extension time; 0 means two thirds of the travel time"),
            opt("airload", Kind::Quantity(Dim::SPEED), "0 m/s", "airspeed whose dynamic pressure doubles retraction time and halves extension time; 0 for none"),
            opt("air", Kind::Ref(AIR), "", "for airspeed and weight on wheels"),
            opt("swept_volume", Kind::Quantity(Dim::VOLUME), "0 L", "fluid the actuator moves over a full travel, for hydraulic demand"),
            opt("initial_down", Kind::Bool, "true", "down at start"),
            opt("retract_inhibit_when", WHEN, "false", "ground safety: retraction blocked, such as weight on wheels"),
        ],
        outputs: &[
            ("position", N),
            ("down_locked", B),
            ("up_locked", B),
            ("in_transit", B),
            ("doors", N),
            ("doors_closed", B),
            ("disagree", B),
            ("demand", VF),
            ("weight_on_wheels", B),
            ("failed", B),
        ],
    },
    PartSpec {
        domain: "gear",
        kind: "brake",
        help: "Wheel brakes: torque from pressure and pad friction, a heat sink heated by the energy absorbed, cooled by convection and radiation, fuse plugs, fans, wear.",
        fields: &[
            opt("circuit", Kind::Ref(CIRCUIT), "", "hydraulic circuit; none for an electric or accumulator-only brake"),
            req("pedal", Kind::Expr(N), "demand 0 to 1, for example \"left_pedal.position\""),
            req("max_pressure", Kind::Quantity(Dim::PRESSURE), "pressure at full pedal"),
            opt("air", Kind::Ref(AIR), "", "for groundspeed, airspeed and outside temperature"),
            opt("heating", Kind::Number, "300", "kelvin rise per second at full brake and 50 m/s, used when no heat sink mass is given"),
            opt("cooling", Kind::Quantity(Dim::TIME), "600 s", "cooling time constant still air, used when no heat sink mass is given"),
            opt("heat_sink", Kind::Quantity(Dim::MASS), "0 kg", "heat sink mass across the brakes this part stands for; 0 uses heating and cooling"),
            opt("material", Kind::Text, "carbon", "carbon or steel: heat capacity, friction and fade"),
            opt("max_torque", Kind::Quantity(Dim::TORQUE), "0 N*m", "torque per wheel at full pressure and nominal friction; 0 uses the energy model only"),
            opt("tyre_radius", Kind::Quantity(Dim::LENGTH), "0.6 m", "rolling radius"),
            opt("wheels", Kind::Number, "1", "wheels this part stands for"),
            opt("fan_when", WHEN, "false", "brake cooling fans running"),
            opt("fuse_temperature", Kind::Quantity(Dim::TEMPERATURE), "450 degC", "wheel fuse plugs melt and deflate the tyres"),
            opt("hot_temperature", Kind::Quantity(Dim::TEMPERATURE), "300 degC", "counts as hot"),
            opt("wear_per_mj", Kind::Number, "0.0001", "pad wear per megajoule absorbed, share of the wear pin"),
        ],
        outputs: &[
            ("pressure", P),
            ("temperature", T),
            ("hot", B),
            ("torque", Type::Num(Dim::TORQUE)),
            ("force", F),
            ("energy", E),
            ("friction", N),
            ("wear", N),
            ("fuse_melted", B),
            ("failed", B),
        ],
    },
    PartSpec {
        domain: "gear",
        kind: "tyre",
        help: "A tyre: pressure follows its gas temperature, heated by its brake and rolling; slow leak, fuse plug release, burst.",
        fields: &[
            req("pressure", Kind::Quantity(Dim::PRESSURE), "cold inflation pressure, gauge, for example \"218 psi\""),
            opt("brake", Kind::Ref(&["gear.brake"]), "", "the brake whose heat soaks in and whose fuse plugs deflate it"),
            opt("air", Kind::Ref(AIR), "", "for groundspeed and outside temperature"),
            opt("leak", Kind::Number, "0.02", "pressure lost per day, share"),
            opt("burst_when", WHEN, "false", "a burst: debris, a hard landing"),
            opt("low", Kind::Number, "0.9", "share of the cold pressure counted as low"),
        ],
        outputs: &[("pressure", P), ("temperature", T), ("low", B), ("deflated", B), ("burst", B), ("failed", B)],
    },
    PartSpec {
        domain: "gear",
        kind: "steering",
        help: "Nosewheel steering: tiller and pedal authority scheduled with groundspeed, a rate-limited actuator, castoring when unpowered or towing.",
        fields: &[
            opt("circuit", Kind::Ref(CIRCUIT), "", "hydraulic circuit that powers it"),
            opt("powered_when", WHEN, "true", "steering engaged, such as its computer running"),
            opt("tiller", Kind::Expr(N), "0", "tiller -1 left to 1 right"),
            opt("pedals", Kind::Expr(N), "0", "rudder pedals -1 to 1"),
            opt("air", Kind::Ref(AIR), "", "for groundspeed"),
            opt("tiller_angle", Kind::Number, "70", "full tiller, degrees"),
            opt("pedal_angle", Kind::Number, "6", "full pedal, degrees"),
            opt("full_until", Kind::Quantity(Dim::SPEED), "20 kt", "groundspeed below which the tiller has full authority"),
            opt("none_above", Kind::Quantity(Dim::SPEED), "70 kt", "groundspeed above which only the pedals steer"),
            opt("rate", Kind::Number, "20", "fastest wheel movement, degrees per second"),
            opt("towing_when", WHEN, "false", "towing: bypassed, the wheel castors"),
        ],
        outputs: &[("angle", N), ("command", N), ("authority", N), ("castoring", B), ("failed", B)],
    },
];

pub(crate) fn slot(c: &Checked, p: &CheckedPart, field: &str, out: &str) -> Option<usize> {
    p.part(field).and_then(|i| c.slots.index.get(&format!("{}.{out}", c.parts[i].id)).copied())
}

fn circuit_share(c: &Checked, p: &CheckedPart) -> Option<(usize, f64)> {
    let i = p.part("circuit")?;
    let cp = &c.parts[i];
    Some((*c.slots.index.get(&format!("{}.pressure", cp.id))?, cp.num("nominal").max(1.0)))
}

const RHO0: f64 = 1.225;
const SIGMA: f64 = 5.670e-8;

pub struct Leg {
    circuit: Option<(usize, f64)>,
    airspeed: Option<usize>,
    ground: Option<usize>,
    down: Option<Expr>,
    free: Option<Expr>,
    inhibit: Option<Expr>,
    time: f64,
    door_time: f64,
    free_time: f64,
    airload_q: f64,
    swept: f64,
    pos: f64,
    doors: f64,
    s: Slots,
}

impl Leg {
    pub fn new(c: &Checked, p: &CheckedPart) -> Self {
        let time = p.num("time").max(0.1);
        let free = p.num("free_fall_time");
        let v = p.num("airload");
        Leg {
            circuit: circuit_share(c, p),
            airspeed: slot(c, p, "air", "airspeed"),
            ground: slot(c, p, "air", "on_ground"),
            down: expr_of(p, "down_when"),
            free: expr_of(p, "free_fall_when"),
            inhibit: expr_of(p, "retract_inhibit_when"),
            time,
            door_time: p.num("door_time").max(0.0),
            free_time: if free > 0.0 { free } else { time * 2.0 / 3.0 },
            airload_q: 0.5 * RHO0 * v * v,
            swept: p.num("swept_volume"),
            pos: if p.flag("initial_down") { 1.0 } else { 0.0 },
            doors: 0.0,
            s: Slots::of(p, &["position", "down_locked", "up_locked", "in_transit", "doors", "doors_closed", "disagree", "demand", "weight_on_wheels", "failed"]),
        }
    }

    pub fn step(&mut self, slots: &mut [f64], dt: f64) {
        let want = self.down.as_mut().is_some_and(|e| e.yes(slots, dt));
        let free = self.free.as_mut().is_some_and(|e| e.yes(slots, dt));
        let inhibit = self.inhibit.as_mut().is_some_and(|e| e.yes(slots, dt));
        let failed = self.s.on(slots, "failed");
        let share = self.circuit.map_or(1.0, |(c, nominal)| (slots[c] / nominal).max(0.0));
        let powered = share > 0.3 && !failed;
        // Actuator speed falls with the square root of the pressure it has; dynamic pressure fights retraction and helps extension.
        let q = self.airspeed.map_or(0.0, |v| 0.5 * RHO0 * slots[v] * slots[v]);
        let air = if self.airload_q > 0.0 { q / self.airload_q } else { 0.0 };
        let speed = share.min(1.0).sqrt() / self.time;
        let door_rate = if self.door_time > 0.0 { share.min(1.0).sqrt() / self.door_time } else { f64::INFINITY };
        let before = self.pos;
        let target = if want { 1.0 } else if inhibit && self.pos > 0.0 { self.pos.max(1.0) } else { 0.0 };
        let moving = (target - self.pos).abs() > 1e-9;
        if free {
            self.doors = 1.0;
            self.pos = (self.pos + dt / self.free_time * (1.0 + air).min(2.0)).min(1.0);
        } else if powered {
            if moving {
                if self.doors < 1.0 {
                    self.doors = (self.doors + door_rate * dt).min(1.0);
                } else {
                    let rate = if target > self.pos { speed * (1.0 + air).min(2.0) } else { speed / (1.0 + air) };
                    self.pos = if target > self.pos { (self.pos + rate * dt).min(1.0) } else { (self.pos - rate * dt).max(0.0) };
                }
            } else if self.doors > 0.0 {
                self.doors = (self.doors - door_rate * dt).max(0.0);
            }
        }
        // Travel ends on the locks, not a rounding error short of them.
        if self.pos < 1e-9 {
            self.pos = 0.0;
        } else if self.pos > 1.0 - 1e-9 {
            self.pos = 1.0;
        }
        let wow = self.ground.is_some_and(|g| slots[g] != 0.0) && self.pos >= 1.0;
        let demand = if powered && !free { (self.pos - before).abs() / dt.max(1e-9) * self.swept } else { 0.0 };
        let s = &self.s;
        s.put(slots, "position", self.pos);
        s.flag(slots, "down_locked", self.pos >= 1.0);
        s.flag(slots, "up_locked", self.pos <= 0.0);
        s.flag(slots, "in_transit", self.pos > 0.0 && self.pos < 1.0);
        s.put(slots, "doors", self.doors);
        s.flag(slots, "doors_closed", self.doors <= 0.0);
        s.flag(slots, "disagree", (want && self.pos < 1.0) || (!want && self.pos > 0.0));
        s.put(slots, "demand", demand);
        s.flag(slots, "weight_on_wheels", wow);
    }

    pub fn state(&self, v: &mut Vec<f64>) {
        v.extend([self.pos, self.doors]);
    }

    pub fn restore(&mut self, it: &mut impl Iterator<Item = f64>) {
        self.pos = it.next().unwrap_or(self.pos);
        self.doors = it.next().unwrap_or(0.0);
    }
}

pub struct Brake {
    circuit: Option<usize>,
    pedal: Option<Expr>,
    fan: Option<Expr>,
    air: [Option<usize>; 3],
    max: f64,
    heating: f64,
    cooling: f64,
    mass: f64,
    carbon: bool,
    max_torque: f64,
    radius: f64,
    wheels: f64,
    fuse: f64,
    hot: f64,
    wear_rate: f64,
    pub temp: f64,
    energy: f64,
    wear: f64,
    melted: bool,
    s: Slots,
}

impl Brake {
    pub fn new(c: &Checked, p: &CheckedPart) -> Self {
        Brake {
            circuit: slot(c, p, "circuit", "pressure"),
            pedal: expr_of(p, "pedal"),
            fan: expr_of(p, "fan_when"),
            air: [slot(c, p, "air", "groundspeed"), slot(c, p, "air", "airspeed"), slot(c, p, "air", "temperature")],
            max: p.num("max_pressure"),
            heating: p.num("heating"),
            cooling: p.num("cooling").max(1.0),
            mass: p.num("heat_sink").max(0.0),
            carbon: !p.text("material").eq_ignore_ascii_case("steel"),
            max_torque: p.num("max_torque").max(0.0),
            radius: p.num("tyre_radius").max(0.1),
            wheels: p.num("wheels").max(1.0),
            fuse: p.num("fuse_temperature"),
            hot: p.num("hot_temperature"),
            wear_rate: p.num("wear_per_mj").max(0.0),
            temp: 288.15,
            energy: 0.0,
            wear: 0.0,
            melted: false,
            s: Slots::of(p, &["pressure", "temperature", "hot", "torque", "force", "energy", "friction", "wear", "fuse_melted", "failed"]),
        }
    }

    /// Pad friction against temperature: carbon gains a little hot and fades past 1000 C; steel fades sooner.
    fn friction(&self) -> f64 {
        let c = self.temp - 273.15;
        if self.carbon {
            (0.85 + 0.15 * (c / 400.0).min(1.0) - 0.5 * ((c - 1000.0) / 800.0).clamp(0.0, 1.0)).max(0.2)
        } else {
            (1.0 - 0.6 * ((c - 400.0) / 500.0).clamp(0.0, 1.0)).max(0.2)
        }
    }

    pub fn step(&mut self, slots: &mut [f64], dt: f64) {
        let demand = self.pedal.as_mut().map_or(0.0, |e| e.eval(slots, dt)).clamp(0.0, 1.0);
        let fans = self.fan.as_mut().is_some_and(|e| e.yes(slots, dt));
        let supply = self.circuit.map_or(self.max, |c| slots[c]);
        let p = if self.s.on(slots, "failed") { 0.0 } else { (demand * self.max).min(supply.max(0.0)) };
        let gs = self.air[0].map_or(0.0, |g| slots[g]).abs();
        let airspeed = self.air[1].map_or(0.0, |v| slots[v]).abs();
        let t_air = self.air[2].map_or(288.15, |t| slots[t]).max(150.0);
        let mu = self.friction();
        let share = p / self.max.max(1.0);
        let (torque, power) = if self.max_torque > 0.0 {
            let torque = self.max_torque * share * mu;
            let omega = gs / self.radius;
            (torque, torque * self.wheels * omega)
        } else {
            (0.0, 0.0)
        };
        if self.mass > 0.0 {
            // Heat sink: absorbed power less convection (forced by airflow, fans) and radiation.
            let cp = if self.carbon { 1420.0 } else { 500.0 };
            let area = 0.0025 * self.mass.powf(2.0 / 3.0) * 40.0;
            let h = 10.0 + 4.0 * (airspeed.max(gs)).sqrt() + if fans { 60.0 } else { 0.0 };
            let loss = h * area * (self.temp - t_air) + 0.8 * SIGMA * area * (self.temp.powi(4) - t_air.powi(4));
            let heat = if self.max_torque > 0.0 { power } else { self.heating * share * (gs / 50.0) * self.mass * cp };
            self.temp += (heat - loss) / (self.mass * cp) * dt;
            self.energy += heat * dt;
            self.wear += heat * dt / 1e6 * self.wear_rate;
        } else {
            let heat = self.heating * share * (gs / 50.0) * mu;
            let cool = if fans { 3.0 } else { 1.0 };
            self.temp += (heat - (self.temp - t_air) * cool / self.cooling) * dt;
            self.energy += power * dt;
            self.wear += power * dt / 1e6 * self.wear_rate;
        }
        if self.temp >= self.fuse {
            self.melted = true;
        }
        let s = &self.s;
        s.put(slots, "pressure", p);
        s.put(slots, "temperature", self.temp);
        s.flag(slots, "hot", self.temp > self.hot);
        s.put(slots, "torque", torque * self.wheels);
        s.put(slots, "force", torque * self.wheels / self.radius);
        s.put(slots, "energy", self.energy);
        s.put(slots, "friction", mu);
        s.put(slots, "wear", self.wear);
        s.flag(slots, "fuse_melted", self.melted);
    }

    pub fn state(&self, v: &mut Vec<f64>) {
        v.extend([self.temp, self.energy, self.wear, self.melted as u8 as f64]);
    }

    pub fn restore(&mut self, it: &mut impl Iterator<Item = f64>) {
        self.temp = it.next().unwrap_or(288.15);
        self.energy = it.next().unwrap_or(0.0);
        self.wear = it.next().unwrap_or(0.0);
        self.melted = it.next().unwrap_or(0.0) != 0.0;
    }
}

pub struct Tyre {
    brake: [Option<usize>; 2],
    air: [Option<usize>; 2],
    burst_e: Option<Expr>,
    cold: f64,
    leak: f64,
    low: f64,
    gas: f64,
    temp: f64,
    burst: bool,
    deflated: bool,
    s: Slots,
}

impl Tyre {
    pub fn new(c: &Checked, p: &CheckedPart) -> Self {
        let cold = p.num("pressure").max(1.0);
        Tyre {
            brake: [slot(c, p, "brake", "temperature"), slot(c, p, "brake", "fuse_melted")],
            air: [slot(c, p, "air", "groundspeed"), slot(c, p, "air", "temperature")],
            burst_e: expr_of(p, "burst_when"),
            cold,
            leak: p.num("leak").max(0.0) / 86_400.0,
            low: p.num("low"),
            gas: 1.0,
            temp: 288.15,
            burst: false,
            deflated: false,
            s: Slots::of(p, &["pressure", "temperature", "low", "deflated", "burst", "failed"]),
        }
    }

    pub fn step(&mut self, slots: &mut [f64], dt: f64) {
        let t_air = self.air[1].map_or(288.15, |t| slots[t]).max(150.0);
        let gs = self.air[0].map_or(0.0, |g| slots[g]).abs();
        let brake_t = self.brake[0].map_or(t_air, |b| slots[b]);
        // Gas temperature: soaks toward the brake (wheel conduction, minutes) and warms with rolling flex.
        let soak = (brake_t - self.temp) / 900.0 + (t_air - self.temp) / 1800.0 + gs * 0.002;
        self.temp += soak * dt;
        if self.burst_e.as_mut().is_some_and(|e| e.yes(slots, dt)) || self.s.on(slots, "failed") {
            self.burst = true;
        }
        if self.brake[1].is_some_and(|f| slots[f] != 0.0) {
            self.deflated = true;
        }
        self.gas = if self.burst || self.deflated { (self.gas - dt / 20.0).max(0.0) } else { (self.gas - self.leak * dt).max(0.0) };
        // Gauge pressure from the gas law at the tyre's own volume, taking absolute pressure through it.
        let absolute = (self.cold + 101_325.0) * self.gas * self.temp / 288.15;
        let p = (absolute - 101_325.0).max(0.0);
        let s = &self.s;
        s.put(slots, "pressure", p);
        s.put(slots, "temperature", self.temp);
        s.flag(slots, "low", p < self.low * self.cold);
        s.flag(slots, "deflated", p < 0.1 * self.cold);
        s.flag(slots, "burst", self.burst);
    }

    pub fn state(&self, v: &mut Vec<f64>) {
        v.extend([self.gas, self.temp, self.burst as u8 as f64, self.deflated as u8 as f64]);
    }

    pub fn restore(&mut self, it: &mut impl Iterator<Item = f64>) {
        self.gas = it.next().unwrap_or(1.0);
        self.temp = it.next().unwrap_or(288.15);
        self.burst = it.next().unwrap_or(0.0) != 0.0;
        self.deflated = it.next().unwrap_or(0.0) != 0.0;
    }
}

pub struct Steering {
    circuit: Option<(usize, f64)>,
    gs: Option<usize>,
    e: [Option<Expr>; 4],
    tiller: f64,
    pedal: f64,
    full: f64,
    none: f64,
    rate: f64,
    angle: f64,
    s: Slots,
}

impl Steering {
    pub fn new(c: &Checked, p: &CheckedPart) -> Self {
        Steering {
            circuit: circuit_share(c, p),
            gs: slot(c, p, "air", "groundspeed"),
            e: ["powered_when", "tiller", "pedals", "towing_when"].map(|k| expr_of(p, k)),
            tiller: p.num("tiller_angle"),
            pedal: p.num("pedal_angle"),
            full: p.num("full_until"),
            none: p.num("none_above").max(p.num("full_until") + 0.1),
            rate: p.num("rate").max(0.1),
            angle: 0.0,
            s: Slots::of(p, &["angle", "command", "authority", "castoring", "failed"]),
        }
    }

    pub fn step(&mut self, slots: &mut [f64], dt: f64) {
        let ev = |e: &mut Option<Expr>, s: &[f64]| e.as_mut().map_or(0.0, |x| x.eval(s, dt));
        let engaged = self.e[0].as_mut().is_none_or(|x| x.yes(slots, dt));
        let tiller = ev(&mut self.e[1], slots).clamp(-1.0, 1.0);
        let pedals = ev(&mut self.e[2], slots).clamp(-1.0, 1.0);
        let towing = ev(&mut self.e[3], slots) != 0.0;
        let pressure = self.circuit.map_or(1.0, |(c, nominal)| slots[c] / nominal);
        let gs = self.gs.map_or(0.0, |g| slots[g]).abs();
        let powered = engaged && pressure > 0.5 && !towing && !self.s.on(slots, "failed");
        let authority = 1.0 - ((gs - self.full) / (self.none - self.full)).clamp(0.0, 1.0);
        let command = (tiller * self.tiller * authority + pedals * self.pedal).clamp(-self.tiller, self.tiller);
        if powered {
            let step = self.rate * pressure.min(1.0) * dt;
            self.angle += (command - self.angle).clamp(-step, step);
        } else {
            // Castoring: the wheel trails back toward centre as the aircraft rolls.
            self.angle -= self.angle * (gs * 0.05 * dt).min(1.0);
        }
        let s = &self.s;
        s.put(slots, "angle", self.angle);
        s.put(slots, "command", command);
        s.put(slots, "authority", authority);
        s.flag(slots, "castoring", !powered);
    }

    pub fn state(&self, v: &mut Vec<f64>) {
        v.push(self.angle);
    }

    pub fn restore(&mut self, it: &mut impl Iterator<Item = f64>) {
        self.angle = it.next().unwrap_or(0.0);
    }
}
