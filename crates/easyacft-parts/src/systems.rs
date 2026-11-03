//! Environment, landing gear, brakes, pressurization, ice protection and oxygen.

use crate::catalog::out;
use crate::util::{Packer, Slots};
use crate::{expr_of, Domain, Expr};
use easyacft_core::Dim;
use easyacft_lang::schema::{opt, req, Kind, PartSpec};
use easyacft_lang::{Checked, CheckedPart, Type};

const B: Type = Type::Bool;
const N: Type = Type::Num(Dim::NONE);
const P: Type = Type::Num(Dim::PRESSURE);
const T: Type = Type::Num(Dim::TEMPERATURE);
const LEN: Type = Type::Num(Dim::LENGTH);
const SPD: Type = Type::Num(Dim::SPEED);
const TIME: Type = Type::Num(Dim::TIME);
const WHEN: Kind = Kind::Expr(Type::Bool);
const AIR: &[&str] = &["environment.air"];
const CIRCUIT: &[&str] = &["hydraulic.circuit"];

pub static SPECS: &[PartSpec] = &[
    PartSpec {
        domain: "environment",
        kind: "air",
        help: "Outside air and flight state; bind these from X-Plane.",
        fields: &[],
        outputs: &[("pressure", P), ("temperature", T), ("altitude", LEN), ("airspeed", SPD), ("groundspeed", SPD), ("mach", N), ("on_ground", B), ("icing", N), ("rain", N)],
    },
    PartSpec {
        domain: "gear",
        kind: "leg",
        help: "A landing gear leg with its doors and locks.",
        fields: &[
            opt("circuit", Kind::Ref(CIRCUIT), "", "hydraulic circuit that moves it; none for electric"),
            req("down_when", WHEN, "the gear lever, for example \"gear_lever.on\""),
            opt("free_fall_when", WHEN, "false", "alternate gravity extension"),
            opt("time", Kind::Quantity(Dim::TIME), "10 s", "extension or retraction time"),
            opt("initial_down", Kind::Bool, "true", "down at start"),
        ],
        outputs: &[("position", N), ("down_locked", B), ("up_locked", B), ("in_transit", B), ("failed", B)],
    },
    PartSpec {
        domain: "gear",
        kind: "brake",
        help: "Wheel brakes with temperature.",
        fields: &[
            opt("circuit", Kind::Ref(CIRCUIT), "", "hydraulic circuit; none for an electric or accumulator-only brake"),
            req("pedal", Kind::Expr(N), "demand 0 to 1, for example \"left_pedal.position\""),
            req("max_pressure", Kind::Quantity(Dim::PRESSURE), "pressure at full pedal"),
            opt("air", Kind::Ref(AIR), "", "for groundspeed"),
            opt("heating", Kind::Number, "300", "kelvin rise per second at full brake and 50 m/s"),
            opt("cooling", Kind::Quantity(Dim::TIME), "600 s", "cooling time constant"),
        ],
        outputs: &[("pressure", P), ("temperature", T), ("hot", B), ("failed", B)],
    },
    PartSpec {
        domain: "cabin",
        kind: "pressurization",
        help: "Cabin pressure from pack inflow, leakage and the outflow valve.",
        fields: &[
            req("air", Kind::Ref(AIR), "outside air"),
            req("volume", Kind::Quantity(Dim::VOLUME), "pressurised volume"),
            req("inflow", Kind::Expr(Type::Num(Dim::MASS_FLOW)), "pack flow, for example \"pack1.flow + pack2.flow\""),
            opt("max_differential", Kind::Quantity(Dim::PRESSURE), "8.6 psi", "safety valve setting"),
            opt("cabin_ceiling", Kind::Quantity(Dim::LENGTH), "8000 ft", "cabin altitude the controller holds at the highest"),
            opt("rate", Kind::Quantity(Dim::SPEED), "2.5 m/s", "fastest cabin altitude change"),
            opt("leak", Kind::Number, "0.002", "leak area, m2"),
            opt("manual_when", WHEN, "false", "outflow valve in manual"),
            opt("manual_open", Kind::Expr(N), "0", "manual outflow valve opening 0 to 1"),
        ],
        outputs: &[("pressure", P), ("altitude", LEN), ("differential", P), ("vertical_speed", SPD), ("outflow", N), ("excess_altitude", B)],
    },
    PartSpec {
        domain: "ice",
        kind: "surface",
        help: "A wing, engine inlet, probe or window that ices and can be heated.",
        fields: &[
            req("air", Kind::Ref(AIR), "outside air, for its icing and temperature"),
            opt("heated_when", WHEN, "false", "when its protection runs"),
            opt("build_time", Kind::Quantity(Dim::TIME), "300 s", "time to full ice at full icing"),
            opt("shed_time", Kind::Quantity(Dim::TIME), "30 s", "time to clear when heated"),
        ],
        outputs: &[("ice", N), ("heated", B), ("iced", B), ("failed", B)],
    },
    PartSpec {
        domain: "oxygen",
        kind: "crew",
        help: "A crew oxygen bottle.",
        fields: &[
            req("pressure", Kind::Quantity(Dim::PRESSURE), "full bottle pressure, for example \"1850 psi\""),
            opt("duration", Kind::Quantity(Dim::TIME), "2 h", "time to empty with one mask on"),
            opt("masks_on", Kind::Expr(N), "0", "masks in use"),
        ],
        outputs: &[("pressure", P), ("low", B), ("failed", B)],
    },
    PartSpec {
        domain: "oxygen",
        kind: "passenger",
        help: "Passenger masks with chemical generators.",
        fields: &[
            opt("deploy_when", WHEN, "false", "for example \"cabin.altitude > 14000 ft\""),
            opt("duration", Kind::Quantity(Dim::TIME), "15 min", "generator burn time"),
        ],
        outputs: &[("deployed", B), ("remaining", TIME), ("failed", B)],
    },
];

const R_AIR: f64 = 287.05;

fn isa_altitude(p: f64) -> f64 {
    44_330.8 * (1.0 - (p.max(1.0) / 101_325.0).powf(0.190_263))
}

fn isa_pressure(h: f64) -> f64 {
    101_325.0 * (1.0 - h / 44_330.8).max(0.0).powf(5.255_88)
}

struct Leg {
    circuit: Option<usize>,
    down: Option<Expr>,
    free: Option<Expr>,
    time: f64,
    pos: f64,
    s: Slots,
}

struct Brake {
    circuit: Option<usize>,
    pedal: Option<Expr>,
    max: f64,
    gs: Option<usize>,
    heating: f64,
    cooling: f64,
    temp: f64,
    s: Slots,
}

struct Press {
    air: [usize; 2],
    volume: f64,
    inflow: Option<Expr>,
    max_diff: f64,
    ceiling: f64,
    rate: f64,
    leak: f64,
    manual: Option<Expr>,
    manual_open: Option<Expr>,
    p: f64,
    target_h: f64,
    open: f64,
    s: Slots,
}

struct Ice {
    icing: usize,
    temp: usize,
    heated: Option<Expr>,
    build: f64,
    shed: f64,
    ice: f64,
    s: Slots,
}

pub struct Systems {
    legs: Vec<Leg>,
    brakes: Vec<Brake>,
    press: Vec<Press>,
    ice: Vec<Ice>,
    crew: Vec<(f64, f64, Option<Expr>, f64, Slots)>,
    pax: Vec<(Option<Expr>, f64, f64, bool, Slots)>,
}

pub fn init(c: &Checked, slots: &mut [f64]) {
    for p in &c.parts {
        match (p.domain, p.kind) {
            ("environment", "air") => {
                slots[out(p, "pressure")] = 101_325.0;
                slots[out(p, "temperature")] = 288.15;
                slots[out(p, "on_ground")] = 1.0;
            }
            ("gear", "leg") if p.flag("initial_down") => {
                slots[out(p, "position")] = 1.0;
                slots[out(p, "down_locked")] = 1.0;
            }
            ("oxygen", "crew") => slots[out(p, "pressure")] = p.num("pressure"),
            ("cabin", "pressurization") => slots[out(p, "pressure")] = 101_325.0,
            _ => {}
        }
    }
}

fn slot_of(c: &Checked, p: &CheckedPart, field: &str, output: &str) -> Option<usize> {
    p.part(field).and_then(|i| c.slots.index.get(&format!("{}.{output}", c.parts[i].id)).copied())
}

impl Systems {
    pub fn new(c: &Checked) -> Option<Self> {
        let mut s = Systems { legs: vec![], brakes: vec![], press: vec![], ice: vec![], crew: vec![], pax: vec![] };
        for p in &c.parts {
            match (p.domain, p.kind) {
                ("gear", "leg") => s.legs.push(Leg {
                    circuit: slot_of(c, p, "circuit", "available"),
                    down: expr_of(p, "down_when"),
                    free: expr_of(p, "free_fall_when"),
                    time: p.num("time").max(0.1),
                    pos: if p.flag("initial_down") { 1.0 } else { 0.0 },
                    s: Slots::of(p, &["position", "down_locked", "up_locked", "in_transit", "failed"]),
                }),
                ("gear", "brake") => s.brakes.push(Brake {
                    circuit: slot_of(c, p, "circuit", "pressure"),
                    pedal: expr_of(p, "pedal"),
                    max: p.num("max_pressure"),
                    gs: slot_of(c, p, "air", "groundspeed"),
                    heating: p.num("heating"),
                    cooling: p.num("cooling").max(1.0),
                    temp: 288.15,
                    s: Slots::of(p, &["pressure", "temperature", "hot", "failed"]),
                }),
                ("cabin", "pressurization") => {
                    let (Some(pa), Some(ta)) = (slot_of(c, p, "air", "pressure"), slot_of(c, p, "air", "temperature")) else { continue };
                    s.press.push(Press {
                        air: [pa, ta],
                        volume: p.num("volume").max(1.0),
                        inflow: expr_of(p, "inflow"),
                        max_diff: p.num("max_differential"),
                        ceiling: p.num("cabin_ceiling"),
                        rate: p.num("rate").max(0.01),
                        leak: p.num("leak"),
                        manual: expr_of(p, "manual_when"),
                        manual_open: expr_of(p, "manual_open"),
                        p: 101_325.0,
                        target_h: 0.0,
                        open: 1.0,
                        s: Slots::of(p, &["pressure", "altitude", "differential", "vertical_speed", "outflow", "excess_altitude"]),
                    })
                }
                ("ice", "surface") => {
                    let (Some(icing), Some(temp)) = (slot_of(c, p, "air", "icing"), slot_of(c, p, "air", "temperature")) else { continue };
                    s.ice.push(Ice { icing, temp, heated: expr_of(p, "heated_when"), build: p.num("build_time").max(1.0), shed: p.num("shed_time").max(1.0), ice: 0.0, s: Slots::of(p, &["ice", "heated", "iced", "failed"]) });
                }
                ("oxygen", "crew") => s.crew.push((p.num("pressure"), p.num("duration").max(1.0), expr_of(p, "masks_on"), p.num("pressure"), Slots::of(p, &["pressure", "low", "failed"]))),
                ("oxygen", "passenger") => s.pax.push((expr_of(p, "deploy_when"), p.num("duration"), p.num("duration"), false, Slots::of(p, &["deployed", "remaining", "failed"]))),
                _ => {}
            }
        }
        let empty = s.legs.is_empty() && s.brakes.is_empty() && s.press.is_empty() && s.ice.is_empty() && s.crew.is_empty() && s.pax.is_empty();
        (!empty).then_some(s)
    }
}

impl Domain for Systems {
    fn name(&self) -> &'static str {
        "systems"
    }

    fn step(&mut self, slots: &mut [f64], dt: f64) {
        for l in &mut self.legs {
            let want = l.down.as_mut().is_some_and(|e| e.yes(slots, dt));
            let free = l.free.as_mut().is_some_and(|e| e.yes(slots, dt));
            let powered = l.circuit.is_none_or(|c| slots[c] != 0.0) && !l.s.on(slots, "failed");
            let rate = dt / l.time;
            if free {
                l.pos = (l.pos + rate * 1.5).min(1.0);
            } else if powered {
                l.pos = if want { (l.pos + rate).min(1.0) } else { (l.pos - rate).max(0.0) };
            }
            l.s.put(slots, "position", l.pos);
            l.s.flag(slots, "down_locked", l.pos >= 1.0);
            l.s.flag(slots, "up_locked", l.pos <= 0.0);
            l.s.flag(slots, "in_transit", l.pos > 0.0 && l.pos < 1.0);
        }
        for b in &mut self.brakes {
            let demand = b.pedal.as_mut().map_or(0.0, |e| e.eval(slots, dt)).clamp(0.0, 1.0);
            let supply = b.circuit.map_or(b.max, |c| slots[c]);
            let p = if b.s.on(slots, "failed") { 0.0 } else { (demand * b.max).min(supply) };
            let gs = b.gs.map_or(0.0, |g| slots[g]).abs();
            let heat = b.heating * (p / b.max.max(1.0)) * (gs / 50.0);
            b.temp += (heat - (b.temp - 288.15) / b.cooling) * dt;
            b.s.put(slots, "pressure", p);
            b.s.put(slots, "temperature", b.temp);
            b.s.flag(slots, "hot", b.temp > 573.15);
        }
        for c in &mut self.press {
            let (pa, ta) = (slots[c.air[0]], slots[c.air[1]].max(150.0));
            let inflow = c.inflow.as_mut().map_or(0.0, |e| e.eval(slots, dt)).max(0.0);
            let manual = c.manual.as_mut().is_some_and(|e| e.yes(slots, dt));
            let alt_air = isa_altitude(pa);
            let goal = (alt_air * 0.2).min(c.ceiling).min(alt_air);
            let step = c.rate * dt;
            c.target_h += (goal - c.target_h).clamp(-step, step);
            let p_target = isa_pressure(c.target_h).min(pa + c.max_diff);
            if manual {
                c.open = c.manual_open.as_mut().map_or(0.0, |e| e.eval(slots, dt)).clamp(0.0, 1.0);
            } else {
                c.open = (c.open + (c.p - p_target) / 2_000.0 * dt).clamp(0.0, 1.0);
            }
            let rho = c.p / (R_AIR * ta.max(270.0));
            let dp = (c.p - pa).max(0.0);
            let area = c.leak + c.open * 0.1;
            let mut outflow = area * (2.0 * rho * dp).sqrt();
            if c.p - pa > c.max_diff {
                outflow += 0.5 * (2.0 * rho * dp).sqrt();
            }
            if c.p < pa {
                outflow = -(c.leak + c.open * 0.1) * (2.0 * rho * (pa - c.p)).sqrt();
            }
            let prev_h = isa_altitude(c.p);
            c.p += (inflow - outflow) * R_AIR * ta.max(270.0) / c.volume * dt;
            c.p = c.p.max(1_000.0);
            let h = isa_altitude(c.p);
            c.s.put(slots, "pressure", c.p);
            c.s.put(slots, "altitude", h);
            c.s.put(slots, "differential", c.p - pa);
            c.s.put(slots, "vertical_speed", (h - prev_h) / dt.max(1e-6));
            c.s.put(slots, "outflow", c.open);
            c.s.flag(slots, "excess_altitude", h > 3_048.0);
        }
        for i in &mut self.ice {
            let heated = !i.s.on(slots, "failed") && i.heated.as_mut().is_some_and(|e| e.yes(slots, dt));
            let icing = slots[i.icing].clamp(0.0, 1.0) * if slots[i.temp] < 275.15 { 1.0 } else { 0.0 };
            i.ice = if heated { (i.ice - dt / i.shed).max(0.0) } else { (i.ice + icing * dt / i.build).min(1.0) };
            i.s.put(slots, "ice", i.ice);
            i.s.flag(slots, "heated", heated);
            i.s.flag(slots, "iced", i.ice > 0.2);
        }
        for (full, dur, masks, p, s) in &mut self.crew {
            let m = masks.as_mut().map_or(0.0, |e| e.eval(slots, dt)).max(0.0);
            *p = (*p - *full * m * dt / *dur).max(0.0);
            s.put(slots, "pressure", if s.on(slots, "failed") { 0.0 } else { *p });
            s.flag(slots, "low", *p < 0.3 * *full);
        }
        for (when, _, left, deployed, s) in &mut self.pax {
            if !*deployed && !s.on(slots, "failed") && when.as_mut().is_some_and(|e| e.yes(slots, dt)) {
                *deployed = true;
            }
            if *deployed {
                *left = (*left - dt).max(0.0);
            }
            s.flag(slots, "deployed", *deployed);
            s.put(slots, "remaining", *left);
        }
    }

    fn state(&self) -> Vec<f64> {
        let mut p = Packer::default();
        self.legs.iter().for_each(|l| p.put(l.pos));
        self.brakes.iter().for_each(|b| p.put(b.temp));
        for c in &self.press {
            p.put(c.p);
            p.put(c.target_h);
            p.put(c.open);
        }
        self.ice.iter().for_each(|i| p.put(i.ice));
        self.crew.iter().for_each(|c| p.put(c.3));
        for x in &self.pax {
            p.put(x.2);
            p.put(x.3 as u8 as f64);
        }
        p.0
    }

    fn restore(&mut self, s: &[f64]) {
        let mut p = Packer(s.to_vec(), 0);
        self.legs.iter_mut().for_each(|l| l.pos = p.take());
        self.brakes.iter_mut().for_each(|b| b.temp = p.take());
        for c in &mut self.press {
            c.p = p.take();
            c.target_h = p.take();
            c.open = p.take();
        }
        self.ice.iter_mut().for_each(|i| i.ice = p.take());
        self.crew.iter_mut().for_each(|c| c.3 = p.take());
        for x in &mut self.pax {
            x.2 = p.take();
            x.3 = p.take() != 0.0;
        }
    }
}
