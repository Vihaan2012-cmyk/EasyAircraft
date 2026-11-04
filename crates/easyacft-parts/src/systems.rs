//! The airframe systems domain: outside air, landing gear, brakes, tyres and steering, cabin pressure, ice and
//! oxygen. Each lives in its own module; this steps them together.

use crate::catalog::out;
use crate::{cabin, gear, ice, oxygen, Domain};
use easyacft_core::Dim;
use easyacft_lang::schema::PartSpec;
use easyacft_lang::{Checked, Type};

const B: Type = Type::Bool;
const N: Type = Type::Num(Dim::NONE);
const P: Type = Type::Num(Dim::PRESSURE);
const T: Type = Type::Num(Dim::TEMPERATURE);
const LEN: Type = Type::Num(Dim::LENGTH);
const SPD: Type = Type::Num(Dim::SPEED);

pub static AIR: &[PartSpec] = &[PartSpec {
    domain: "environment",
    kind: "air",
    help: "Outside air and flight state; bind these from X-Plane.",
    fields: &[],
    outputs: &[
        ("pressure", P),
        ("temperature", T),
        ("altitude", LEN),
        ("airspeed", SPD),
        ("groundspeed", SPD),
        ("vertical_speed", SPD),
        ("mach", N),
        ("on_ground", B),
        ("icing", N),
        ("rain", N),
        ("aoa", N),
        ("sideslip", N),
        ("heading", N),
        ("pitch", N),
        ("roll", N),
        ("latitude", N),
        ("longitude", N),
    ],
}];

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
            ("gear", "tyre") => slots[out(p, "pressure")] = p.num("pressure"),
            ("gear", "brake") => slots[out(p, "temperature")] = 288.15,
            ("oxygen", "crew") => slots[out(p, "pressure")] = p.num("pressure"),
            ("cabin", "pressurization") => {
                slots[out(p, "pressure")] = 101_325.0;
                slots[out(p, "temperature")] = 295.0;
            }
            _ => {}
        }
    }
}

#[derive(Default)]
pub struct Systems {
    legs: Vec<gear::Leg>,
    brakes: Vec<gear::Brake>,
    tyres: Vec<gear::Tyre>,
    steering: Vec<gear::Steering>,
    cabins: Vec<cabin::Cabin>,
    ice: Vec<ice::Ice>,
    crew: Vec<oxygen::Crew>,
    pax: Vec<oxygen::Pax>,
}

impl Systems {
    pub fn new(c: &Checked) -> Option<Self> {
        let mut s = Systems::default();
        for p in &c.parts {
            match (p.domain, p.kind) {
                ("gear", "leg") => s.legs.push(gear::Leg::new(c, p)),
                ("gear", "brake") => s.brakes.push(gear::Brake::new(c, p)),
                ("gear", "tyre") => s.tyres.push(gear::Tyre::new(c, p)),
                ("gear", "steering") => s.steering.push(gear::Steering::new(c, p)),
                ("cabin", "pressurization") => s.cabins.extend(cabin::Cabin::new(c, p)),
                ("ice", "surface") => s.ice.extend(ice::Ice::new(c, p)),
                ("oxygen", "crew") => s.crew.push(oxygen::Crew::new(c, p)),
                ("oxygen", "passenger") => s.pax.push(oxygen::Pax::new(p)),
                _ => {}
            }
        }
        let empty = s.legs.is_empty() && s.brakes.is_empty() && s.tyres.is_empty() && s.steering.is_empty() && s.cabins.is_empty() && s.ice.is_empty() && s.crew.is_empty() && s.pax.is_empty();
        (!empty).then_some(s)
    }
}

impl Domain for Systems {
    fn name(&self) -> &'static str {
        "systems"
    }

    fn step(&mut self, slots: &mut [f64], dt: f64) {
        self.legs.iter_mut().for_each(|x| x.step(slots, dt));
        self.brakes.iter_mut().for_each(|x| x.step(slots, dt));
        self.tyres.iter_mut().for_each(|x| x.step(slots, dt));
        self.steering.iter_mut().for_each(|x| x.step(slots, dt));
        self.cabins.iter_mut().for_each(|x| x.step(slots, dt));
        self.ice.iter_mut().for_each(|x| x.step(slots, dt));
        self.crew.iter_mut().for_each(|x| x.step(slots, dt));
        self.pax.iter_mut().for_each(|x| x.step(slots, dt));
    }

    fn state(&self) -> Vec<f64> {
        let mut v = Vec::new();
        self.legs.iter().for_each(|x| x.state(&mut v));
        self.brakes.iter().for_each(|x| x.state(&mut v));
        self.tyres.iter().for_each(|x| x.state(&mut v));
        self.steering.iter().for_each(|x| x.state(&mut v));
        self.cabins.iter().for_each(|x| x.state(&mut v));
        self.ice.iter().for_each(|x| x.state(&mut v));
        self.crew.iter().for_each(|x| x.state(&mut v));
        self.pax.iter().for_each(|x| x.state(&mut v));
        v
    }

    fn restore(&mut self, s: &[f64]) {
        let mut it = s.iter().copied();
        self.legs.iter_mut().for_each(|x| x.restore(&mut it));
        self.brakes.iter_mut().for_each(|x| x.restore(&mut it));
        self.tyres.iter_mut().for_each(|x| x.restore(&mut it));
        self.steering.iter_mut().for_each(|x| x.restore(&mut it));
        self.cabins.iter_mut().for_each(|x| x.restore(&mut it));
        self.ice.iter_mut().for_each(|x| x.restore(&mut it));
        self.crew.iter_mut().for_each(|x| x.restore(&mut it));
        self.pax.iter_mut().for_each(|x| x.restore(&mut it));
    }
}
