//! Oxygen: crew bottles whose pressure follows the gas law with their temperature, drawn by diluter-demand
//! masks whose oxygen share rises with cabin altitude, with 100 % and emergency overpressure modes and an
//! overboard discharge disc; passenger chemical generators started when masks are pulled, burning on a
//! falling flow profile.

use crate::util::Slots;
use crate::{expr_of, Expr};
use easyacft_core::Dim;
use easyacft_lang::schema::{opt, req, Kind, PartSpec};
use easyacft_lang::{Checked, CheckedPart, Type};

const B: Type = Type::Bool;
const N: Type = Type::Num(Dim::NONE);
const P: Type = Type::Num(Dim::PRESSURE);
const T: Type = Type::Num(Dim::TEMPERATURE);
const TIME: Type = Type::Num(Dim::TIME);
const VF: Type = Type::Num(Dim::VOLUME_FLOW);
const VOL: Type = Type::Num(Dim::VOLUME);
const WHEN: Kind = Kind::Expr(Type::Bool);

pub static SPECS: &[PartSpec] = &[
    PartSpec {
        domain: "oxygen",
        kind: "crew",
        help: "A crew oxygen bottle feeding diluter-demand masks.",
        fields: &[
            req("pressure", Kind::Quantity(Dim::PRESSURE), "full bottle pressure at 21 C, for example \"1850 psi\""),
            opt("duration", Kind::Quantity(Dim::TIME), "2 h", "time to empty with one mask on in normal mode at 8000 ft cabin, used when no capacity is given"),
            opt("capacity", Kind::Quantity(Dim::VOLUME), "0 L", "free oxygen at full pressure, litres at standard conditions; 0 uses duration"),
            opt("masks_on", Kind::Expr(N), "0", "masks in use"),
            opt("mode", Kind::Expr(N), "0", "regulator: 0 normal (diluted), 1 100 %, 2 emergency overpressure"),
            opt("cabin", Kind::Ref(&["cabin.pressurization"]), "", "cabin, for its altitude and temperature"),
            opt("minute_volume", Kind::Quantity(Dim::VOLUME_FLOW), "15 L/min", "breathing per crew member"),
            opt("low_pressure", Kind::Number, "0.3", "share of full pressure counted as low"),
            opt("burst_pressure", Kind::Quantity(Dim::PRESSURE), "2775 psi", "overboard discharge disc; over-temperature can burst it"),
            opt("supply_when", WHEN, "true", "the crew supply valve is open"),
            opt("service_when", WHEN, "false", "a ground refill"),
        ],
        outputs: &[("pressure", P), ("low", B), ("quantity", VOL), ("flow", VF), ("endurance", TIME), ("temperature", T), ("discharged", B), ("failed", B)],
    },
    PartSpec {
        domain: "oxygen",
        kind: "passenger",
        help: "Passenger masks with chemical generators: they drop when deployed and a generator starts when its mask is pulled.",
        fields: &[
            opt("deploy_when", WHEN, "false", "for example \"cabin.altitude > 14000 ft\""),
            opt("duration", Kind::Quantity(Dim::TIME), "15 min", "generator burn time"),
            opt("masks", Kind::Number, "1", "masks this part stands for"),
            opt("pulled", Kind::Expr(N), "1", "share of the dropped masks passengers pull"),
            opt("peak_flow", Kind::Quantity(Dim::VOLUME_FLOW), "4 L/min", "flow per mask at the start of the burn; it falls to a third by the end"),
        ],
        outputs: &[("deployed", B), ("remaining", TIME), ("flow", VF), ("active", N), ("exhausted", B), ("failed", B)],
    },
];

const PSI: f64 = 6894.757;

/// Oxygen share a diluter-demand regulator gives at a cabin pressure: air's own share near sea level,
/// rising so inspired oxygen pressure holds, 100 % from about 34 000 ft.
fn diluter_share(p_cabin: f64) -> f64 {
    let needed = 0.21 * 101_325.0 / p_cabin.max(1.0);
    needed.clamp(0.0, 1.0) * ((101_325.0 - p_cabin).max(0.0) / (101_325.0 - 25_000.0)).clamp(0.0, 1.0).sqrt().max(0.0)
}

pub struct Crew {
    cabin: [Option<usize>; 2],
    e: [Option<Expr>; 4],
    full: f64,
    capacity: f64,
    minute: f64,
    low: f64,
    burst: f64,
    gas: f64,
    temp: f64,
    discharged: bool,
    pub s: Slots,
}

impl Crew {
    pub fn new(c: &Checked, p: &CheckedPart) -> Self {
        let full = p.num("pressure").max(1.0);
        let minute = p.num("minute_volume").max(1e-6);
        let cap = p.num("capacity");
        let at_8000 = diluter_share(75_262.0) * minute * 75_262.0 / 101_325.0;
        Crew {
            cabin: [crate::gear::slot(c, p, "cabin", "pressure"), crate::gear::slot(c, p, "cabin", "temperature")],
            e: ["masks_on", "mode", "supply_when", "service_when"].map(|k| expr_of(p, k)),
            full,
            capacity: if cap > 0.0 { cap } else { at_8000.max(1e-9) * p.num("duration").max(1.0) },
            minute,
            low: p.num("low_pressure"),
            burst: p.num("burst_pressure"),
            gas: 1.0,
            temp: 294.15,
            discharged: false,
            s: Slots::of(p, &["pressure", "low", "quantity", "flow", "endurance", "temperature", "discharged", "failed"]),
        }
    }

    pub fn step(&mut self, slots: &mut [f64], dt: f64) {
        let ev = |e: &mut Option<Expr>, s: &[f64]| e.as_mut().map_or(0.0, |x| x.eval(s, dt));
        let masks = ev(&mut self.e[0], slots).max(0.0);
        let mode = ev(&mut self.e[1], slots).round();
        let supply = self.e[2].as_mut().is_none_or(|x| x.yes(slots, dt));
        if ev(&mut self.e[3], slots) != 0.0 {
            self.gas = 1.0;
            self.discharged = false;
        }
        let p_cab = self.cabin[0].map_or(75_262.0, |c| slots[c]).max(1000.0);
        let t_cab = self.cabin[1].map_or(294.15, |c| slots[c]);
        self.temp += (t_cab - self.temp) / 3600.0 * dt;
        let share = match mode as i64 {
            1 => 1.0,
            m if m >= 2 => 1.0,
            _ => diluter_share(p_cab),
        };
        // Oxygen drawn, at standard conditions: breathing scaled to cabin pressure, overpressure leaks round the mask.
        let extra = if mode >= 2.0 { 1.6 } else { 1.0 };
        let flow = if supply && !self.discharged { masks * self.minute * share * extra * p_cab / 101_325.0 } else { 0.0 };
        self.gas = (self.gas - flow * dt / self.capacity).max(0.0);
        let p = self.full * self.gas * self.temp / 294.15;
        if p > self.burst {
            self.discharged = true;
        }
        if self.discharged {
            self.gas = (self.gas - dt / 30.0).max(0.0);
        }
        let failed = self.s.on(slots, "failed");
        let shown = if failed { 0.0 } else { self.full * self.gas * self.temp / 294.15 };
        let s = &self.s;
        s.put(slots, "pressure", shown);
        s.flag(slots, "low", shown < self.low * self.full);
        s.put(slots, "quantity", self.capacity * self.gas);
        s.put(slots, "flow", flow);
        s.put(slots, "endurance", if flow > 0.0 { self.capacity * self.gas / flow } else { 1e9 });
        s.put(slots, "temperature", self.temp);
        s.flag(slots, "discharged", self.discharged);
    }

    pub fn state(&self, v: &mut Vec<f64>) {
        v.extend([self.gas, self.temp, self.discharged as u8 as f64]);
    }

    pub fn restore(&mut self, it: &mut impl Iterator<Item = f64>) {
        self.gas = it.next().unwrap_or(1.0);
        self.temp = it.next().unwrap_or(294.15);
        self.discharged = it.next().unwrap_or(0.0) != 0.0;
    }
}

pub struct Pax {
    deploy: Option<Expr>,
    pulled: Option<Expr>,
    duration: f64,
    masks: f64,
    peak: f64,
    deployed: bool,
    burn: Vec<f64>,
    pub s: Slots,
}

impl Pax {
    pub fn new(p: &CheckedPart) -> Self {
        Pax {
            deploy: expr_of(p, "deploy_when"),
            pulled: expr_of(p, "pulled"),
            duration: p.num("duration").max(1.0),
            masks: p.num("masks").max(1.0),
            peak: p.num("peak_flow").max(0.0),
            deployed: false,
            burn: Vec::new(),
            s: Slots::of(p, &["deployed", "remaining", "flow", "active", "exhausted", "failed"]),
        }
    }

    pub fn step(&mut self, slots: &mut [f64], dt: f64) {
        if !self.deployed && !self.s.on(slots, "failed") && self.deploy.as_mut().is_some_and(|e| e.yes(slots, dt)) {
            self.deployed = true;
        }
        let pulled = if self.deployed { self.pulled.as_mut().map_or(1.0, |e| e.eval(slots, dt)).clamp(0.0, 1.0) } else { 0.0 };
        // Each pulled mask fires its generator once; generators burn their full time whatever happens after.
        let started = (pulled * self.masks).round() as usize;
        while self.burn.len() < started {
            self.burn.push(0.0);
        }
        let mut flow = 0.0;
        let mut active = 0.0;
        for t in &mut self.burn {
            if *t < self.duration {
                let k = *t / self.duration;
                flow += self.peak * (1.0 - 2.0 / 3.0 * k);
                active += 1.0;
                *t += dt;
            }
        }
        let first = self.burn.first().copied().unwrap_or(0.0);
        let remaining = if self.deployed { (self.duration - first).max(0.0) } else { self.duration };
        let s = &self.s;
        s.flag(slots, "deployed", self.deployed);
        s.put(slots, "remaining", remaining);
        s.put(slots, "flow", flow);
        s.put(slots, "active", active);
        s.flag(slots, "exhausted", self.deployed && !self.burn.is_empty() && active == 0.0);
    }

    pub fn state(&self, v: &mut Vec<f64>) {
        v.push(self.deployed as u8 as f64);
        v.push(self.burn.len() as f64);
        v.extend(self.burn.iter().copied());
    }

    pub fn restore(&mut self, it: &mut impl Iterator<Item = f64>) {
        self.deployed = it.next().unwrap_or(0.0) != 0.0;
        let n = it.next().unwrap_or(0.0) as usize;
        self.burn = (0..n).map(|_| it.next().unwrap_or(0.0)).collect();
    }
}

pub const _PSI: f64 = PSI;
