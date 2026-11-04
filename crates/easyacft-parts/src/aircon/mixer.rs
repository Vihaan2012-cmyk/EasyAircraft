//! The mixer unit, recirculation fans, emergency ram air and the low-pressure ground connection.

use super::thermo::{self, lag, Actuator, Confirm, Stream, P_SL};
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
const MF: Type = Type::Num(Dim::MASS_FLOW);
const W: Type = Type::Num(Dim::POWER);
const WHEN: Kind = Kind::Expr(Type::Bool);

pub const MIXER: PartSpec = PartSpec {
    domain: "aircon",
    kind: "mixer",
    help: "The mixer unit: pack air, recirculated cabin air, emergency ram air and a low-pressure ground cart mix here before the zone ducts.",
    fields: &[
        req("packs", Kind::Refs(&["aircon.pack"]), "packs that feed it"),
        req("fuselage", Kind::Ref(&["pressurization.fuselage"]), "the pressurised fuselage it sits in"),
        opt("recirculation", Kind::Refs(&["aircon.recirc_fan"]), "", "recirculation fans that feed it"),
        opt("air", Kind::Ref(&["environment.air"]), "", "outside air, for emergency ram air"),
        opt("volume", Kind::Quantity(Dim::VOLUME), "1.2 m3", "air in the mixer and its ducts"),
        opt("duct_loss", Kind::Quantity(Dim::PRESSURE), "0.6 psi", "pressure from the mixer to the cabin at its design flow"),
        opt("design_flow", Kind::Quantity(Dim::MASS_FLOW), "4 kg/s", "flow the duct loss is given at"),
        opt("ram_air_when", WHEN, "false", "emergency ram air inlet commanded open"),
        opt("ram_air_area", Kind::Number, "0.015", "emergency ram air inlet area, m²"),
        opt("ram_air_time", Kind::Quantity(Dim::TIME), "10 s", "emergency ram air door travel time"),
        opt("ground_flow", Kind::Expr(MF), "0 kg/s", "a low-pressure ground air cart"),
        opt("ground_temperature", Kind::Expr(T), "283 K", "air from that cart"),
        opt("ground_humidity", Kind::Expr(N), "0.4", "relative humidity of that air"),
    ],
    outputs: &[
        ("temperature", T),
        ("pressure", P),
        ("flow", MF),
        ("humidity", N),
        ("co2", N),
        ("ozone", N),
        ("pack_flow", MF),
        ("recirculation_flow", MF),
        ("ram_air_flow", MF),
        ("ram_air_door", N),
        ("ground_flow", MF),
        ("fresh_share", N),
        ("failed", B),
    ],
};

pub const FAN: PartSpec = PartSpec {
    domain: "aircon",
    kind: "recirc_fan",
    help: "A recirculation fan drawing cabin air through its HEPA filter back to the mixer; its motor is an electrical load.",
    fields: &[
        opt("zones", Kind::Refs(&["aircon.zone"]), "", "zones it draws from; none means all of them"),
        opt("flow", Kind::Quantity(Dim::MASS_FLOW), "0.7 kg/s", "flow at full speed with a clean filter"),
        opt("power", Kind::Quantity(Dim::POWER), "2.2 kW", "motor power at full speed"),
        opt("on_when", WHEN, "true", "selected on"),
        opt("powered_when", WHEN, "true", "its motor has power"),
        opt("spool_time", Kind::Quantity(Dim::TIME), "6 s", "time to reach full speed"),
        opt("filter_hours", Kind::Quantity(Dim::TIME), "5000 h", "filter life to fully loaded"),
        opt("filter_loading", Kind::Number, "0.2", "filter loading at start, 0 clean to 1 blocked"),
        opt("overheat_when", WHEN, "false", "motor overheat: its thermal switch stops it"),
    ],
    outputs: &[("running", B), ("speed", N), ("flow", MF), ("power", W), ("filter_loading", N), ("filter_clogged", B), ("temperature", T), ("fault", B), ("failed", B)],
};

/// Outside air carbon dioxide, ppm.
pub const CO2_OUTSIDE: f64 = 420.0;

pub struct Mixer {
    pub packs: Vec<usize>,
    pub fans: Vec<usize>,
    pub fuselage: usize,
    air: Option<[usize; 4]>,
    volume: f64,
    loss: f64,
    design: f64,
    ram_when: Option<Expr>,
    ram_area: f64,
    ram_door: Actuator,
    ground: Option<Expr>,
    ground_t: Option<Expr>,
    ground_rh: Option<Expr>,
    pub t: f64,
    pub w: f64,
    pub co2: f64,
    /// Ozone, ppb; set from its inflows by the domain.
    pub ozone: f64,
    pub p: f64,
    /// Outside air it brought in this step: packs, ram air and ground air.
    pub fresh: f64,
    pub out: Stream,
    s: Slots,
}

impl Mixer {
    pub fn new(c: &Checked, p: &CheckedPart, packs: Vec<usize>, fans: Vec<usize>, fuselage: usize) -> Self {
        let air = (|| Some([slot_of(c, p, "air", "pressure")?, slot_of(c, p, "air", "temperature")?, slot_of(c, p, "air", "mach")?, slot_of(c, p, "air", "airspeed")?]))();
        Mixer {
            packs,
            fans,
            fuselage,
            air,
            volume: p.num("volume").max(0.01),
            loss: p.num("duct_loss").max(0.0),
            design: p.num("design_flow").max(0.01),
            ram_when: crate::expr_of(p, "ram_air_when"),
            ram_area: p.num("ram_air_area").max(0.0),
            ram_door: Actuator::new(0.0, p.num("ram_air_time")),
            ground: crate::expr_of(p, "ground_flow"),
            ground_t: crate::expr_of(p, "ground_temperature"),
            ground_rh: crate::expr_of(p, "ground_humidity"),
            t: 293.15,
            w: 0.006,
            co2: CO2_OUTSIDE,
            ozone: 30.0,
            p: P_SL,
            fresh: 0.0,
            out: Stream::none(293.15, P_SL),
            s: Slots::of(p, &["temperature", "pressure", "flow", "humidity", "co2", "ozone", "pack_flow", "recirculation_flow", "ram_air_flow", "ram_air_door", "ground_flow", "fresh_share", "failed"]),
        }
    }

    /// Mix this step's inflows. `packs` and `recirc` are already delivered streams; `recirc_co2` is their CO2 in ppm.
    pub fn step(&mut self, slots: &mut [f64], dt: f64, cabin_p: f64, packs: &[Stream], recirc: &[Stream], recirc_co2: f64, ambient_w: f64) {
        let ram_target = if yes(&mut self.ram_when, slots, dt, false) { 1.0 } else { 0.0 };
        let door = self.ram_door.drive(ram_target, 1.0, dt);
        let ram = match self.air {
            Some([pa, ta, mach, tas]) if door > 1e-3 => {
                let (pa, ta, mach, tas) = (slots[pa].max(100.0), slots[ta].max(150.0), slots[mach], slots[tas]);
                let p_ram = thermo::ram_pressure(pa, mach.max(0.0), 0.8);
                let t_ram = thermo::recovery_temperature(ta, tas.max(0.0), 1.0);
                let m = thermo::orifice(self.ram_area * door, p_ram, t_ram, cabin_p);
                Stream::new(m, t_ram, cabin_p, ambient_w)
            }
            _ => Stream::none(288.15, cabin_p),
        };
        let g_flow = eval(&mut self.ground, slots, dt, 0.0).max(0.0);
        let g_t = eval(&mut self.ground_t, slots, dt, 283.0);
        let g_w = thermo::humidity_ratio(eval(&mut self.ground_rh, slots, dt, 0.4), g_t, P_SL);
        let ground = Stream::new(g_flow, g_t, cabin_p, g_w);
        let pack_flow: f64 = packs.iter().map(|s| s.flow).sum();
        let recirc_flow: f64 = recirc.iter().map(|s| s.flow).sum();
        let mut all: Vec<Stream> = Vec::with_capacity(packs.len() + recirc.len() + 2);
        all.extend_from_slice(packs);
        all.extend_from_slice(recirc);
        all.push(ram);
        all.push(ground);
        let mixed = thermo::mix(&all);
        let fresh = pack_flow + ram.flow + ground.flow;
        self.fresh = fresh;
        let co2_in = if mixed.flow > 1e-9 { (fresh * CO2_OUTSIDE + recirc_flow * recirc_co2) / mixed.flow } else { self.co2 };
        // The mixer's own air smooths what it delivers.
        let mass = (cabin_p * self.volume / (thermo::R * self.t.max(150.0))).max(0.01);
        let tau = if mixed.flow > 1e-6 { mass / mixed.flow } else { 120.0 };
        if mixed.flow > 1e-6 {
            self.t = lag(self.t, mixed.t, tau, dt);
            self.w = lag(self.w, mixed.w, tau, dt);
            self.co2 = lag(self.co2, co2_in, tau, dt);
        } else {
            self.t = lag(self.t, self.t, tau, dt);
        }
        let share = mixed.flow / self.design;
        self.p = cabin_p + self.loss * share * share;
        self.out = Stream::new(mixed.flow, self.t, self.p, self.w);
        let s = &self.s;
        s.put(slots, "temperature", self.t);
        s.put(slots, "pressure", self.p);
        s.put(slots, "flow", mixed.flow);
        s.put(slots, "humidity", thermo::relative_humidity(self.w, self.t, self.p));
        s.put(slots, "co2", self.co2);
        s.put(slots, "ozone", self.ozone);
        s.put(slots, "pack_flow", pack_flow);
        s.put(slots, "recirculation_flow", recirc_flow);
        s.put(slots, "ram_air_flow", ram.flow);
        s.put(slots, "ram_air_door", door);
        s.put(slots, "ground_flow", ground.flow);
        s.put(slots, "fresh_share", if mixed.flow > 1e-9 { fresh / mixed.flow } else { 0.0 });
    }

    pub fn state(&self, v: &mut Vec<f64>) {
        v.extend(self.ram_door.state());
        v.extend([self.t, self.w, self.co2, self.p, self.ozone]);
    }

    pub fn restore(&mut self, it: &mut impl Iterator<Item = f64>) {
        let mut n = || it.next().unwrap_or(0.0);
        self.ram_door.restore([n(), n()]);
        self.t = n();
        self.w = n();
        self.co2 = n();
        self.p = n();
        self.ozone = n();
        self.out = Stream::new(0.0, self.t, self.p, self.w);
    }
}

pub struct Fan {
    pub zones: Vec<usize>,
    rated: f64,
    power: f64,
    on: Option<Expr>,
    powered: Option<Expr>,
    hot: Option<Expr>,
    spool: f64,
    life: f64,
    pub speed: f64,
    pub loading: f64,
    tripped: Confirm,
    pub out: Stream,
    s: Slots,
}

impl Fan {
    pub fn new(p: &CheckedPart, zones: Vec<usize>) -> Self {
        Fan {
            zones,
            rated: p.num("flow").max(0.0),
            power: p.num("power").max(0.0),
            on: crate::expr_of(p, "on_when"),
            powered: crate::expr_of(p, "powered_when"),
            hot: crate::expr_of(p, "overheat_when"),
            spool: p.num("spool_time").max(0.1),
            life: p.num("filter_hours").max(1.0),
            speed: 0.0,
            loading: p.num("filter_loading").clamp(0.0, 1.0),
            tripped: Confirm::default(),
            out: Stream::none(293.15, P_SL),
            s: Slots::of(p, &["running", "speed", "flow", "power", "filter_loading", "filter_clogged", "temperature", "fault", "failed"]),
        }
    }

    /// Run the fan on air drawn from its zones.
    pub fn step(&mut self, slots: &mut [f64], dt: f64, intake: Stream) {
        let failed = self.s.on(slots, "failed");
        let hot = yes(&mut self.hot, slots, dt, false);
        let tripped = self.tripped.hold(hot, 2.0, dt);
        let run = !failed && !tripped && yes(&mut self.on, slots, dt, true) && yes(&mut self.powered, slots, dt, true);
        let target = if run { 1.0 } else { 0.0 };
        self.speed = lag(self.speed, target, if run { self.spool / 3.0 } else { self.spool }, dt);
        if self.speed > 0.5 {
            self.loading = (self.loading + dt / (self.life * (0.3 + self.loading))).min(1.0);
        }
        // A loaded filter raises the system resistance: flow falls on the fan curve and the motor unloads a little.
        let resistance = 1.0 + 3.0 * self.loading.powi(2);
        let flow = self.rated * self.speed / resistance.sqrt();
        let power = if run { self.power * self.speed.powi(3) * (0.75 + 0.25 / resistance.sqrt()) } else { 0.0 };
        let rise = if flow > 1e-6 { power * 0.9 / (flow * thermo::CP) } else { 0.0 };
        self.out = Stream::new(flow, intake.t + rise, intake.p, intake.w);
        let s = &self.s;
        s.flag(slots, "running", run && self.speed > 0.8);
        s.put(slots, "speed", self.speed);
        s.put(slots, "flow", flow);
        s.put(slots, "power", power);
        s.put(slots, "filter_loading", self.loading);
        s.flag(slots, "filter_clogged", self.loading > 0.9);
        s.put(slots, "temperature", self.out.t);
        s.flag(slots, "fault", tripped || (yes(&mut self.on, slots, 0.0, true) && !run));
    }

    pub fn state(&self, v: &mut Vec<f64>) {
        v.extend([self.speed, self.loading, self.tripped.elapsed]);
    }

    pub fn restore(&mut self, it: &mut impl Iterator<Item = f64>) {
        let mut n = || it.next().unwrap_or(0.0);
        self.speed = n();
        self.loading = n();
        self.tripped.elapsed = n();
    }
}
