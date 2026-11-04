//! Hot (trim) air: the trim air pressure regulating and shutoff valve on the bleed, the hot air manifold, and
//! the trim air valves that add hot air to each zone's duct.

use super::thermo::{self, lag, Actuator, Confirm, Latch, Stream, P_SL, PSI};
use super::{slot_of, yes};
use crate::util::Slots;
use crate::Expr;
use easyacft_core::Dim;
use easyacft_lang::schema::{opt, req, Kind, PartSpec};
use easyacft_lang::{Checked, CheckedPart, Type};

const B: Type = Type::Bool;
const N: Type = Type::Num(Dim::NONE);
const P: Type = Type::Num(Dim::PRESSURE);
const T: Type = Type::Num(Dim::TEMPERATURE);
const MF: Type = Type::Num(Dim::MASS_FLOW);
const WHEN: Kind = Kind::Expr(Type::Bool);

pub const HOT_AIR: PartSpec = PartSpec {
    domain: "aircon",
    kind: "hot_air",
    help: "Trim air supply: a pressure regulating and shutoff valve taking bleed air upstream of the packs into a hot air manifold held a few psi above the cabin.",
    fields: &[
        req("duct", Kind::Ref(&["pneumatic.duct"]), "bleed duct it taps"),
        req("fuselage", Kind::Ref(&["pressurization.fuselage"]), "the cabin it is referenced to"),
        opt("on_when", WHEN, "true", "hot air push-button and the controller"),
        opt("regulate", Kind::Quantity(Dim::PRESSURE), "4 psi", "manifold pressure above the cabin it holds"),
        opt("overpressure", Kind::Quantity(Dim::PRESSURE), "8 psi", "manifold pressure above the cabin that shuts it"),
        opt("area", Kind::Number, "0.004", "valve flow area fully open, m²"),
        opt("volume", Kind::Quantity(Dim::VOLUME), "0.08 m3", "manifold volume"),
        opt("travel_time", Kind::Quantity(Dim::TIME), "5 s", "valve travel time"),
        opt("leak_when", WHEN, "false", "a manifold or duct leak"),
        opt("leak_area", Kind::Number, "0.001", "that leak, m²"),
        opt("jammed_when", WHEN, "false", "valve jammed where it is"),
    ],
    outputs: &[("open", B), ("position", N), ("pressure", P), ("temperature", T), ("flow", MF), ("leak_flow", MF), ("overpressure", B), ("fault", B), ("failed", B)],
};

pub const TRIM_VALVE: PartSpec = PartSpec {
    domain: "aircon",
    kind: "trim_valve",
    help: "A trim air valve: its zone controller opens it to warm the zone's supply duct with hot air.",
    fields: &[
        req("hot_air", Kind::Ref(&["aircon.hot_air"]), "hot air manifold"),
        opt("area", Kind::Number, "0.0012", "flow area fully open, m²"),
        opt("travel_time", Kind::Quantity(Dim::TIME), "8 s", "travel time"),
        opt("jammed_when", WHEN, "false", "jammed where it is"),
        opt("powered_when", WHEN, "true", "its motor has power"),
    ],
    outputs: &[("position", N), ("flow", MF), ("temperature", T), ("jammed", B), ("failed", B)],
};

pub struct HotAir {
    duct: [usize; 2],
    pub fuselage: usize,
    on: Option<Expr>,
    regulate: f64,
    overpressure: f64,
    cda: f64,
    volume: f64,
    valve: Actuator,
    leak_when: Option<Expr>,
    leak_cda: f64,
    jammed: Option<Expr>,
    over: Latch,
    over_t: Confirm,
    open_integral: f64,
    pub p: f64,
    pub t: f64,
    pub w: f64,
    /// Flow the trim valves drew last step.
    pub drawn: f64,
    s: Slots,
}

impl HotAir {
    pub fn new(c: &Checked, p: &CheckedPart, fuselage: usize) -> Option<Self> {
        Some(HotAir {
            duct: [slot_of(c, p, "duct", "pressure")?, slot_of(c, p, "duct", "temperature")?],
            fuselage,
            on: crate::expr_of(p, "on_when"),
            regulate: p.num("regulate"),
            overpressure: p.num("overpressure"),
            cda: 0.8 * p.num("area").max(0.0),
            volume: p.num("volume").max(0.001),
            valve: Actuator::new(0.0, p.num("travel_time")),
            leak_when: crate::expr_of(p, "leak_when"),
            leak_cda: 0.8 * p.num("leak_area").max(0.0),
            jammed: crate::expr_of(p, "jammed_when"),
            over: Latch::default(),
            over_t: Confirm::default(),
            open_integral: 0.0,
            p: P_SL,
            t: 293.15,
            w: 0.0,
            drawn: 0.0,
            s: Slots::of(p, &["open", "position", "pressure", "temperature", "flow", "leak_flow", "overpressure", "fault", "failed"]),
        })
    }

    pub fn step(&mut self, slots: &mut [f64], dt: f64, cabin_p: f64, ambient_w: f64) {
        let failed = self.s.on(slots, "failed");
        let (pb, tb) = (slots[self.duct[0]].max(0.0), slots[self.duct[1]].max(150.0));
        let commanded = !failed && yes(&mut self.on, slots, dt, true);
        let over_now = self.over_t.hold(self.p - cabin_p > self.overpressure, 0.5, dt);
        let over = self.over.update(over_now, !commanded);
        self.valve.jammed = yes(&mut self.jammed, slots, dt, false);
        // A pneumatic regulator settles far faster than anything here: it opens as far as the demand needs to hold
        // the manifold at its set pressure, and the pressure sags when even full open cannot pass the demand.
        let set_p = cabin_p + self.regulate;
        let supply_ok = pb > cabin_p + 2.0 * PSI;
        let leak = if yes(&mut self.leak_when, slots, dt, false) { thermo::orifice(self.leak_cda, self.p, self.t, cabin_p) } else { 0.0 };
        let need = self.drawn + leak;
        let regulating = commanded && !over && supply_ok;
        let full = thermo::orifice(self.cda, pb, tb, set_p.min(pb));
        let want = if regulating { if full > 1e-9 { (need / full).clamp(0.0, 1.0) } else { 1.0 } } else { 0.0 };
        let pos = self.valve.drive(want, 1.0, dt);
        let area = self.cda * pos;
        let p_static = if regulating && (need <= 1e-6 || pos <= 1e-3) {
            // Holding pressure with nothing flowing: the valve sits on its seat at the set pressure.
            set_p.min(pb)
        } else if pos > 1e-3 && pb > cabin_p {
            // Manifold pressure at which the valve passes the demand.
            let (mut lo, mut hi) = (cabin_p, pb);
            for _ in 0..40 {
                let mid = 0.5 * (lo + hi);
                if thermo::orifice(area, pb, tb, mid) > need { lo = mid } else { hi = mid }
            }
            let solved = 0.5 * (lo + hi);
            if self.valve.jammed || !regulating { solved } else { solved.min(set_p) }
        } else {
            cabin_p
        };
        let tau = (self.p * self.volume / (thermo::R * self.t.max(150.0)) / need.max(0.02)).clamp(0.05, 5.0);
        self.p = lag(self.p, p_static, tau, dt);
        let inflow = if pos > 1e-3 { thermo::orifice(area, pb, tb, self.p) } else { 0.0 };
        if inflow > 1e-6 {
            self.t = lag(self.t, tb, 2.0, dt);
            self.w = ambient_w;
        } else {
            self.t = lag(self.t, 300.0, 300.0, dt);
        }
        self.open_integral = pos;
        let s = &self.s;
        s.flag(slots, "open", pos > 1e-3);
        s.put(slots, "position", pos);
        s.put(slots, "pressure", self.p);
        s.put(slots, "temperature", self.t);
        s.put(slots, "flow", inflow);
        s.put(slots, "leak_flow", leak);
        s.flag(slots, "overpressure", over);
        s.flag(slots, "fault", over || (commanded && pos < 1e-3 && pb > cabin_p + 2.0 * PSI && self.valve.jammed));
    }

    pub fn state(&self, v: &mut Vec<f64>) {
        v.extend(self.valve.state());
        v.extend([self.over.on as u8 as f64, self.over_t.elapsed, self.open_integral, self.p, self.t, self.w, self.drawn]);
    }

    pub fn restore(&mut self, it: &mut impl Iterator<Item = f64>) {
        let mut n = || it.next().unwrap_or(0.0);
        self.valve.restore([n(), n()]);
        self.over.on = n() != 0.0;
        self.over_t.elapsed = n();
        self.open_integral = n();
        self.p = n();
        self.t = n();
        self.w = n();
        self.drawn = n();
    }
}

pub struct TrimValve {
    pub hot_air: usize,
    cda: f64,
    valve: Actuator,
    jammed: Option<Expr>,
    powered: Option<Expr>,
    /// Commanded position from the zone controller.
    pub command: f64,
    pub out: Stream,
    s: Slots,
}

impl TrimValve {
    pub fn new(p: &CheckedPart, hot_air: usize) -> Self {
        TrimValve {
            hot_air,
            cda: 0.8 * p.num("area").max(0.0),
            valve: Actuator::new(0.0, p.num("travel_time")),
            jammed: crate::expr_of(p, "jammed_when"),
            powered: crate::expr_of(p, "powered_when"),
            command: 0.0,
            out: Stream::none(293.15, P_SL),
            s: Slots::of(p, &["position", "flow", "temperature", "jammed", "failed"]),
        }
    }

    pub fn step(&mut self, slots: &mut [f64], dt: f64, manifold: (f64, f64, f64), duct_p: f64) {
        let failed = self.s.on(slots, "failed");
        self.valve.jammed = failed || yes(&mut self.jammed, slots, dt, false);
        let speed = if yes(&mut self.powered, slots, dt, true) { 1.0 } else { 0.0 };
        let pos = self.valve.drive(self.command, speed, dt);
        let (p, t, w) = manifold;
        let flow = thermo::orifice(self.cda * pos, p, t, duct_p);
        self.out = Stream::new(flow, t, duct_p, w);
        let s = &self.s;
        s.put(slots, "position", pos);
        s.put(slots, "flow", flow);
        s.put(slots, "temperature", t);
        s.flag(slots, "jammed", self.valve.jammed);
    }

    pub fn state(&self, v: &mut Vec<f64>) {
        v.extend(self.valve.state());
        v.push(self.command);
    }

    pub fn restore(&mut self, it: &mut impl Iterator<Item = f64>) {
        let mut n = || it.next().unwrap_or(0.0);
        self.valve.restore([n(), n()]);
        self.command = n();
    }
}
