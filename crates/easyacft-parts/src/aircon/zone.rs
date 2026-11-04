//! A cabin or cockpit zone: its supply duct (mixer air plus trim air), the air in the zone either well mixed or
//! resolved on a voxel grid, the furnishings' heat capacity, heat from people, equipment and sun, heat lost
//! through the skin at its recovery temperature, humidity and CO2, its temperature sensor, and the zone controller
//! that turns the selected temperature into a duct demand and a trim valve command.

use super::cfd::{self, Grid, Shape};
use super::mixer::CO2_OUTSIDE;
use super::thermo::{self, lag, Confirm, Latch, Pi, Sensor, Stream, CP, P_SL};
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
const SPD: Type = Type::Num(Dim::SPEED);
const WHEN: Kind = Kind::Expr(Type::Bool);

pub const SPEC: PartSpec = PartSpec {
    domain: "aircon",
    kind: "zone",
    help: "A cabin or cockpit zone with its duct, sensor and zone controller; give it a grid cell size to resolve the airflow inside it.",
    fields: &[
        req("mixer", Kind::Ref(&["aircon.mixer"]), "mixer unit that supplies its duct"),
        req("fuselage", Kind::Ref(&["pressurization.fuselage"]), "pressurised fuselage it is in"),
        req("air", Kind::Ref(&["environment.air"]), "outside air, for skin heat loss"),
        req("share", Kind::Number, "its share of the mixer flow, by duct sizing"),
        req("volume", Kind::Quantity(Dim::VOLUME), "air volume"),
        opt("trim_valve", Kind::Ref(&["aircon.trim_valve"]), "", "its trim air valve"),
        opt("selected", Kind::Expr(T), "297 K", "temperature selected for the zone"),
        opt("occupants", Kind::Expr(N), "0", "people in the zone"),
        opt("heat", Kind::Expr(W), "0 W", "galleys, lighting, in-flight entertainment and other equipment heat"),
        opt("solar", Kind::Expr(W), "0 W", "sunlight through the windows"),
        opt("skin_ua", Kind::Number, "0", "heat lost through the skin and insulation, W/K; 0 sizes it from the volume"),
        opt("furnishing_mass", Kind::Quantity(Dim::MASS), "0 kg", "seats, panels, floor and galleys; 0 sizes it from the volume"),
        opt("duct_time", Kind::Quantity(Dim::TIME), "25 s", "supply duct thermal lag"),
        opt("duct_min", Kind::Quantity(Dim::TEMPERATURE), "275 K", "coldest duct temperature the controller asks for"),
        opt("duct_max", Kind::Quantity(Dim::TEMPERATURE), "343 K", "hottest duct temperature the controller asks for"),
        opt("duct_overheat", Kind::Quantity(Dim::TEMPERATURE), "361 K", "duct overheat: the trim valve shuts and latches until reset"),
        opt("reset_when", WHEN, "false", "duct overheat reset"),
        opt("sensor_time", Kind::Quantity(Dim::TIME), "30 s", "zone temperature sensor lag, behind its inspirator fan"),
        opt("sensor_failed_when", WHEN, "false", "sensor failed: the controller holds the duct at a fixed 297 K"),
        opt("controller_failed_when", WHEN, "false", "zone controller lost: its trim valve stays where it is"),
        opt("temperature", Kind::Quantity(Dim::TEMPERATURE), "0 K", "air temperature at start; 0 K means the outside air"),
        opt("smoke_when", WHEN, "false", "something smoulders in the zone"),
        opt("smoke_rate", Kind::Quantity(Dim::MASS_FLOW), "0.05 g/s", "smoke it gives off"),
        opt("smoke_at", Kind::Number, "0.5", "where along the zone, 0 front to 1 back"),
        opt("detector_threshold", Kind::Number, "4", "smoke detector alarm, mg/m³ (about 3 %/m obscuration)"),
        opt("metabolic", Kind::Number, "1.0", "occupant activity for comfort, met: 1.0 seated, 2.0 cabin crew walking"),
        opt("clothing", Kind::Number, "0.7", "occupant clothing for comfort, clo"),
        opt("transfer_to", Kind::Ref(&["aircon.zone"]), "", "the next zone along the cabin that part of its air drifts into, toward the outflow valve"),
        opt("transfer", Kind::Number, "0", "share of its air that leaves through that zone rather than its own grilles"),
        opt("skin_area", Kind::Number, "0", "inner skin area behind the lining, m²; 0 sizes it from the volume"),
        opt("insulation_capacity", Kind::Number, "0.25", "water the insulation blankets hold before they drip, kg/m²"),
        opt("vapour_conductance", Kind::Number, "0.0012", "how readily cabin moisture reaches the cold skin through the lining, m/s"),
        opt("drying_when", WHEN, "false", "a zonal drying system blows dry air along the skin"),
        opt("drying_humidity", Kind::Number, "0.0008", "humidity ratio of that dry air, kg/kg"),
        opt("grid_cell", Kind::Quantity(Dim::LENGTH), "0 m", "resolve the airflow on a grid of this cell size; 0 m for well mixed"),
        opt("length", Kind::Quantity(Dim::LENGTH), "10 m", "grid: zone length"),
        opt("section", Kind::Text, "circle", "grid: cabin cross-section: circle, ellipse, rectangle or polygon"),
        opt("radius", Kind::Quantity(Dim::LENGTH), "3.2 m", "grid: inner radius of a circular section"),
        opt("width", Kind::Quantity(Dim::LENGTH), "6 m", "grid: inner width of an ellipse or rectangle"),
        opt("height", Kind::Quantity(Dim::LENGTH), "6.5 m", "grid: inner height of an ellipse or rectangle"),
        opt("section_points", Kind::Text, "", "grid: polygon corners from the fuselage centre in metres, \"y,z y,z ...\""),
        opt("bins", Kind::Bool, "true", "grid: overhead bins along both sides"),
        opt("floor", Kind::Quantity(Dim::LENGTH), "-1.3 m", "grid: floor height from the fuselage centre"),
        opt("ceiling", Kind::Quantity(Dim::LENGTH), "1.4 m", "grid: ceiling height from the fuselage centre"),
        opt("seats_abreast", Kind::Number, "8", "grid: seats across"),
        opt("grid_rate", Kind::Quantity(Dim::FREQUENCY), "2 Hz", "grid: solver updates per second"),
    ],
    outputs: &[
        ("temperature", T),
        ("sensed", T),
        ("selected", T),
        ("duct_temperature", T),
        ("duct_demand", T),
        ("trim_command", N),
        ("supply_flow", MF),
        ("humidity", N),
        ("co2", N),
        ("heat_load", W),
        ("skin_loss", W),
        ("furnishing_temperature", T),
        ("head_temperature", T),
        ("ankle_temperature", T),
        ("ceiling_temperature", T),
        ("stratification", T),
        ("min_temperature", T),
        ("max_temperature", T),
        ("draught", SPD),
        ("air_speed", SPD),
        ("seat_co2", N),
        ("exhaust_temperature", T),
        ("air_changes", N),
        ("ozone", N),
        ("ozone_average", N),
        ("ozone_high", B),
        ("co2_high", B),
        ("pmv", N),
        ("ppd", N),
        ("smoke", N),
        ("smoke_detected", B),
        ("skin_temperature", T),
        ("insulation_water", Type::Num(Dim::MASS)),
        ("insulation_ice", Type::Num(Dim::MASS)),
        ("drip", MF),
        ("condensing", B),
        ("duct_overheat", B),
        ("hot", B),
        ("cold", B),
        ("fault", B),
        ("failed", B),
    ],
};

const OUTPUTS: &[&str] = &[
    "temperature",
    "sensed",
    "selected",
    "duct_temperature",
    "duct_demand",
    "trim_command",
    "supply_flow",
    "humidity",
    "co2",
    "heat_load",
    "skin_loss",
    "furnishing_temperature",
    "head_temperature",
    "ankle_temperature",
    "ceiling_temperature",
    "stratification",
    "min_temperature",
    "max_temperature",
    "draught",
    "air_speed",
    "seat_co2",
    "exhaust_temperature",
    "air_changes",
    "ozone",
    "ozone_average",
    "ozone_high",
    "co2_high",
    "pmv",
    "ppd",
    "smoke",
    "smoke_detected",
    "skin_temperature",
    "insulation_water",
    "insulation_ice",
    "drip",
    "condensing",
    "duct_overheat",
    "hot",
    "cold",
    "fault",
    "failed",
];

const PAX_SENSIBLE: f64 = 75.0;
const PAX_LATENT: f64 = 45.0;
const PAX_CO2: f64 = 5.2e-6;
/// Skin and insulation conductance per cubic metre of zone when none is given, W/K.
const SKIN_PER_M3: f64 = 2.2;
/// Furnishing mass per cubic metre when none is given, kg, and its specific heat.
const FURNISHING_PER_M3: f64 = 9.0;
const FURNISHING_C: f64 = 1200.0;
/// Air-to-furnishing conductance per kg of furnishings, W/K.
const FURNISHING_UA_PER_KG: f64 = 0.6;
/// Skin recovery factor on the fuselage boundary layer.
const SKIN_RECOVERY: f64 = 0.89;
/// Carbon dioxide limit, ppm.
const CO2_LIMIT: f64 = 5000.0;

pub struct Zone {
    pub mixer: usize,
    pub fuselage: usize,
    pub trim: Option<usize>,
    air: [usize; 3],
    pub share: f64,
    volume: f64,
    selected: Option<Expr>,
    occupants: Option<Expr>,
    heat: Option<Expr>,
    solar: Option<Expr>,
    reset: Option<Expr>,
    sensor_failed: Option<Expr>,
    ctl_failed: Option<Expr>,
    skin_ua: f64,
    furn_c: f64,
    furn_ua: f64,
    duct_tau: f64,
    duct_limit: f64,
    sensor_tau: f64,
    pub t: f64,
    pub w: f64,
    pub co2: f64,
    furn_t: f64,
    duct_t: f64,
    duct_w: f64,
    sensor: Sensor,
    zone_ctl: Pi,
    trim_ctl: Pi,
    overheat: Latch,
    overheat_t: Confirm,
    pub duct_demand: f64,
    pub trim_command: f64,
    grid: Option<Grid>,
    grid_period: f64,
    /// Waiting for the first step to take the outside air temperature.
    from_outside: bool,
    grid_clock: f64,
    /// Round-robin slot so zones with grids take turns.
    pub grid_turn: usize,
    last: cfd::Outputs,
    /// Air leaving the zone this step, at the exhaust's state.
    pub exhaust: Stream,
    pub exhaust_co2: f64,
    pub ozone: super::quality::Ozone,
    smoke_when: Option<Expr>,
    smoke_rate: f64,
    smoke_at: f64,
    detector: f64,
    met: f64,
    clo: f64,
    /// Lumped smoke, mg/m³.
    smoke: f64,
    /// Air extract fans take from the zone this step.
    pub extract: f64,
    /// Water a duct humidifier evaporates into its supply this step, kg/s.
    pub humidify: f64,
    pub condensing: bool,
    pub transfer_to: Option<usize>,
    transfer: f64,
    /// Air arriving from the zone before it along the cabin, with its CO2, ozone and smoke.
    pub transfer_in: Stream,
    pub transfer_in_co2: f64,
    pub transfer_in_smoke: f64,
    skin_area: f64,
    capacity: f64,
    vapour_g: f64,
    drying: Option<Expr>,
    drying_w: f64,
    water: f64,
    ice: f64,
    s: Slots,
}

impl Zone {
    pub fn new(c: &Checked, p: &CheckedPart, mixer: usize, fuselage: usize, trim: Option<usize>, transfer_to: Option<usize>) -> Option<Self> {
        let air = [slot_of(c, p, "air", "temperature")?, slot_of(c, p, "air", "airspeed")?, slot_of(c, p, "air", "pressure")?];
        let volume = p.num("volume").max(0.1);
        let skin = p.num("skin_ua");
        let furn_m = p.num("furnishing_mass");
        let furn_m = if furn_m > 0.0 { furn_m } else { FURNISHING_PER_M3 * volume };
        let t0 = p.num("temperature");
        let t0 = if t0 > 100.0 { t0 } else { 288.15 };
        let cell = p.num("grid_cell");
        let grid = (cell > 0.0).then(|| {
            let shape = Shape {
                section: cfd::Section::parse(p.text("section"), p.num("radius"), p.num("width"), p.num("height"), p.text("section_points")),
                length: p.num("length").max(0.5),
                floor: p.num("floor"),
                ceiling: p.num("ceiling"),
                cell,
                seats_abreast: p.num("seats_abreast").max(1.0),
                bins: p.flag("bins"),
            };
            Grid::new(shape, t0, 0.006, CO2_OUTSIDE)
        });
        Some(Zone {
            mixer,
            fuselage,
            trim,
            air,
            share: p.num("share").max(0.0),
            volume,
            selected: crate::expr_of(p, "selected"),
            occupants: crate::expr_of(p, "occupants"),
            heat: crate::expr_of(p, "heat"),
            solar: crate::expr_of(p, "solar"),
            reset: crate::expr_of(p, "reset_when"),
            sensor_failed: crate::expr_of(p, "sensor_failed_when"),
            ctl_failed: crate::expr_of(p, "controller_failed_when"),
            skin_ua: if skin > 0.0 { skin } else { SKIN_PER_M3 * volume },
            furn_c: furn_m * FURNISHING_C,
            furn_ua: furn_m * FURNISHING_UA_PER_KG,
            duct_tau: p.num("duct_time").max(0.0),
            duct_limit: p.num("duct_overheat"),
            sensor_tau: p.num("sensor_time").max(0.0),
            t: t0,
            w: 0.006,
            co2: CO2_OUTSIDE,
            furn_t: t0,
            duct_t: t0,
            duct_w: 0.006,
            sensor: Sensor::new(t0, p.num("sensor_time").max(0.0)),
            zone_ctl: Pi::new(1.2, 0.015, p.num("duct_min"), p.num("duct_max")).starting_at(293.0),
            trim_ctl: Pi::new(0.02, 0.01, 0.0, 1.0),
            overheat: Latch::default(),
            overheat_t: Confirm::default(),
            duct_demand: 293.0,
            trim_command: 0.0,
            grid_period: 1.0 / p.num("grid_rate").max(0.05),
            from_outside: p.num("temperature") <= 100.0,
            grid,
            grid_clock: 0.0,
            grid_turn: 0,
            last: cfd::Outputs { mean_t: t0, head_t: t0, ankle_t: t0, ceiling_t: t0, sensor_t: t0, exhaust_t: t0, exhaust_q: 0.006, exhaust_co2: CO2_OUTSIDE, min_t: t0, max_t: t0, mean_q: 0.006, mean_co2: CO2_OUTSIDE, seat_co2_max: CO2_OUTSIDE, ..Default::default() },
            exhaust: Stream::none(t0, P_SL),
            exhaust_co2: CO2_OUTSIDE,
            ozone: super::quality::Ozone::new(),
            smoke_when: crate::expr_of(p, "smoke_when"),
            smoke_rate: p.num("smoke_rate"),
            smoke_at: p.num("smoke_at"),
            detector: p.num("detector_threshold").max(0.01),
            met: p.num("metabolic").clamp(0.7, 4.0),
            clo: p.num("clothing").clamp(0.0, 2.0),
            smoke: 0.0,
            extract: 0.0,
            humidify: 0.0,
            condensing: false,
            transfer_to,
            transfer: p.num("transfer").clamp(0.0, 0.9),
            transfer_in: Stream::none(t0, P_SL),
            transfer_in_co2: CO2_OUTSIDE,
            transfer_in_smoke: 0.0,
            skin_area: { let a = p.num("skin_area"); if a > 0.0 { a } else { 2.2 * volume.powf(2.0 / 3.0) * (volume / 50.0).max(1.0).powf(1.0 / 3.0) } },
            capacity: p.num("insulation_capacity").max(1e-4),
            vapour_g: p.num("vapour_conductance").max(0.0),
            drying: crate::expr_of(p, "drying_when"),
            drying_w: p.num("drying_humidity").max(0.0),
            water: 0.0,
            ice: 0.0,
            s: Slots::of(p, OUTPUTS),
        })
    }

    /// The air this zone passes on along the cabin, and the share of its exhaust that goes back to its own grilles.
    pub fn transfer_out(&self) -> (Stream, f64, f64) {
        (self.exhaust.with_flow(self.exhaust.flow * self.transfer), self.exhaust_co2, self.smoke)
    }

    pub fn returned(&self) -> Stream {
        self.exhaust.with_flow(self.exhaust.flow * (1.0 - self.transfer))
    }

    pub fn has_grid(&self) -> bool {
        self.grid.is_some()
    }

    pub fn cells(&self) -> usize {
        self.grid.as_ref().map_or(0, |g| g.cells())
    }

    pub fn section(&self) -> Option<Vec<Vec<Option<f64>>>> {
        self.grid.as_ref().map(|g| g.section())
    }

    /// Step the zone. `supply` is its share of mixer air, `trim` the hot air its valve added, `cabin_p` the
    /// fuselage pressure; `grid_now` says whether this zone's grid takes its turn this step.
    pub fn step(&mut self, slots: &mut [f64], dt: f64, supply: Stream, supply_co2: f64, supply_ozone: f64, trim: Stream, cabin_p: f64, grid_now: bool) {
        let failed = self.s.on(slots, "failed");
        let (ta, tas) = (slots[self.air[0]].max(150.0), slots[self.air[1]].max(0.0));
        let skin_t = thermo::recovery_temperature(ta, tas, SKIN_RECOVERY);
        if self.from_outside {
            self.from_outside = false;
            self.start_at(ta);
        }
        let selected = eval(&mut self.selected, slots, dt, 297.0).clamp(280.0, 310.0);
        let occupants = eval(&mut self.occupants, slots, dt, 0.0).max(0.0);
        let equip = eval(&mut self.heat, slots, dt, 0.0);
        let solar = eval(&mut self.solar, slots, dt, 0.0).max(0.0);

        // Supply duct: trim air mixed in, then the duct walls' lag.
        let arriving = self.transfer_in;
        let duct = super::supply::Humidifier::apply(self.humidify, thermo::mix(&[supply, trim, arriving]));
        let duct_co2 = if duct.flow > 1e-9 { (supply.flow * supply_co2 + trim.flow * CO2_OUTSIDE + arriving.flow * self.transfer_in_co2) / duct.flow } else { supply_co2 };
        // Wet insulation conducts heat far better than dry.
        let wet = ((self.water + self.ice) / (self.capacity * self.skin_area)).min(1.0);
        let skin_ua = self.skin_ua * (1.0 + 1.5 * wet);
        if duct.flow > 1e-6 {
            self.duct_t = lag(self.duct_t, duct.t, self.duct_tau, dt);
            self.duct_w = duct.w;
        } else {
            self.duct_t = lag(self.duct_t, self.t, 4.0 * self.duct_tau.max(1.0), dt);
        }

        let rho = cabin_p / (thermo::R * self.t.max(150.0));
        let air_mass = (rho * self.volume).max(0.01);
        let heat = occupants * PAX_SENSIBLE + equip + solar;
        let smoke_now = if yes(&mut self.smoke_when, slots, dt, false) { self.smoke_rate } else { 0.0 };
        self.ozone.step(supply_ozone, duct.flow, air_mass, occupants, cabin_p, dt);
        let (skin_loss, furn_gain);
        if let Some(g) = &mut self.grid {
            self.grid_clock += dt;
            if grid_now && self.grid_clock >= self.grid_period {
                let step_dt = self.grid_clock.min(2.0);
                self.grid_clock = 0.0;
                let inp = cfd::Inputs {
                    supply_flow: duct.flow,
                    supply_t: self.duct_t,
                    supply_q: self.duct_w,
                    supply_co2: duct_co2,
                    smoke_rate: smoke_now,
                    smoke_at: (self.smoke_at, 0.5, 0.2),
                    extract: self.extract,
                    occupants,
                    heat: equip + solar,
                    skin_t,
                    skin_ua,
                    density: rho,
                    furnishing_t: self.furn_t,
                    furnishing_ua: self.furn_ua,
                };
                self.last = g.step(&inp, step_dt);
            }
            self.t = self.last.mean_t;
            self.w = self.last.mean_q;
            self.co2 = self.last.mean_co2;
            skin_loss = self.last.wall_loss;
            furn_gain = self.last.furnishing_gain;
            self.exhaust = Stream::new(duct.flow, self.last.exhaust_t, cabin_p, self.last.exhaust_q);
            self.exhaust_co2 = self.last.exhaust_co2;
            self.smoke = self.last.smoke_mean;
        } else {
            // Well mixed: implicit in the supply and skin so large flows stay stable.
            skin_loss = skin_ua * (self.t - skin_t);
            furn_gain = self.furn_ua * (self.furn_t - self.t);
            let c_air = air_mass * CP;
            let g_supply = duct.flow * CP;
            self.t = (c_air * self.t + dt * (g_supply * self.duct_t + heat + self.furn_ua * self.furn_t + skin_ua * skin_t)) / (c_air + dt * (g_supply + self.furn_ua + skin_ua));
            let water = occupants * PAX_LATENT / thermo::LATENT;
            self.w = (air_mass * self.w + dt * (duct.flow * self.duct_w + water)) / (air_mass + dt * duct.flow);
            let co2_gen = occupants * PAX_CO2 * 1e6 / 1.52;
            self.co2 = (air_mass * self.co2 + dt * (duct.flow * duct_co2 + co2_gen)) / (air_mass + dt * duct.flow);
            self.exhaust = Stream::new(duct.flow, self.t, cabin_p, self.w);
            self.exhaust_co2 = self.co2;
            // Well mixed: what is made, against what the exhaust, extract fans and settling take away.
            let removal = (duct.flow + self.extract) / air_mass + 1.0 / 1800.0;
            let brought = arriving.flow * self.transfer_in_smoke / air_mass;
            self.smoke = (self.smoke + dt * (smoke_now * 1e6 / self.volume + brought)) / (1.0 + dt * removal);
        }
        // Furnishings exchange with the air around them.
        self.furn_t += -furn_gain * dt / self.furn_c;

        // Sensor and zone controller.
        let sensor_failed = yes(&mut self.sensor_failed, slots, dt, false);
        let ctl_failed = failed || yes(&mut self.ctl_failed, slots, dt, false);
        let truth = if self.grid.is_some() { self.last.sensor_t } else { self.t };
        self.sensor.tau = self.sensor_tau;
        let sensed = self.sensor.read(truth, sensor_failed, dt);
        if !ctl_failed {
            self.duct_demand = if sensor_failed { 297.0 } else { self.zone_ctl.step(selected - sensed, dt) };
        }
        let over_now = self.overheat_t.hold(self.duct_t > self.duct_limit, 1.0, dt);
        let over = self.overheat.update(over_now, yes(&mut self.reset, slots, dt, false));
        if !ctl_failed {
            let want = if over { 0.0 } else { self.trim_ctl.step((self.duct_demand - self.duct_t) / 10.0, dt) };
            if over {
                self.trim_ctl.reset(0.0);
            }
            self.trim_command = want;
        }

        let air_changes = if self.volume > 0.0 { duct.flow / rho.max(0.1) * 3600.0 / self.volume } else { 0.0 };
        let (head, ankle, ceiling, min_t, max_t, draught, speed, seat_co2) = if self.grid.is_some() {
            let l = &self.last;
            (l.head_t, l.ankle_t, l.ceiling_t, l.min_t, l.max_t, l.draught, l.mean_speed, l.seat_co2_max)
        } else {
            (self.t, self.t, self.t, self.t, self.t, 0.0, duct.flow / rho.max(0.1) / self.volume.powf(2.0 / 3.0), self.co2)
        };
        // Moisture behind the lining: it condenses or freezes on the cold structure and drips when it thaws.
        let structure = skin_t + 0.15 * (self.t - skin_t);
        let w_air = if yes(&mut self.drying, slots, dt, false) { self.drying_w.min(self.w) } else { self.w };
        let rho_air = cabin_p / (thermo::R * self.t.max(150.0));
        let flux = rho_air * self.vapour_g * self.skin_area * (w_air - thermo::w_sat(structure, cabin_p));
        let (mut condensing, mut drip) = (false, 0.0);
        if flux > 0.0 {
            condensing = true;
            if structure < thermo::T_ZERO { self.ice += flux * dt } else { self.water += flux * dt }
        } else {
            let dry = -flux * dt;
            let from_water = dry.min(self.water);
            self.water -= from_water;
            self.ice = (self.ice - 0.3 * (dry - from_water)).max(0.0);
        }
        if structure > thermo::T_ZERO && self.ice > 0.0 {
            let melt = (8.0 * self.skin_area * (structure - thermo::T_ZERO) / 3.34e5 * dt).min(self.ice);
            self.ice -= melt;
            self.water += melt;
        }
        let hold = self.capacity * self.skin_area;
        if self.water > hold {
            drip = (self.water - hold) / 30.0;
            self.water -= drip * dt;
        }
        let detector = if self.grid.is_some() { self.last.smoke_detector } else { self.smoke };
        let wall_t = self.t - 0.3 * (self.t - skin_t);
        let radiant = 0.6 * self.furn_t + 0.4 * wall_t;
        let rh = thermo::relative_humidity(self.w, self.t, cabin_p);
        let comfort_v = if self.grid.is_some() { self.last.draught.max(0.05) } else { 0.1 };
        let (vote, ppd) = super::quality::pmv(head, radiant, comfort_v, rh, self.met, self.clo);
        let s = &self.s;
        s.put(slots, "temperature", self.t);
        s.put(slots, "sensed", sensed);
        s.put(slots, "selected", selected);
        s.put(slots, "duct_temperature", self.duct_t);
        s.put(slots, "duct_demand", self.duct_demand);
        s.put(slots, "trim_command", self.trim_command);
        s.put(slots, "supply_flow", duct.flow);
        s.put(slots, "humidity", thermo::relative_humidity(self.w, self.t, cabin_p));
        s.put(slots, "co2", self.co2);
        s.put(slots, "heat_load", heat);
        s.put(slots, "skin_loss", skin_loss);
        s.put(slots, "furnishing_temperature", self.furn_t);
        s.put(slots, "head_temperature", head);
        s.put(slots, "ankle_temperature", ankle);
        s.put(slots, "ceiling_temperature", ceiling);
        s.put(slots, "stratification", head - ankle);
        s.put(slots, "min_temperature", min_t);
        s.put(slots, "max_temperature", max_t);
        s.put(slots, "draught", draught);
        s.put(slots, "air_speed", speed);
        s.put(slots, "seat_co2", seat_co2);
        s.put(slots, "exhaust_temperature", self.exhaust.t);
        s.put(slots, "air_changes", air_changes);
        s.put(slots, "ozone", self.ozone.sea_level(cabin_p));
        s.put(slots, "ozone_average", self.ozone.twa);
        s.flag(slots, "ozone_high", self.ozone.sea_level(cabin_p) > super::quality::OZONE_PEAK || self.ozone.twa > super::quality::OZONE_TWA);
        // FAR 25.831 holds carbon dioxide to half a percent by volume, sea-level equivalent.
        s.flag(slots, "co2_high", self.co2.max(seat_co2) * cabin_p / P_SL > CO2_LIMIT);
        s.put(slots, "pmv", vote);
        s.put(slots, "ppd", ppd);
        s.put(slots, "smoke", detector);
        s.flag(slots, "smoke_detected", detector > self.detector);
        s.put(slots, "skin_temperature", structure);
        s.put(slots, "insulation_water", self.water);
        s.put(slots, "insulation_ice", self.ice);
        s.put(slots, "drip", drip);
        s.flag(slots, "condensing", condensing);
        self.condensing = condensing;
        s.flag(slots, "duct_overheat", over);
        s.flag(slots, "hot", self.t > selected + 3.0);
        s.flag(slots, "cold", self.t < selected - 3.0);
        s.flag(slots, "fault", over || sensor_failed || ctl_failed);
    }

    pub fn scalar_state(&self, v: &mut Vec<f64>) {
        v.extend([self.t, self.w, self.co2, self.furn_t, self.duct_t, self.duct_w, self.sensor.value]);
        v.extend([self.zone_ctl.integral, self.trim_ctl.integral, self.overheat.on as u8 as f64, self.overheat_t.elapsed, self.duct_demand, self.trim_command, self.grid_clock]);
        v.extend([self.exhaust.flow, self.exhaust.t, self.exhaust.p, self.exhaust.w, self.exhaust_co2, self.from_outside as u8 as f64]);
        v.extend([self.ozone.ppb, self.ozone.twa, self.smoke, self.extract, self.water, self.ice]);
        v.extend([self.transfer_in.flow, self.transfer_in.t, self.transfer_in.p, self.transfer_in.w, self.transfer_in_co2, self.transfer_in_smoke]);
    }

    pub fn restore_scalar(&mut self, it: &mut impl Iterator<Item = f64>) {
        let mut n = || it.next().unwrap_or(0.0);
        self.t = n();
        self.w = n();
        self.co2 = n();
        self.furn_t = n();
        self.duct_t = n();
        self.duct_w = n();
        self.sensor.value = n();
        self.zone_ctl.integral = n();
        self.trim_ctl.integral = n();
        self.overheat.on = n() != 0.0;
        self.overheat_t.elapsed = n();
        self.duct_demand = n();
        self.trim_command = n();
        self.grid_clock = n();
        self.exhaust = Stream { flow: n(), t: n(), p: n(), w: n() };
        self.exhaust_co2 = n();
        self.from_outside = n() != 0.0;
        self.ozone.ppb = n();
        self.ozone.twa = n();
        self.smoke = n();
        self.extract = n();
        self.water = n();
        self.ice = n();
        self.transfer_in = Stream { flow: n(), t: n(), p: n(), w: n() };
        self.transfer_in_co2 = n();
        self.transfer_in_smoke = n();
    }

    pub fn grid_state(&self, v: &mut Vec<f64>) {
        if let Some(g) = &self.grid {
            g.state(v);
        }
    }

    pub fn grid_restore(&mut self, it: &mut impl Iterator<Item = f64>) {
        if let Some(g) = &mut self.grid {
            g.restore(it);
        }
    }

    fn start_at(&mut self, t: f64) {
        self.t = t;
        self.furn_t = t;
        self.duct_t = t;
        self.sensor.value = t;
        for x in [&mut self.last.mean_t, &mut self.last.head_t, &mut self.last.ankle_t, &mut self.last.ceiling_t, &mut self.last.sensor_t, &mut self.last.exhaust_t, &mut self.last.min_t, &mut self.last.max_t] {
            *x = t;
        }
        self.exhaust.t = t;
        if let Some(g) = &mut self.grid {
            g.settle(t, self.w, self.co2);
        }
    }

    /// After a numerical upset: still air at the zone's last good temperature.
    pub fn settle(&mut self) {
        let t = if self.t.is_finite() { self.t } else { 295.0 };
        if let Some(g) = &mut self.grid {
            g.settle(t, self.w, self.co2);
        }
    }
}
