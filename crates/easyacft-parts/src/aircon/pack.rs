//! An air-conditioning pack: flow control valve, ozone converter, primary and secondary heat exchangers with metal
//! heat capacity, a bootstrap air cycle machine whose speed comes from its shaft power balance, the high-pressure
//! water separation loop (reheater, condenser, water extractor), bypass and anti-ice valves, ram air doors with
//! foreign-object closure, a ram fan on the ground, and the pack controller's protections.

use super::thermo::{self, compress, condense, expand, humidity_ratio, lag, Actuator, Confirm, Pi, Stream, CP, P_SL, T_ZERO};
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
const HZ: Type = Type::Num(Dim::FREQUENCY);
const WHEN: Kind = Kind::Expr(Type::Bool);

pub const SPEC: PartSpec = PartSpec {
    domain: "aircon",
    kind: "pack",
    help: "An air cycle machine pack fed from a bleed duct, delivering cooled, dried air to a mixer.",
    fields: &[
        req("duct", Kind::Ref(&["pneumatic.duct", "aircon.cabin_air_compressor"]), "bleed duct it draws from, or a cabin air compressor"),
        req("air", Kind::Ref(&["environment.air"]), "outside air and flight state"),
        req("flow", Kind::Quantity(Dim::MASS_FLOW), "normal pack flow, for example \"1.1 kg/s\""),
        opt("on_when", WHEN, "true", "pack valve commanded open: its push-button and the controller"),
        opt("powered_when", WHEN, "true", "the pack controller has power"),
        opt("flow_selector", Kind::Expr(N), "1", "flow as a share of normal: 0.8 low, 1.0 normal, 1.2 high"),
        opt("flow_demand", Kind::Expr(N), "1", "extra flow factor from the system controller, such as single-pack or APU bleed"),
        opt("outlet_demand", Kind::Expr(T), "285 K", "pack outlet temperature the controller asks for"),
        opt("ambient_humidity", Kind::Expr(N), "0.5", "outside relative humidity, 0 to 1"),
        opt("minimum_pressure", Kind::Quantity(Dim::PRESSURE), "12 psi", "bleed pressure above ambient the flow control valve needs to open; it springs shut below"),
        opt("fcv_time", Kind::Quantity(Dim::TIME), "4 s", "flow control valve travel time"),
        opt("primary_ua", Kind::Number, "5500", "primary heat exchanger conductance at design ram flow, W/K"),
        opt("secondary_ua", Kind::Number, "7000", "secondary heat exchanger conductance, W/K"),
        opt("reheater_ua", Kind::Number, "1200", "reheater conductance, W/K"),
        opt("condenser_ua", Kind::Number, "1800", "condenser conductance, W/K"),
        opt("exchanger_mass", Kind::Quantity(Dim::MASS), "45 kg", "aluminium in the heat exchangers, for their warm-up and cool-down"),
        opt("compressor_ratio", Kind::Number, "2.4", "ACM compressor pressure ratio at rated speed"),
        opt("compressor_efficiency", Kind::Number, "0.78", "ACM compressor isentropic efficiency"),
        opt("turbine_efficiency", Kind::Number, "0.84", "ACM turbine isentropic efficiency"),
        opt("rated_speed", Kind::Quantity(Dim::FREQUENCY), "40000 rpm", "ACM rated speed"),
        opt("inertia", Kind::Number, "0.0025", "ACM rotor inertia, kg m²"),
        opt("fan_power", Kind::Quantity(Dim::POWER), "7 kW", "ram fan power at rated speed"),
        opt("overspeed", Kind::Number, "1.15", "ACM overspeed trip, share of rated speed"),
        opt("ram_area", Kind::Number, "0.12", "ram air inlet area fully open, m²"),
        opt("ram_fan_flow", Kind::Quantity(Dim::MASS_FLOW), "4 kg/s", "ram air the fan draws on the ground at rated speed"),
        opt("ram_door_time", Kind::Quantity(Dim::TIME), "12 s", "ram inlet and outlet door travel time"),
        opt("foreign_object_closure", Kind::Bool, "true", "ram doors close on the runway above 30 m/s to keep debris out"),
        opt("bypass_time", Kind::Quantity(Dim::TIME), "6 s", "bypass (temperature control) valve travel time"),
        opt("anti_ice_temperature", Kind::Quantity(Dim::TEMPERATURE), "275 K", "anti-ice valve holds the condenser cold passages above this so turbine fog cannot freeze on them"),
        opt("water_extractor_efficiency", Kind::Number, "0.92", "share of condensed water the extractor removes"),
        opt("ozone_converter", Kind::Bool, "true", "fitted; efficiency falls off when cold"),
        opt("compressor_limit", Kind::Quantity(Dim::TEMPERATURE), "503 K", "compressor outlet overheat shutdown"),
        opt("outlet_limit", Kind::Quantity(Dim::TEMPERATURE), "368 K", "pack outlet overheat shutdown"),
        opt("fcv_failed_when", WHEN, "false", "flow control valve failed shut"),
        opt("acm_seized_when", WHEN, "false", "air cycle machine seized: the pack runs on its heat exchangers alone"),
        opt("fouling", Kind::Expr(N), "0", "heat exchanger fouling, 0 clean to 1 blocked"),
        opt("ram_door_jammed_when", WHEN, "false", "ram doors jammed where they are"),
        opt("bypass_jammed_when", WHEN, "false", "bypass valve jammed where it is"),
        opt("extractor_clogged_when", WHEN, "false", "water extractor clogged: free water goes downstream"),
        opt("anti_ice_failed_when", WHEN, "false", "anti-ice valve failed shut: turbine fog freezes in the condenser"),
    ],
    outputs: &[
        ("operating", B),
        ("flow", MF),
        ("bleed_demand", MF),
        ("outlet_temperature", T),
        ("outlet_pressure", P),
        ("outlet_humidity", N),
        ("free_water", MF),
        ("water_extracted", MF),
        ("compressor_inlet_temperature", T),
        ("compressor_outlet_temperature", T),
        ("turbine_inlet_temperature", T),
        ("turbine_outlet_temperature", T),
        ("separator_temperature", T),
        ("acm_speed", N),
        ("acm_rpm", HZ),
        ("turbine_power", W),
        ("compressor_power", W),
        ("ram_flow", MF),
        ("ram_door", N),
        ("bypass", N),
        ("fcv", N),
        ("anti_ice", N),
        ("ozone_efficiency", N),
    ("ozone", N),
        ("heat_rejected", W),
        ("exchanger_temperature", T),
        ("compressor_overheat", B),
        ("overheat", B),
        ("condenser_ice", N),
        ("fault", N),
        ("failed", B),
    ],
};

const OUTPUTS: &[&str] = &[
    "operating",
    "flow",
    "bleed_demand",
    "outlet_temperature",
    "outlet_pressure",
    "outlet_humidity",
    "free_water",
    "water_extracted",
    "compressor_inlet_temperature",
    "compressor_outlet_temperature",
    "turbine_inlet_temperature",
    "turbine_outlet_temperature",
    "separator_temperature",
    "acm_speed",
    "acm_rpm",
    "turbine_power",
    "compressor_power",
    "ram_flow",
    "ram_door",
    "bypass",
    "fcv",
    "anti_ice",
    "ozone_efficiency",
    "ozone",
    "heat_rejected",
    "exchanger_temperature",
    "compressor_overheat",
    "overheat",
    "condenser_ice",
    "fault",
    "failed",
];

/// Pressure lost across each heat exchanger pass, share of inlet pressure.
const HX_LOSS: f64 = 0.035;
/// Bearing and windage loss at rated speed, W.
const BEARING_W: f64 = 350.0;
/// Turbine nozzle design point: inlet pressure and temperature at normal flow.
const DESIGN_TURBINE_P: f64 = 3.0 * P_SL;
const DESIGN_TURBINE_T: f64 = 305.0;
const ALUMINIUM_C: f64 = 900.0;
/// Each heat exchanger's share of the metal mass.
const MASS_SHARE: [f64; 4] = [0.35, 0.35, 0.12, 0.18];
/// The pack controller reacts to its outlet sensor through this lag, s.
const SENSOR_TAU: f64 = 2.0;
/// Ram doors close for foreign objects above this ground speed.
const FOD_SPEED: f64 = 30.0;

pub mod fault {
    pub const NONE: f64 = 0.0;
    pub const COMPRESSOR_OVERHEAT: f64 = 1.0;
    pub const OUTLET_OVERHEAT: f64 = 2.0;
    pub const OVERSPEED: f64 = 3.0;
    pub const LOW_PRESSURE: f64 = 4.0;
    pub const ACM_SEIZED: f64 = 5.0;
    pub const FCV: f64 = 6.0;
    pub const NO_POWER: f64 = 7.0;
}

/// A heat exchanger whose metal stores heat between its hot and cold streams.
#[derive(Clone, Copy, Debug)]
struct Exchanger {
    ua: f64,
    capacity: f64,
    metal: f64,
}

impl Exchanger {
    fn new(ua: f64, mass: f64, t: f64) -> Self {
        Exchanger { ua: ua.max(0.0), capacity: (mass * ALUMINIUM_C).max(1.0), metal: t }
    }

    /// Pass both streams for `dt`; returns hot and cold outlet temperatures and heat taken from the hot side.
    fn pass(&mut self, t_hot: f64, m_hot: f64, t_cold: f64, m_cold: f64, ua_scale: f64, dt: f64) -> (f64, f64, f64) {
        let half = self.ua * ua_scale.max(0.0);
        let side = |m: f64| {
            let c = m.max(0.0) * CP;
            if c <= 1e-9 { 0.0 } else { c * (1.0 - (-2.0 * half / c).exp()) }
        };
        let (gh, gc) = (side(m_hot), side(m_cold));
        // Implicit metal temperature so a large flow cannot overshoot it.
        let metal = (self.capacity * self.metal + dt * (gh * t_hot + gc * t_cold)) / (self.capacity + dt * (gh + gc));
        self.metal = metal;
        let q_hot = gh * (t_hot - metal);
        let q_cold = gc * (metal - t_cold);
        let th = if m_hot > 1e-9 { t_hot - q_hot / (m_hot * CP) } else { metal };
        let tc = if m_cold > 1e-9 { t_cold + q_cold / (m_cold * CP) } else { metal };
        (th, tc, q_hot)
    }
}

pub struct Pack {
    duct: [usize; 2],
    air: [usize; 7],
    nominal: f64,
    on: Option<Expr>,
    powered: Option<Expr>,
    selector: Option<Expr>,
    demand_factor: Option<Expr>,
    outlet_demand: Option<Expr>,
    humidity: Option<Expr>,
    fcv_failed: Option<Expr>,
    seized: Option<Expr>,
    fouling: Option<Expr>,
    ram_jammed: Option<Expr>,
    bypass_jammed: Option<Expr>,
    clogged: Option<Expr>,
    ai_failed: Option<Expr>,
    /// Ice in the condenser cold passages, as a share of what blocks them.
    ice: f64,
    minimum_dp: f64,
    pr_rated: f64,
    eta_c: f64,
    eta_t: f64,
    omega_rated: f64,
    inertia: f64,
    fan_power: f64,
    overspeed: f64,
    ram_area: f64,
    ram_fan_flow: f64,
    fod: bool,
    anti_ice_t: f64,
    extractor_eff: f64,
    ozone: bool,
    compressor_limit: f64,
    outlet_limit: f64,
    turbine_cda: f64,
    primary: Exchanger,
    secondary: Exchanger,
    reheater: Exchanger,
    condenser: Exchanger,
    fcv: Actuator,
    ram_door: Actuator,
    bypass: Actuator,
    anti_ice: Actuator,
    temp_ctl: Pi,
    omega: f64,
    outlet_sensor: f64,
    last: Last,
    comp_hot: Confirm,
    outlet_hot: Confirm,
    over: Confirm,
    shut: f64,
    was_on: bool,
    /// Pressure at the pack outlet, set by its mixer each step.
    pub downstream: f64,
    /// What the pack delivered this step.
    pub out: Stream,
    /// Ozone in outside air and leaving the pack, ppb.
    pub ozone_ambient: f64,
    pub ozone_out: f64,
    s: Slots,
}

/// Values from the previous step that close the loops inside the pack.
#[derive(Clone, Copy, Debug)]
struct Last {
    ram_after_secondary: f64,
    separator: f64,
    turbine_in: f64,
    turbine_out: f64,
    compressor_out: f64,
    outlet: f64,
}

impl Pack {
    pub fn new(c: &Checked, p: &CheckedPart) -> Option<Self> {
        let duct = [slot_of(c, p, "duct", "pressure")?, slot_of(c, p, "duct", "temperature")?];
        let air = [
            slot_of(c, p, "air", "pressure")?,
            slot_of(c, p, "air", "temperature")?,
            slot_of(c, p, "air", "airspeed")?,
            slot_of(c, p, "air", "mach")?,
            slot_of(c, p, "air", "on_ground")?,
            slot_of(c, p, "air", "groundspeed")?,
            slot_of(c, p, "air", "latitude")?,
        ];
        let nominal = p.num("flow").max(0.01);
        let mass = p.num("exchanger_mass").max(0.1);
        let k_choke = (thermo::GAMMA / thermo::R).sqrt() * (2.0 / (thermo::GAMMA + 1.0)).powf((thermo::GAMMA + 1.0) / (2.0 * (thermo::GAMMA - 1.0)));
        let t0 = 288.15;
        Some(Pack {
            duct,
            air,
            nominal,
            on: crate::expr_of(p, "on_when"),
            powered: crate::expr_of(p, "powered_when"),
            selector: crate::expr_of(p, "flow_selector"),
            demand_factor: crate::expr_of(p, "flow_demand"),
            outlet_demand: crate::expr_of(p, "outlet_demand"),
            humidity: crate::expr_of(p, "ambient_humidity"),
            fcv_failed: crate::expr_of(p, "fcv_failed_when"),
            seized: crate::expr_of(p, "acm_seized_when"),
            fouling: crate::expr_of(p, "fouling"),
            ram_jammed: crate::expr_of(p, "ram_door_jammed_when"),
            bypass_jammed: crate::expr_of(p, "bypass_jammed_when"),
            clogged: crate::expr_of(p, "extractor_clogged_when"),
            ai_failed: crate::expr_of(p, "anti_ice_failed_when"),
            ice: 0.0,
            minimum_dp: p.num("minimum_pressure"),
            pr_rated: p.num("compressor_ratio").max(1.01),
            eta_c: p.num("compressor_efficiency").clamp(0.3, 0.95),
            eta_t: p.num("turbine_efficiency").clamp(0.3, 0.95),
            omega_rated: p.num("rated_speed").max(100.0),
            inertia: p.num("inertia").max(1e-5),
            fan_power: p.num("fan_power").max(0.0),
            overspeed: p.num("overspeed").max(1.0),
            ram_area: p.num("ram_area").max(0.0),
            ram_fan_flow: p.num("ram_fan_flow").max(0.0),
            fod: p.flag("foreign_object_closure"),
            anti_ice_t: p.num("anti_ice_temperature"),
            extractor_eff: p.num("water_extractor_efficiency").clamp(0.0, 1.0),
            ozone: p.flag("ozone_converter"),
            compressor_limit: p.num("compressor_limit"),
            outlet_limit: p.num("outlet_limit"),
            turbine_cda: nominal * DESIGN_TURBINE_T.sqrt() / (DESIGN_TURBINE_P * k_choke),
            primary: Exchanger::new(p.num("primary_ua"), mass * MASS_SHARE[0], t0),
            secondary: Exchanger::new(p.num("secondary_ua"), mass * MASS_SHARE[1], t0),
            reheater: Exchanger::new(p.num("reheater_ua"), mass * MASS_SHARE[2], t0),
            condenser: Exchanger::new(p.num("condenser_ua"), mass * MASS_SHARE[3], t0),
            fcv: Actuator::new(0.0, p.num("fcv_time")),
            ram_door: Actuator::new(1.0, p.num("ram_door_time")),
            bypass: Actuator::new(0.1, p.num("bypass_time")),
            anti_ice: Actuator::new(0.0, 3.0),
            temp_ctl: Pi::new(0.02, 0.008, -1.0, 1.0),
            omega: 0.0,
            outlet_sensor: t0,
            last: Last { ram_after_secondary: t0, separator: t0, turbine_in: t0, turbine_out: t0, compressor_out: t0, outlet: t0 },
            comp_hot: Confirm::default(),
            outlet_hot: Confirm::default(),
            over: Confirm::default(),
            shut: fault::NONE,
            was_on: false,
            downstream: P_SL,
            out: Stream::none(t0, P_SL),
            ozone_ambient: 40.0,
            ozone_out: 40.0,
            s: Slots::of(p, OUTPUTS),
        })
    }

    /// Turbine inlet pressure that passes `m` through the nozzle into `p_out` at `t_in`.
    fn nozzle_pressure(&self, m: f64, t_in: f64, p_out: f64) -> f64 {
        if m <= 0.0 {
            return p_out;
        }
        let (mut lo, mut hi) = (p_out, p_out * 40.0);
        for _ in 0..40 {
            let mid = 0.5 * (lo + hi);
            if thermo::orifice(self.turbine_cda, mid, t_in, p_out) < m { lo = mid } else { hi = mid }
        }
        0.5 * (lo + hi)
    }

    pub fn step(&mut self, slots: &mut [f64], dt: f64) {
        let s = &self.s;
        let failed = s.on(slots, "failed");
        let (p_bleed, t_bleed) = (slots[self.duct[0]], slots[self.duct[1]].max(150.0));
        let (pa, ta, tas, mach, ground, gs) = (slots[self.air[0]].max(100.0), slots[self.air[1]].max(150.0), slots[self.air[2]], slots[self.air[3]], slots[self.air[4]] != 0.0, slots[self.air[5]]);
        let commanded = yes(&mut self.on, slots, dt, true);
        let powered = yes(&mut self.powered, slots, dt, true);
        let selector = eval(&mut self.selector, slots, dt, 1.0).clamp(0.0, 2.0);
        let factor = eval(&mut self.demand_factor, slots, dt, 1.0).clamp(0.0, 2.0);
        let demand_t = eval(&mut self.outlet_demand, slots, dt, 285.0).clamp(270.0, 360.0);
        let rh = eval(&mut self.humidity, slots, dt, 0.5).clamp(0.0, 1.0);
        let fcv_failed = yes(&mut self.fcv_failed, slots, dt, false) || failed;
        let seized = yes(&mut self.seized, slots, dt, false);
        let fouling = eval(&mut self.fouling, slots, dt, 0.0).clamp(0.0, 1.0);
        self.ram_door.jammed = yes(&mut self.ram_jammed, slots, dt, false);
        self.bypass.jammed = yes(&mut self.bypass_jammed, slots, dt, false);
        let clogged = yes(&mut self.clogged, slots, dt, false);
        let ai_failed = yes(&mut self.ai_failed, slots, dt, false);

        // Protections latch until the pack is selected off.
        if !commanded {
            self.shut = fault::NONE;
        }
        let pressure_ok = p_bleed - pa >= self.minimum_dp;
        let enabled = commanded && powered && !fcv_failed && self.shut == fault::NONE;
        let open_target = if enabled && pressure_ok { 1.0 } else { 0.0 };
        // Pneumatically actuated: it only moves open with pressure behind it, and springs shut without.
        let fcv_speed = if open_target < self.fcv.position { 3.0 } else if pressure_ok { 1.0 } else { 0.0 };
        let fcv = self.fcv.drive(open_target, fcv_speed, dt);

        let w_in = humidity_ratio(rh, ta, pa);
        let omega_share = self.omega / self.omega_rated;
        let pr = if seized { 1.0 } else { 1.0 + (self.pr_rated - 1.0) * omega_share * omega_share };

        // Ram air: dynamic pressure in flight, the ACM fan on the ground; doors shut on the runway at speed.
        let fod_close = self.fod && ground && gs.abs().max(tas.abs()) > FOD_SPEED;
        let p_ram = thermo::ram_pressure(pa, mach.max(0.0), 0.75);
        let t_ram = thermo::recovery_temperature(ta, tas.max(0.0), 1.0);
        let door = self.ram_door.position;
        let ram_dynamic = thermo::orifice(self.ram_area * door, p_ram, t_ram, pa);
        let ram_fan = if seized { 0.25 * self.ram_fan_flow * door.sqrt() } else { self.ram_fan_flow * (0.35 + 0.65 * omega_share).clamp(0.0, 1.2) * door.sqrt() };
        let ram = ram_dynamic.max(ram_fan);

        let wanted = self.nominal * selector * factor;
        let bypass = self.bypass.position;
        let m_acm_frac = if seized { 0.0 } else { 1.0 - bypass };

        // Pressure the ACM path needs for the wanted flow; the bleed supply caps the flow.
        let t_turbine_guess = self.last.turbine_in.max(200.0);
        let need_turbine = self.nozzle_pressure(wanted * m_acm_frac, t_turbine_guess, self.downstream);
        let loss = (1.0 - HX_LOSS).powi(4);
        let need_inlet = if seized { self.downstream / (1.0 - HX_LOSS).powi(2) + 2.0 * PSI_F } else { need_turbine / (pr * loss) };
        let supply = p_bleed.max(0.0);
        let choke = (1.0 - self.ice).clamp(0.05, 1.0);
        let mut flow = if fcv > 1e-3 && need_inlet > 0.0 { wanted * (supply / need_inlet).min(1.0).powf(0.5) * (fcv / 1.0).min(1.0) * choke } else { 0.0 };
        if !pressure_ok && fcv <= 1e-3 {
            flow = 0.0;
        }
        let operating = flow > 0.02 * self.nominal;
        let p_inlet = need_inlet.min(supply);

        // Ozone converter: catalytic, efficient only when the bleed air is hot.
        let ozone_eff = if self.ozone { ((t_bleed - 400.0) / 60.0).clamp(0.2, 0.95) } else { 0.0 };
        // Compressing bleed air destroys some ozone before the converter does the rest.
        self.ozone_ambient = super::quality::ambient_ozone(thermo::isa_altitude(pa), slots[self.air[6]]);
        self.ozone_out = self.ozone_ambient * 0.85 * (1.0 - ozone_eff);

        // Primary heat exchanger: bleed against ram air that has already passed the secondary.
        let ua_scale = (1.0 - fouling) * (ram / (self.ram_fan_flow.max(0.1))).clamp(0.0, 1.5).sqrt().max(0.05);
        let (t1, ram_out, q1) = self.primary.pass(t_bleed, flow, self.last.ram_after_secondary, ram, ua_scale, dt);
        let p1 = p_inlet * (1.0 - HX_LOSS);

        // Anti-ice air leaves ahead of the ACM and rejoins at the turbine outlet.
        let m_ai_in = (AI_SHARE * flow * self.anti_ice.position).min(flow * m_acm_frac);
        let m_acm = flow * m_acm_frac - m_ai_in;
        let (t2, p2) = if m_acm > 1e-6 { (compress(t1, pr, self.eta_c), p1 * pr) } else { (t1, p1) };
        let (t3, ram_mid, q2) = self.secondary.pass(t2, m_acm, t_ram, ram, ua_scale, dt);
        let p3 = p2 * (1.0 - HX_LOSS);

        // High-pressure water separation: reheater hot side, condenser hot side, extractor, reheater cold side.
        let (t4, _, _) = self.reheater.pass(t3, m_acm, self.last.separator, m_acm, 1.0, dt);
        let (t5, _, _) = self.condenser.pass(t4, m_acm, self.last.turbine_out, m_acm, 1.0, dt);
        let p5 = p3 * (1.0 - HX_LOSS);
        let (wet, condensed) = condense(Stream::new(m_acm, t5, p5, w_in));
        let extract = if clogged { 0.0 } else { self.extractor_eff };
        let removed = condensed * extract;
        let carried = (condensed - removed) / m_acm.max(1e-9);
        let separator_t = wet.t;
        let (_, t6, _) = self.reheater.pass(t3, 0.0, separator_t, m_acm, 1.0, 0.0);
        let p6 = p5 * (1.0 - HX_LOSS);
        let turbine_ratio = (p6 / self.downstream.max(1.0)).max(1.0);
        let t_turbine = if m_acm > 1e-6 { expand(t6, turbine_ratio, self.eta_t) } else { t6 };
        // The anti-ice valve injects bleed air at the turbine outlet, ahead of the condenser's cold side.
        let t7 = if m_acm + m_ai_in > 1e-9 { (m_acm * t_turbine + m_ai_in * t_bleed) / (m_acm + m_ai_in) } else { t_turbine };
        // Fog the turbine makes freezes on the condenser's cold passages and chokes them; warm air melts it out.
        let fog = (wet.w + carried - thermo::w_sat(t7, self.downstream)).max(0.0) * m_acm;
        let surface = self.condenser.metal;
        let iced = surface < T_ZERO && fog > 0.0;
        if iced {
            self.ice = (self.ice + fog / (0.002 * self.nominal) * dt / 300.0).min(1.0);
        } else if surface > T_ZERO + 1.0 {
            self.ice = (self.ice - (surface - T_ZERO) * dt / 600.0).max(0.0);
        }
        let m_cold = m_acm + m_ai_in;
        let (_, t8, _) = self.condenser.pass(t4, 0.0, t7, m_cold, 1.0, 0.0);
        let w_cold = if m_cold > 1e-9 { (m_acm * (wet.w + carried) + m_ai_in * w_in) / m_cold } else { w_in };
        let acm_out = Stream::new(m_cold, t8, self.downstream, w_cold);

        // Shaft: turbine drives the compressor and ram fan against bearing loss.
        let p_turbine = m_acm * CP * (t6 - t7);
        let p_comp = m_acm * CP * (t2 - t1);
        let p_fan = self.fan_power * omega_share.max(0.0).powi(3) * door.max(0.2);
        let p_bearing = BEARING_W * omega_share.max(0.0) + 30.0;
        if seized {
            self.omega = 0.0;
        } else {
            // Implicit in speed: compressor and fan absorb more as the shaft speeds up, which makes it stiff.
            let (pr_rated, eta_c, omega_r, fan, j) = (self.pr_rated, self.eta_c, self.omega_rated, self.fan_power, self.inertia);
            let net_at = |w: f64| {
                let x = (w / omega_r).max(0.0);
                let pr_w = 1.0 + (pr_rated - 1.0) * x * x;
                let comp = m_acm * CP * (compress(t1, pr_w, eta_c) - t1);
                p_turbine - comp - fan * x.powi(3) * door.max(0.2) - BEARING_W * x - 30.0
            };
            let w0 = self.omega;
            let dw = 0.01 * omega_r;
            let net = net_at(w0);
            let slope = (net_at(w0 + dw) - net) / dw;
            let inertia_term = j * w0.max(0.05 * omega_r);
            self.omega = (w0 + net * dt / (inertia_term - slope.min(0.0) * dt)).clamp(0.0, 1.5 * omega_r);
            let _ = (p_comp, p_fan, p_bearing);
        }
        if self.over.hold(self.omega > self.overspeed * self.omega_rated, 0.5, dt) && self.shut == fault::NONE {
            self.shut = fault::OVERSPEED;
        }

        // Bypass air leaves the primary heat exchanger and joins downstream of the turbine; anti-ice taps bleed.
        let m_bypass = (flow - m_acm - m_ai_in).max(0.0);
        let bypass_stream = Stream::new(m_bypass, t1, self.downstream, w_in);
        let mut out = thermo::mix(&[acm_out, bypass_stream]);
        if flow <= 1e-6 {
            out = Stream::none(lag(self.last.outlet, ta, 60.0, dt), self.downstream);
        }

        // Pack controller: outlet temperature through bypass and ram doors, separator anti-ice, protections.
        self.outlet_sensor = lag(self.outlet_sensor, out.t, SENSOR_TAU, dt);
        let comp_hot = self.comp_hot.hold(t2 > self.compressor_limit, 1.0, dt);
        let out_hot = self.outlet_hot.hold(self.outlet_sensor > self.outlet_limit, 1.0, dt);
        if comp_hot && self.shut == fault::NONE {
            self.shut = fault::COMPRESSOR_OVERHEAT;
        }
        if out_hot && self.shut == fault::NONE {
            self.shut = fault::OUTLET_OVERHEAT;
        }
        let ctl_on = powered && operating;
        let u = if ctl_on { self.temp_ctl.step(demand_t - self.outlet_sensor, dt) } else { self.temp_ctl.integral };
        let approaching = t2 > self.compressor_limit - 25.0;
        let ram_target = if fod_close {
            0.0
        } else if !ctl_on {
            1.0
        } else if approaching || self.shut != fault::NONE {
            1.0
        } else {
            (0.55 - 0.9 * u).clamp(0.08, 1.0)
        };
        let ram_speed = if powered { 1.0 } else { 0.0 };
        self.ram_door.drive(ram_target, ram_speed, dt);
        let bypass_target = if !ctl_on { 0.3 } else if seized { 1.0 } else { (0.3 + 0.7 * u).clamp(0.0, 0.9) };
        self.bypass.drive(bypass_target, ram_speed, dt);
        // Opens while the turbine's fog could freeze, as far as it takes to hold the outlet just above freezing.
        let risk = fog > 0.0 && surface < self.anti_ice_t;
        let ai_target = if ctl_on && (risk || iced) { (self.anti_ice.position + dt * (self.anti_ice_t + 1.0 - surface) / 10.0).clamp(0.0, 1.0) } else { (self.anti_ice.position - dt / 10.0).max(0.0) };
        self.anti_ice.drive(if ai_failed { 0.0 } else { ai_target }, ram_speed, dt);

        self.last = Last { ram_after_secondary: ram_mid, separator: separator_t, turbine_in: t6, turbine_out: t7, compressor_out: t2, outlet: out.t };
        self.was_on = operating;
        self.out = out;
        let code = if self.shut != fault::NONE {
            self.shut
        } else if seized {
            fault::ACM_SEIZED
        } else if fcv_failed && commanded {
            fault::FCV
        } else if commanded && !powered {
            fault::NO_POWER
        } else if commanded && !pressure_ok {
            fault::LOW_PRESSURE
        } else {
            fault::NONE
        };
        let metal = (self.primary.metal + self.secondary.metal) * 0.5;
        let s = &self.s;
        s.flag(slots, "operating", operating);
        s.put(slots, "flow", flow);
        s.put(slots, "bleed_demand", if enabled { wanted } else { 0.0 });
        s.put(slots, "outlet_temperature", out.t);
        s.put(slots, "outlet_pressure", self.downstream);
        s.put(slots, "outlet_humidity", out.w);
        s.put(slots, "free_water", carried * m_acm);
        s.put(slots, "water_extracted", removed);
        s.put(slots, "compressor_inlet_temperature", t1);
        s.put(slots, "compressor_outlet_temperature", t2);
        s.put(slots, "turbine_inlet_temperature", t6);
        s.put(slots, "turbine_outlet_temperature", t7);
        s.put(slots, "separator_temperature", separator_t);
        s.put(slots, "acm_speed", self.omega / self.omega_rated);
        s.put(slots, "acm_rpm", self.omega);
        s.put(slots, "turbine_power", p_turbine);
        s.put(slots, "compressor_power", p_comp);
        s.put(slots, "ram_flow", ram);
        s.put(slots, "ram_door", self.ram_door.position);
        s.put(slots, "bypass", self.bypass.position);
        s.put(slots, "fcv", fcv);
        s.put(slots, "anti_ice", self.anti_ice.position);
        s.put(slots, "ozone_efficiency", ozone_eff);
        s.put(slots, "ozone", self.ozone_out);
        s.put(slots, "heat_rejected", q1 + q2);
        s.put(slots, "exchanger_temperature", metal);
        s.flag(slots, "compressor_overheat", self.shut == fault::COMPRESSOR_OVERHEAT);
        s.flag(slots, "overheat", self.shut == fault::OUTLET_OVERHEAT || self.shut == fault::COMPRESSOR_OVERHEAT);
        s.put(slots, "condenser_ice", self.ice);
        s.put(slots, "fault", code);
        let _ = (ram_out, p_bleed);
    }

    pub fn state(&self, v: &mut Vec<f64>) {
        v.extend([self.primary.metal, self.secondary.metal, self.reheater.metal, self.condenser.metal]);
        for a in [&self.fcv, &self.ram_door, &self.bypass, &self.anti_ice] {
            v.extend(a.state());
        }
        v.extend([self.temp_ctl.integral, self.omega, self.outlet_sensor]);
        v.extend([self.last.ram_after_secondary, self.last.separator, self.last.turbine_in, self.last.turbine_out, self.last.compressor_out, self.last.outlet]);
        v.extend([self.comp_hot.elapsed, self.outlet_hot.elapsed, self.over.elapsed, self.shut, self.was_on as u8 as f64, self.downstream]);
        v.extend([self.out.flow, self.out.t, self.out.p, self.out.w, self.ice]);
    }

    pub fn restore(&mut self, it: &mut impl Iterator<Item = f64>) {
        let mut n = || it.next().unwrap_or(0.0);
        self.primary.metal = n();
        self.secondary.metal = n();
        self.reheater.metal = n();
        self.condenser.metal = n();
        for a in [&mut self.fcv, &mut self.ram_door, &mut self.bypass, &mut self.anti_ice] {
            a.restore([n(), n()]);
        }
        self.temp_ctl.integral = n();
        self.omega = n();
        self.outlet_sensor = n();
        self.last = Last { ram_after_secondary: n(), separator: n(), turbine_in: n(), turbine_out: n(), compressor_out: n(), outlet: n() };
        self.comp_hot.elapsed = n();
        self.outlet_hot.elapsed = n();
        self.over.elapsed = n();
        self.shut = n();
        self.was_on = n() != 0.0;
        self.downstream = n();
        self.out = Stream { flow: n(), t: n(), p: n(), w: n() };
        self.ice = n();
    }
}

const PSI_F: f64 = thermo::PSI;
/// Most of the pack flow the anti-ice valve can bring round as hot bleed.
const AI_SHARE: f64 = 0.3;
