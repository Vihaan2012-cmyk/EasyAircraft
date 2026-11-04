//! Avionics ventilation (blower and extract fans, skin heat exchanger, inlet, overboard and skin valves, open,
//! closed, intermediate and smoke configurations) and cargo compartment ventilation and heating.

use super::thermo::{self, Actuator, Confirm, Latch, CP};
use super::{eval, slot_of, yes};
use crate::util::Slots;
use crate::Expr;
use easyacft_core::Dim;
use easyacft_lang::schema::{opt, req, Kind, PartSpec};
use easyacft_lang::{Checked, CheckedPart, Type};

const B: Type = Type::Bool;
const N: Type = Type::Num(Dim::NONE);
const T: Type = Type::Num(Dim::TEMPERATURE);
const MF: Type = Type::Num(Dim::MASS_FLOW);
const W: Type = Type::Num(Dim::POWER);
const WHEN: Kind = Kind::Expr(Type::Bool);

pub const AVIONICS: PartSpec = PartSpec {
    domain: "aircon",
    kind: "avionics_ventilation",
    help: "Avionics bay cooling: blower and extract fans move air over the equipment; on the ground with a cool day it breathes outside air, in flight it recirculates through a skin heat exchanger, with smoke it extracts overboard.",
    fields: &[
        req("air", Kind::Ref(&["environment.air"]), "outside air"),
        req("fuselage", Kind::Ref(&["pressurization.fuselage"]), "the cabin it sits under"),
        opt("heat", Kind::Expr(W), "6 kW", "heat the equipment gives off"),
        opt("equipment_mass", Kind::Quantity(Dim::MASS), "400 kg", "racks and boxes, for their thermal inertia"),
        opt("flow", Kind::Quantity(Dim::MASS_FLOW), "0.4 kg/s", "blower flow at full speed"),
        opt("blower_power", Kind::Quantity(Dim::POWER), "1.1 kW", "blower motor power"),
        opt("extract_power", Kind::Quantity(Dim::POWER), "0.9 kW", "extract motor power"),
        opt("skin_ua", Kind::Number, "180", "skin heat exchanger conductance, W/K"),
        opt("blower_when", WHEN, "true", "blower selected on and powered"),
        opt("extract_when", WHEN, "true", "extract fan selected on and powered"),
        opt("override_when", WHEN, "false", "override: cabin air from the conditioned supply, extract overboard"),
        opt("smoke_when", WHEN, "false", "smoke in the bay"),
        opt("open_below", Kind::Quantity(Dim::TEMPERATURE), "285 K", "on the ground, open-circuit configuration below this outside temperature"),
        opt("valve_time", Kind::Quantity(Dim::TIME), "8 s", "valve travel time"),
        opt("overheat_temperature", Kind::Quantity(Dim::TEMPERATURE), "340 K", "equipment overheat warning"),
    ],
    outputs: &[
        ("air_temperature", T),
        ("equipment_temperature", T),
        ("flow", MF),
        ("configuration", N),
        ("inlet_valve", N),
        ("overboard_valve", N),
        ("skin_valve", N),
        ("blower_running", B),
        ("extract_running", B),
        ("power", W),
        ("smoke", B),
        ("overheat", B),
        ("fault", B),
        ("failed", B),
    ],
};

pub const CARGO: PartSpec = PartSpec {
    domain: "aircon",
    kind: "cargo",
    help: "A cargo compartment: inlet and outlet isolation valves, an extract fan, hot air and an electric heater under a temperature controller, and smoke isolation.",
    fields: &[
        req("fuselage", Kind::Ref(&["pressurization.fuselage"]), "pressurised fuselage it is in"),
        req("air", Kind::Ref(&["environment.air"]), "outside air"),
        opt("hot_air", Kind::Ref(&["aircon.hot_air"]), "", "hot air manifold for its heating"),
        opt("volume", Kind::Quantity(Dim::VOLUME), "40 m3", "compartment volume"),
        opt("selected", Kind::Expr(T), "288 K", "temperature selected"),
        opt("ventilated_when", WHEN, "true", "ventilation selected on"),
        opt("heater_power", Kind::Quantity(Dim::POWER), "3 kW", "electric heater"),
        opt("fan_flow", Kind::Quantity(Dim::MASS_FLOW), "0.15 kg/s", "extract fan flow"),
        opt("fan_power", Kind::Quantity(Dim::POWER), "0.4 kW", "extract fan power"),
        opt("hot_air_area", Kind::Number, "0.0006", "hot air valve area, m²"),
        opt("skin_ua", Kind::Number, "120", "heat lost through its walls and floor, W/K"),
        opt("cargo_heat", Kind::Expr(W), "0 W", "heat from the load, such as live animals"),
        opt("smoke_when", WHEN, "false", "smoke detected"),
        opt("valve_time", Kind::Quantity(Dim::TIME), "6 s", "isolation valve travel time"),
    ],
    outputs: &[("temperature", T), ("inlet_valve", N), ("outlet_valve", N), ("fan_running", B), ("heater_on", B), ("hot_air_flow", MF), ("power", W), ("isolated", B), ("smoke", B), ("fault", B), ("failed", B)],
};

const EQUIP_C: f64 = 700.0;
/// Air-to-equipment conductance per kg of equipment, W/K.
const EQUIP_UA_PER_KG: f64 = 2.5;

pub mod config {
    pub const CLOSED: f64 = 0.0;
    pub const OPEN: f64 = 1.0;
    pub const INTERMEDIATE: f64 = 2.0;
    pub const SMOKE: f64 = 3.0;
    pub const OVERRIDE: f64 = 4.0;
}

pub struct Avionics {
    pub fuselage: usize,
    air: [usize; 4],
    heat: Option<Expr>,
    equip_c: f64,
    equip_ua: f64,
    flow: f64,
    blower_p: f64,
    extract_p: f64,
    skin_ua: f64,
    blower: Option<Expr>,
    extract: Option<Expr>,
    over: Option<Expr>,
    smoke: Option<Expr>,
    open_below: f64,
    hot_limit: f64,
    inlet: Actuator,
    overboard: Actuator,
    skin: Actuator,
    air_t: f64,
    equip_t: f64,
    hot: Confirm,
    s: Slots,
}

impl Avionics {
    pub fn new(c: &Checked, p: &CheckedPart, fuselage: usize) -> Option<Self> {
        let mass = p.num("equipment_mass").max(1.0);
        let vt = p.num("valve_time");
        Some(Avionics {
            fuselage,
            air: [slot_of(c, p, "air", "temperature")?, slot_of(c, p, "air", "on_ground")?, slot_of(c, p, "air", "airspeed")?, slot_of(c, p, "air", "pressure")?],
            heat: crate::expr_of(p, "heat"),
            equip_c: mass * EQUIP_C,
            equip_ua: mass * EQUIP_UA_PER_KG,
            flow: p.num("flow").max(0.0),
            blower_p: p.num("blower_power"),
            extract_p: p.num("extract_power"),
            skin_ua: p.num("skin_ua").max(0.0),
            blower: crate::expr_of(p, "blower_when"),
            extract: crate::expr_of(p, "extract_when"),
            over: crate::expr_of(p, "override_when"),
            smoke: crate::expr_of(p, "smoke_when"),
            open_below: p.num("open_below"),
            hot_limit: p.num("overheat_temperature"),
            inlet: Actuator::new(0.0, vt),
            overboard: Actuator::new(0.0, vt),
            skin: Actuator::new(1.0, vt),
            air_t: 295.0,
            equip_t: 295.0,
            hot: Confirm::default(),
            s: Slots::of(p, &["air_temperature", "equipment_temperature", "flow", "configuration", "inlet_valve", "overboard_valve", "skin_valve", "blower_running", "extract_running", "power", "smoke", "overheat", "fault", "failed"]),
        })
    }

    pub fn step(&mut self, slots: &mut [f64], dt: f64, cabin_t: f64) {
        let failed = self.s.on(slots, "failed");
        let (ta, ground, tas) = (slots[self.air[0]].max(150.0), slots[self.air[1]] != 0.0, slots[self.air[2]].max(0.0));
        let skin_t = thermo::recovery_temperature(ta, tas, 0.89);
        let heat = eval(&mut self.heat, slots, dt, 6000.0);
        let blower = !failed && yes(&mut self.blower, slots, dt, true);
        let extract = !failed && yes(&mut self.extract, slots, dt, true);
        let overridden = yes(&mut self.over, slots, dt, false);
        let smoke = yes(&mut self.smoke, slots, dt, false);
        let cfg = if smoke {
            config::SMOKE
        } else if overridden || !blower {
            config::OVERRIDE
        } else if ground && ta < self.open_below {
            config::OPEN
        } else if ground {
            config::INTERMEDIATE
        } else {
            config::CLOSED
        };
        let (inlet, overboard, skin) = match cfg {
            x if x == config::OPEN => (1.0, 1.0, 0.0),
            x if x == config::INTERMEDIATE => (0.0, 0.5, 1.0),
            x if x == config::SMOKE => (0.0, 1.0, 0.0),
            x if x == config::OVERRIDE => (0.0, 1.0, 0.0),
            _ => (0.0, 0.0, 1.0),
        };
        let (i, o, k) = (self.inlet.drive(inlet, 1.0, dt), self.overboard.drive(overboard, 1.0, dt), self.skin.drive(skin, 1.0, dt));
        let moving = if blower && cfg != config::SMOKE { 1.0 } else if extract { 0.6 } else { 0.0 };
        let m = self.flow * moving;
        // Air entering the racks: outside air when open, cabin air in override or smoke, skin-cooled recirculation when closed.
        let fresh_t = ta.max(240.0);
        let supply_t = if cfg == config::OPEN {
            fresh_t * i + self.air_t * (1.0 - i)
        } else if cfg == config::OVERRIDE || cfg == config::SMOKE {
            cabin_t
        } else {
            let (cool, _, _) = thermo::exchanger(self.air_t, m, skin_t, 2.0 * m.max(0.05), self.skin_ua * k);
            cool * (1.0 - o) + cabin_t * o
        };
        let air_c = 30.0 * CP;
        let g = m * CP;
        self.air_t = (air_c * self.air_t + dt * (g * supply_t + self.equip_ua * self.equip_t)) / (air_c + dt * (g + self.equip_ua));
        self.equip_t += (heat - self.equip_ua * (self.equip_t - self.air_t)) * dt / self.equip_c;
        let over = self.hot.hold(self.equip_t > self.hot_limit, 5.0, dt);
        let power = if blower { self.blower_p } else { 0.0 } + if extract { self.extract_p } else { 0.0 };
        let s = &self.s;
        s.put(slots, "air_temperature", self.air_t);
        s.put(slots, "equipment_temperature", self.equip_t);
        s.put(slots, "flow", m);
        s.put(slots, "configuration", cfg);
        s.put(slots, "inlet_valve", i);
        s.put(slots, "overboard_valve", o);
        s.put(slots, "skin_valve", k);
        s.flag(slots, "blower_running", blower);
        s.flag(slots, "extract_running", extract);
        s.put(slots, "power", power);
        s.flag(slots, "smoke", smoke);
        s.flag(slots, "overheat", over);
        s.flag(slots, "fault", over || !blower || !extract);
    }

    pub fn state(&self, v: &mut Vec<f64>) {
        for a in [&self.inlet, &self.overboard, &self.skin] {
            v.extend(a.state());
        }
        v.extend([self.air_t, self.equip_t, self.hot.elapsed]);
    }

    pub fn restore(&mut self, it: &mut impl Iterator<Item = f64>) {
        let mut n = || it.next().unwrap_or(0.0);
        for a in [&mut self.inlet, &mut self.overboard, &mut self.skin] {
            a.restore([n(), n()]);
        }
        self.air_t = n();
        self.equip_t = n();
        self.hot.elapsed = n();
    }
}

pub struct Cargo {
    pub fuselage: usize,
    pub hot_air: Option<usize>,
    air: [usize; 2],
    volume: f64,
    selected: Option<Expr>,
    vent: Option<Expr>,
    heater_p: f64,
    fan_flow: f64,
    fan_p: f64,
    hot_cda: f64,
    skin_ua: f64,
    load: Option<Expr>,
    smoke: Option<Expr>,
    inlet: Actuator,
    outlet: Actuator,
    heater: bool,
    t: f64,
    isolated: Latch,
    /// Hot air flow it drew last step.
    pub hot_flow: f64,
    s: Slots,
}

impl Cargo {
    pub fn new(c: &Checked, p: &CheckedPart, fuselage: usize, hot_air: Option<usize>) -> Option<Self> {
        let vt = p.num("valve_time");
        Some(Cargo {
            fuselage,
            hot_air,
            air: [slot_of(c, p, "air", "temperature")?, slot_of(c, p, "air", "airspeed")?],
            volume: p.num("volume").max(1.0),
            selected: crate::expr_of(p, "selected"),
            vent: crate::expr_of(p, "ventilated_when"),
            heater_p: p.num("heater_power"),
            fan_flow: p.num("fan_flow"),
            fan_p: p.num("fan_power"),
            hot_cda: 0.8 * p.num("hot_air_area"),
            skin_ua: p.num("skin_ua"),
            load: crate::expr_of(p, "cargo_heat"),
            smoke: crate::expr_of(p, "smoke_when"),
            inlet: Actuator::new(0.0, vt),
            outlet: Actuator::new(0.0, vt),
            heater: false,
            t: 288.0,
            isolated: Latch::default(),
            hot_flow: 0.0,
            s: Slots::of(p, &["temperature", "inlet_valve", "outlet_valve", "fan_running", "heater_on", "hot_air_flow", "power", "isolated", "smoke", "fault", "failed"]),
        })
    }

    /// `cabin` is the cabin air it ventilates with; `hot` the hot air manifold (pressure, temperature) if any.
    pub fn step(&mut self, slots: &mut [f64], dt: f64, cabin: (f64, f64), hot: Option<(f64, f64)>) {
        let failed = self.s.on(slots, "failed");
        let (ta, tas) = (slots[self.air[0]].max(150.0), slots[self.air[1]].max(0.0));
        let skin_t = thermo::recovery_temperature(ta, tas, 0.89);
        let selected = eval(&mut self.selected, slots, dt, 288.0);
        let vent = !failed && yes(&mut self.vent, slots, dt, true);
        let smoke = yes(&mut self.smoke, slots, dt, false);
        let isolated = self.isolated.update(smoke, !vent);
        let open = vent && !isolated;
        let (i, o) = (self.inlet.drive(if open { 1.0 } else { 0.0 }, 1.0, dt), self.outlet.drive(if open { 1.0 } else { 0.0 }, 1.0, dt));
        let m = self.fan_flow * i.min(o);
        let (cabin_p, cabin_t) = cabin;
        let err = selected - self.t;
        self.heater = !failed && open && (if self.heater { err > -0.5 } else { err > 1.5 });
        let hot_open = open && err > 0.0;
        self.hot_flow = match hot {
            Some((hp, ht)) if hot_open => thermo::orifice(self.hot_cda * (err / 3.0).clamp(0.0, 1.0), hp, ht, cabin_p),
            _ => 0.0,
        };
        let hot_t = hot.map_or(cabin_t, |h| h.1);
        let load = eval(&mut self.load, slots, dt, 0.0);
        let air_c = (cabin_p * self.volume / (thermo::R * self.t)) * CP + 60.0 * self.volume * 1000.0;
        let heat = if self.heater { self.heater_p } else { 0.0 } + load + self.hot_flow * CP * (hot_t - self.t);
        let g = m * CP;
        self.t = (air_c * self.t + dt * (g * cabin_t + heat + self.skin_ua * skin_t)) / (air_c + dt * (g + self.skin_ua));
        let s = &self.s;
        s.put(slots, "temperature", self.t);
        s.put(slots, "inlet_valve", i);
        s.put(slots, "outlet_valve", o);
        s.flag(slots, "fan_running", open);
        s.flag(slots, "heater_on", self.heater);
        s.put(slots, "hot_air_flow", self.hot_flow);
        s.put(slots, "power", if open { self.fan_p } else { 0.0 } + if self.heater { self.heater_p } else { 0.0 });
        s.flag(slots, "isolated", isolated);
        s.flag(slots, "smoke", smoke);
        s.flag(slots, "fault", isolated && vent);
    }

    pub fn state(&self, v: &mut Vec<f64>) {
        v.extend(self.inlet.state());
        v.extend(self.outlet.state());
        v.extend([self.heater as u8 as f64, self.t, self.isolated.on as u8 as f64, self.hot_flow]);
    }

    pub fn restore(&mut self, it: &mut impl Iterator<Item = f64>) {
        let mut n = || it.next().unwrap_or(0.0);
        self.inlet.restore([n(), n()]);
        self.outlet.restore([n(), n()]);
        self.heater = n() != 0.0;
        self.t = n();
        self.isolated.on = n() != 0.0;
        self.hot_flow = n();
    }
}
