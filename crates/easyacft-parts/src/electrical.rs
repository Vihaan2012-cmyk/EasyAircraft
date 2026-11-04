//! Electrical domain: buses, batteries, generators with their control units, switching, diodes, protection,
//! loads, converters, chargers and shedding.

use crate::util::Slots;
use crate::{expr_of, Domain, Expr};
use easyacft_core::{Dim, Network};
use easyacft_lang::schema::{opt, req, Kind, PartSpec};
use easyacft_lang::{Checked, CheckedPart, Type};
use std::collections::HashMap;

const B: Type = Type::Bool;
const V: Type = Type::Num(Dim::VOLTAGE);
const A: Type = Type::Num(Dim::CURRENT);
const W: Type = Type::Num(Dim::POWER);
const HZ: Type = Type::Num(Dim::FREQUENCY);
const N: Type = Type::Num(Dim::NONE);
const T: Type = Type::Num(Dim::TEMPERATURE);
const Q: Type = Type::Num(Dim::CHARGE);
const BUS: &[&str] = &["electrical.bus"];
const WHEN: Kind = Kind::Expr(Type::Bool);
const CONVERTER_OUT: &[(&str, Type)] = &[("current", A), ("power", W), ("active", B), ("temperature", T), ("overheat", B), ("limiting", B), ("failed", B)];

pub static SPECS: &[PartSpec] = &[
    PartSpec {
        domain: "electrical",
        kind: "bus",
        help: "A busbar. AC buses take the frequency of whatever feeds them.",
        fields: &[
            req("nominal", Kind::Quantity(Dim::VOLTAGE), "for example \"28 V\""),
            opt("powered_at", Kind::Number, "0.8", "share of nominal counted as powered"),
            opt("short_when", WHEN, "false", "a short to structure: fault current flows until something upstream trips"),
            opt("fault_resistance", Kind::Quantity(Dim::RESISTANCE), "0.01 ohm", "resistance of that short"),
        ],
        outputs: &[("voltage", V), ("frequency", HZ), ("powered", B), ("failed", B)],
    },
    PartSpec {
        domain: "electrical",
        kind: "battery",
        help: "A battery: open-circuit voltage from charge, internal resistance and usable capacity that change with temperature, Peukert loss at high current, charge efficiency, I²R heating and thermal runaway.",
        fields: &[
            req("bus", Kind::Ref(BUS), "the bus it feeds"),
            req("voltage", Kind::Quantity(Dim::VOLTAGE), "nominal voltage, for example \"24 V\""),
            req("capacity", Kind::Quantity(Dim::CHARGE), "for example \"44 Ah\""),
            opt("resistance", Kind::Quantity(Dim::RESISTANCE), "0.02 ohm", "internal resistance at 15 °C"),
            opt("charge", Kind::Number, "1.0", "state of charge at start, 0 to 1"),
            opt("charge_limit", Kind::Quantity(Dim::CURRENT), "40 A", "most current it takes when charging"),
            opt("chemistry", Kind::Text, "nicd", "nicd, lead_acid or lithium: sets the voltage curve, Peukert exponent, charge efficiency, cold loss and runaway temperature"),
            opt("ambient", Kind::Expr(T), "288.15 K", "temperature of the bay it sits in"),
            opt("mass", Kind::Quantity(Dim::MASS), "25 kg", "for its heat capacity"),
            opt("cooling", Kind::Number, "10", "heat loss to the bay, W per K"),
        ],
        outputs: &[("voltage", V), ("current", A), ("charge", N), ("remaining", Q), ("temperature", T), ("overheat", B), ("runaway", B), ("failed", B)],
    },
    PartSpec {
        domain: "electrical",
        kind: "generator",
        help: "A generator and its control unit: drive speed, constant or variable frequency, excitation build-up, a voltage regulator, over/under-voltage, overfrequency, overcurrent and overload trips, drive disconnect and drive oil temperature.",
        fields: &[
            req("bus", Kind::Ref(BUS), "the bus at its output"),
            req("voltage", Kind::Quantity(Dim::VOLTAGE), "regulated voltage"),
            opt("frequency", Kind::Quantity(Dim::FREQUENCY), "0 Hz", "0 for DC; for a variable-frequency generator, the frequency at rated drive speed"),
            req("rating", Kind::Quantity(Dim::POWER), "apparent power, for example \"90 kVA\""),
            opt("power_factor", Kind::Number, "1", "real power it delivers per VA of rating; 0.75 to 1 for aircraft AC generators"),
            opt("driven_when", WHEN, "true", "when its drive turns it and its control switch is on"),
            opt("speed", Kind::Expr(N), "1", "drive speed as a share of rated"),
            opt("variable_frequency", Kind::Bool, "false", "frequency follows drive speed, as on a VFG or a starter-generator"),
            opt("min_speed", Kind::Number, "0.5", "below this share of rated speed it goes off line"),
            opt("max_speed", Kind::Number, "1.2", "above this for 1 s it trips for overfrequency"),
            opt("excitation_time", Kind::Quantity(Dim::TIME), "0.2 s", "from turning fast enough to ready, while the field builds"),
            opt("regulator_time", Kind::Quantity(Dim::TIME), "0.5 s", "how fast its voltage regulator trims out load droop"),
            opt("protection", Kind::Bool, "true", "its control unit trips it on a fault"),
            opt("overload_time", Kind::Quantity(Dim::TIME), "300 s", "how long it carries 150 % before the overload trip"),
            opt("reset_when", WHEN, "false", "a trip clears on the rising edge of this, or when it stops being driven"),
            opt("disconnect_when", WHEN, "false", "disconnects the drive for good"),
            opt("oil_ambient", Kind::Expr(T), "333.15 K", "drive oil temperature off load, after its cooler"),
            opt("oil_rise", Kind::Quantity(Dim::TEMPERATURE), "60 K", "oil temperature rise at full load"),
            opt("oil_limit", Kind::Quantity(Dim::TEMPERATURE), "458.15 K", "oil overheat warning"),
            opt("oil_cooled_when", WHEN, "true", "its oil cooler works"),
        ],
        outputs: &[
            ("voltage", V),
            ("frequency", HZ),
            ("current", A),
            ("load", W),
            ("ready", B),
            ("tripped", B),
            ("fault", N),
            ("disconnected", B),
            ("oil_temperature", T),
            ("oil_overheat", B),
            ("failed", B),
        ],
    },
    PartSpec {
        domain: "electrical",
        kind: "ext_power",
        help: "Ground power.",
        fields: &[
            req("bus", Kind::Ref(BUS), "the bus at its output"),
            req("voltage", Kind::Quantity(Dim::VOLTAGE), "supply voltage"),
            opt("frequency", Kind::Quantity(Dim::FREQUENCY), "0 Hz", "0 for DC"),
            req("rating", Kind::Quantity(Dim::POWER), "supply rating, apparent power"),
            opt("power_factor", Kind::Number, "1", "real power it delivers per VA of rating"),
            opt("connected_when", WHEN, "false", "when the ground unit is plugged in"),
        ],
        outputs: &[("voltage", V), ("frequency", HZ), ("current", A), ("load", W), ("available", B), ("failed", B)],
    },
    PartSpec {
        domain: "electrical",
        kind: "contactor",
        help: "A switch between two buses.",
        fields: &[
            req("between", Kind::Refs(BUS), "two bus ids"),
            opt("close_when", WHEN, "true", "when it closes"),
            opt("close_delay", Kind::Quantity(Dim::TIME), "0 s", "how long close_when must hold before it closes; it opens at once"),
            opt("welded_when", WHEN, "false", "its contacts are welded shut"),
        ],
        outputs: &[("closed", B), ("current", A), ("failed", B)],
    },
    PartSpec {
        domain: "electrical",
        kind: "breaker",
        help: "A thermal circuit breaker that carries 115 % of its rating indefinitely, with a magnetic instant trip; pull it to reset. A fuse blows instead and stays open until replaced.",
        fields: &[
            req("between", Kind::Refs(BUS), "two bus ids"),
            req("rating", Kind::Quantity(Dim::CURRENT), "for example \"10 A\""),
            opt("trip_time", Kind::Quantity(Dim::TIME), "3 s", "time to trip at twice its rating, from cold"),
            opt("instant", Kind::Number, "10", "multiple of its rating that trips it at once"),
            opt("fuse", Kind::Bool, "false", "a fuse: pulling it does not reset it"),
            opt("replaced_when", WHEN, "false", "a blown fuse is replaced on the rising edge of this"),
        ],
        outputs: &[("closed", B), ("tripped", B), ("pulled", B), ("current", A), ("failed", B)],
    },
    PartSpec {
        domain: "electrical",
        kind: "diode",
        help: "Lets current through one way only, from the first bus to the second.",
        fields: &[
            req("between", Kind::Refs(BUS), "from bus, to bus"),
            opt("shorted_when", WHEN, "false", "failed short: conducts both ways"),
        ],
        outputs: &[("conducting", B), ("current", A), ("failed", B)],
    },
    PartSpec {
        domain: "electrical",
        kind: "load",
        help: "Something that draws power: constant power (electronics), resistive (heaters, lamps) or a motor with a starting inrush.",
        fields: &[
            req("bus", Kind::Ref(BUS), "the bus it draws from"),
            req("power", Kind::Expr(W), "for example \"250 W\", or an expression such as \"heater.value * 1 kW\"; resistive loads draw this at nominal voltage"),
            opt("on_when", WHEN, "true", "when it is switched on"),
            opt("shed_priority", Kind::Number, "0", "0 never shed; higher numbers are shed first"),
            opt("min_voltage", Kind::Quantity(Dim::VOLTAGE), "0 V", "below this it switches itself off, back on 0.5 s after the bus is 10 % above it; 0 V means half the bus nominal"),
            opt("type", Kind::Text, "constant_power", "constant_power, resistive or motor"),
            opt("inrush", Kind::Number, "5", "motor: starting power as a multiple of running power"),
            opt("start_time", Kind::Quantity(Dim::TIME), "1 s", "motor: how long the inrush takes to die away"),
            opt("power_factor", Kind::Number, "1", "real per apparent power, for generator loading"),
        ],
        outputs: &[("powered", B), ("current", A), ("power", W), ("shed", B), ("failed", B)],
    },
    PartSpec {
        domain: "electrical",
        kind: "tru",
        help: "Transformer-rectifier unit: AC in, DC out through its output diodes, constant-current limiting and overheat.",
        fields: &[
            req("input", Kind::Ref(BUS), "AC bus"),
            req("output", Kind::Ref(BUS), "DC bus"),
            req("voltage", Kind::Quantity(Dim::VOLTAGE), "DC output voltage"),
            req("rating", Kind::Quantity(Dim::POWER), "output rating"),
            opt("efficiency", Kind::Number, "0.9", "0 to 1"),
            opt("on_when", WHEN, "true", "when it runs"),
            opt("current_limit", Kind::Number, "1.5", "share of rating it holds as a constant current when overloaded"),
            opt("cooled_when", WHEN, "true", "its cooling fan or airflow works"),
            opt("overheat_temperature", Kind::Quantity(Dim::TEMPERATURE), "393.15 K", "it shuts down above this and restarts 20 K below"),
        ],
        outputs: CONVERTER_OUT,
    },
    PartSpec {
        domain: "electrical",
        kind: "inverter",
        help: "Static inverter: DC in, AC out, current limiting and overheat.",
        fields: &[
            req("input", Kind::Ref(BUS), "DC bus"),
            req("output", Kind::Ref(BUS), "AC bus"),
            req("voltage", Kind::Quantity(Dim::VOLTAGE), "AC output voltage"),
            opt("frequency", Kind::Quantity(Dim::FREQUENCY), "400 Hz", "output frequency"),
            req("rating", Kind::Quantity(Dim::POWER), "output rating"),
            opt("efficiency", Kind::Number, "0.85", "0 to 1"),
            opt("on_when", WHEN, "true", "when it runs"),
            opt("current_limit", Kind::Number, "1.5", "share of rating it holds as a constant current when overloaded"),
            opt("cooled_when", WHEN, "true", "its cooling works"),
            opt("overheat_temperature", Kind::Quantity(Dim::TEMPERATURE), "373.15 K", "it shuts down above this and restarts 20 K below"),
        ],
        outputs: CONVERTER_OUT,
    },
    PartSpec {
        domain: "electrical",
        kind: "charger",
        help: "Battery charger: constant current up to its float voltage, then constant voltage.",
        fields: &[
            req("input", Kind::Ref(BUS), "the bus it draws from"),
            req("output", Kind::Ref(BUS), "the battery's bus"),
            req("voltage", Kind::Quantity(Dim::VOLTAGE), "float voltage"),
            req("current", Kind::Quantity(Dim::CURRENT), "constant-current charge rate"),
            opt("efficiency", Kind::Number, "0.85", "0 to 1"),
            opt("on_when", WHEN, "true", "when it charges"),
            opt("cooled_when", WHEN, "true", "its cooling works"),
            opt("overheat_temperature", Kind::Quantity(Dim::TEMPERATURE), "373.15 K", "it shuts down above this and restarts 20 K below"),
        ],
        outputs: CONVERTER_OUT,
    },
];

const SWITCH_OHM: f64 = 1e-4;
const DIODE_OHM: f64 = 5e-3;
/// Reverse leakage at each end of a diode, so an unloaded side still shows which way current wants to go.
const DIODE_LEAK: f64 = 1e-3;
const REGULATION: f64 = 0.02;
const UNSHED_MARGIN: f64 = 0.9;
/// Below this share of charge a battery's open-circuit voltage falls away to nothing.
const KNEE: f64 = 0.1;
/// A dropped-out load comes back once its bus is this far above its minimum voltage for `RECOVER_S`.
const RECOVER_RATIO: f64 = 1.1;
const RECOVER_S: f64 = 0.5;
/// Constant-power loads are linearised at last step's voltage, never below this share of nominal: a
/// lagged constant-power load diverges once its conductance passes the source's. Loads drop out at their
/// minimum voltage above it, so the floor only holds through a transient.
const LINEARISE_FLOOR: f64 = 0.5;
const T_REF: f64 = 288.15;
/// Specific heat of a battery's cells and case, J/kg/K.
const BATTERY_C: f64 = 1100.0;
/// A cell this far past its runaway temperature has vented and gone open circuit.
const DESTROYED_ABOVE: f64 = 60.0;
const OIL_TAU: f64 = 90.0;
const CONVERTER_TAU: f64 = 240.0;
const CONVERTER_BAY: f64 = 303.15;
/// Thermal breakers carry this share of their rating indefinitely.
const BREAKER_HOLD: f64 = 1.15;
/// Most a voltage regulator raises its set point to trim out droop.
const TRIM_MAX: f64 = 0.05;

#[derive(Clone, Copy)]
struct Chemistry {
    base: f64,
    slope: f64,
    peukert: f64,
    charge_eff: f64,
    cold_per_k: f64,
    overheat: f64,
    runaway: f64,
}

fn chemistry(name: &str) -> Chemistry {
    match name {
        "lead_acid" => Chemistry { base: 0.87, slope: 0.18, peukert: 1.2, charge_eff: 0.85, cold_per_k: 0.012, overheat: 323.15, runaway: 343.15 },
        "lithium" => Chemistry { base: 0.84, slope: 0.21, peukert: 1.03, charge_eff: 0.98, cold_per_k: 0.008, overheat: 343.15, runaway: 403.15 },
        _ => Chemistry { base: 0.88, slope: 0.17, peukert: 1.05, charge_eff: 0.83, cold_per_k: 0.009, overheat: 333.15, runaway: 353.15 },
    }
}

/// Open-circuit voltage at a state of charge: a gentle slope over the useful range, then collapse.
fn ocv(c: &Chemistry, nominal: f64, charge: f64) -> f64 {
    let at_knee = c.base + c.slope * KNEE;
    nominal * if charge >= KNEE { c.base + c.slope * charge } else { at_knee * charge.max(0.0) / KNEE }
}

fn eval(e: &mut Option<Expr>, slots: &[f64], dt: f64, or: f64) -> f64 {
    e.as_mut().map_or(or, |x| x.eval(slots, dt))
}

fn yes(e: &mut Option<Expr>, slots: &[f64], dt: f64, or: bool) -> bool {
    e.as_mut().map_or(or, |x| x.yes(slots, dt))
}

struct Bus {
    node: usize,
    nominal: f64,
    powered_at: f64,
    short: Option<Expr>,
    fault_g: f64,
    s: Slots,
}

struct Battery {
    node: usize,
    nominal: f64,
    capacity: f64,
    g: f64,
    limit: f64,
    g_now: f64,
    charge: f64,
    chem: Chemistry,
    ambient: Option<Expr>,
    heat_capacity: f64,
    cooling: f64,
    temp: f64,
    amb: f64,
    destroyed: bool,
    s: Slots,
}

#[derive(Default)]
struct Gcu {
    heat: f64,
    tripped: bool,
    fault: f64,
    ov: f64,
    uv: f64,
    oc: f64,
    os: f64,
    disconnected: bool,
    oil: f64,
    reset_was: bool,
    excited: f64,
    trim: f64,
}

struct Source {
    node: usize,
    voltage: f64,
    frequency: f64,
    rating: f64,
    power_factor: f64,
    g: f64,
    when: Option<Expr>,
    ext: bool,
    speed: Option<Expr>,
    variable: bool,
    min_speed: f64,
    max_speed: f64,
    excitation: f64,
    regulator: f64,
    protect: bool,
    overload_heat: f64,
    reset: Option<Expr>,
    disconnect: Option<Expr>,
    oil_ambient: Option<Expr>,
    oil_rise: f64,
    oil_limit: f64,
    oil_cooled: Option<Expr>,
    on: bool,
    f_now: f64,
    gcu: Gcu,
    s: Slots,
}

impl Source {
    fn trip(&mut self, code: f64) {
        if !self.gcu.tripped {
            self.gcu.tripped = true;
            self.gcu.fault = code;
        }
    }

    fn set_point(&self) -> f64 {
        self.voltage * (1.0 + self.gcu.trim)
    }
}

#[derive(Clone, Copy, PartialEq)]
enum SwitchType {
    Contactor,
    Breaker,
    Diode,
}

struct Switch {
    kind: SwitchType,
    link: usize,
    ends: (usize, usize),
    when: Option<Expr>,
    welded: Option<Expr>,
    delay: f64,
    held: f64,
    rating: f64,
    trip_heat: f64,
    instant: f64,
    fuse: bool,
    replaced: Option<Expr>,
    replaced_was: bool,
    heat: f64,
    tripped: bool,
    conducting: bool,
    s: Slots,
}

#[derive(Clone, Copy, PartialEq)]
enum LoadType {
    Power,
    Resistive,
    Motor,
}

struct Load {
    bus: usize,
    node: usize,
    power: f64,
    power_e: Option<Expr>,
    when: Option<Expr>,
    priority: f64,
    shed: bool,
    min_voltage: f64,
    dropped: bool,
    recover: f64,
    g: f64,
    kind: LoadType,
    inrush: f64,
    start_time: f64,
    started: f64,
    power_factor: f64,
    s: Slots,
}

struct Converter {
    input: usize,
    in_node: usize,
    out_node: usize,
    out_bus: usize,
    voltage: f64,
    frequency: f64,
    rating: f64,
    amps: f64,
    eff: f64,
    g: f64,
    when: Option<Expr>,
    cooled: Option<Expr>,
    limit: f64,
    overheat_at: f64,
    drawn: f64,
    e: f64,
    temp: f64,
    hot: bool,
    s: Slots,
}

pub struct Electrical {
    net: Network,
    ports: crate::ports::Ports,
    buses: Vec<Bus>,
    bus_of: HashMap<usize, usize>,
    batteries: Vec<Battery>,
    sources: Vec<Source>,
    switches: Vec<Switch>,
    loads: Vec<Load>,
    converters: Vec<Converter>,
    /// Loads that can be shed, highest priority (shed first) first.
    shed_order: Vec<usize>,
    scratch: Scratch,
}

/// Per-step working space, kept to avoid allocating every step.
#[derive(Default)]
struct Scratch {
    powered: Vec<bool>,
    wanted: Vec<bool>,
    islands: Vec<usize>,
    capacity: Vec<f64>,
    demand: Vec<f64>,
    has_capacity: Vec<bool>,
    frequency: Vec<f64>,
}

impl Electrical {
    pub fn new(c: &Checked) -> Option<Self> {
        let parts: Vec<(usize, &CheckedPart)> = c.parts.iter().enumerate().filter(|(_, p)| p.domain == "electrical").collect();
        if parts.is_empty() {
            return None;
        }
        let mut net = Network::new();
        let mut e = Electrical { net: Network::new(), ports: Default::default(), buses: vec![], bus_of: HashMap::new(), batteries: vec![], sources: vec![], switches: vec![], loads: vec![], converters: vec![], shed_order: vec![], scratch: Scratch::default() };
        for &(i, p) in parts.iter().filter(|(_, p)| p.kind == "bus") {
            let node = net.add_node(0.0);
            e.bus_of.insert(i, e.buses.len());
            e.buses.push(Bus {
                node,
                nominal: p.num("nominal"),
                powered_at: p.num("powered_at"),
                short: expr_of(p, "short_when"),
                fault_g: 1.0 / p.num("fault_resistance").max(1e-6),
                s: Slots::of(p, &["voltage", "frequency", "powered", "failed"]),
            });
        }
        let node = |e: &Electrical, part: Option<usize>| part.and_then(|x| e.bus_of.get(&x)).map(|&b| e.buses[b].node);
        for &(_, p) in &parts {
            match p.kind {
                "battery" => {
                    let Some(n) = node(&e, p.part("bus")) else { continue };
                    let g = 1.0 / p.num("resistance").max(1e-6);
                    e.batteries.push(Battery {
                        node: n,
                        nominal: p.num("voltage"),
                        capacity: p.num("capacity").max(1.0),
                        g,
                        limit: p.num("charge_limit").max(0.1),
                        g_now: g,
                        charge: p.num("charge").clamp(0.0, 1.0),
                        chem: chemistry(p.text("chemistry")),
                        ambient: expr_of(p, "ambient"),
                        heat_capacity: p.num("mass").max(0.1) * BATTERY_C,
                        cooling: p.num("cooling").max(0.0),
                        temp: f64::NAN,
                        amb: T_REF,
                        destroyed: false,
                        s: Slots::of(p, &["voltage", "current", "charge", "remaining", "temperature", "overheat", "runaway", "failed"]),
                    });
                }
                "generator" | "ext_power" => {
                    let Some(n) = node(&e, p.part("bus")) else { continue };
                    let (v, r) = (p.num("voltage"), p.num("rating").max(1.0));
                    let ext = p.kind == "ext_power";
                    let names: &[&'static str] = if ext {
                        &["voltage", "frequency", "current", "load", "available", "failed"]
                    } else {
                        &["voltage", "frequency", "current", "load", "ready", "tripped", "fault", "disconnected", "oil_temperature", "oil_overheat", "failed"]
                    };
                    let gen = |k: &str| if ext { 0.0 } else { p.num(k) };
                    let gen_expr = |k: &str| if ext { None } else { expr_of(p, k) };
                    e.sources.push(Source {
                        node: n,
                        voltage: v,
                        frequency: p.num("frequency"),
                        rating: r,
                        power_factor: p.num("power_factor").clamp(0.05, 1.0),
                        g: r / (REGULATION * v * v).max(1e-9),
                        when: expr_of(p, if ext { "connected_when" } else { "driven_when" }),
                        ext,
                        speed: gen_expr("speed"),
                        variable: !ext && p.flag("variable_frequency"),
                        min_speed: gen("min_speed"),
                        max_speed: if ext { f64::INFINITY } else { p.num("max_speed") },
                        excitation: gen("excitation_time"),
                        regulator: gen("regulator_time").max(0.01),
                        protect: !ext && p.flag("protection"),
                        overload_heat: gen("overload_time") * 1.25,
                        reset: gen_expr("reset_when"),
                        disconnect: gen_expr("disconnect_when"),
                        oil_ambient: gen_expr("oil_ambient"),
                        oil_rise: gen("oil_rise"),
                        oil_limit: gen("oil_limit"),
                        oil_cooled: gen_expr("oil_cooled_when"),
                        on: false,
                        f_now: 0.0,
                        gcu: Gcu { oil: 333.15, ..Default::default() },
                        s: Slots::of(p, names),
                    });
                }
                "contactor" | "breaker" | "diode" => {
                    let ends = p.parts("between");
                    let (Some(a), Some(b)) = (node(&e, ends.first().copied()), node(&e, ends.get(1).copied())) else { continue };
                    let kind = match p.kind {
                        "breaker" => SwitchType::Breaker,
                        "diode" => SwitchType::Diode,
                        _ => SwitchType::Contactor,
                    };
                    let link = net.add_link(a, b, if kind == SwitchType::Diode { DIODE_OHM } else { SWITCH_OHM }, 0.0);
                    let names: &[&'static str] = match kind {
                        SwitchType::Breaker => &["closed", "tripped", "pulled", "current", "failed"],
                        SwitchType::Diode => &["conducting", "current", "failed"],
                        SwitchType::Contactor => &["closed", "current", "failed"],
                    };
                    let breaker = kind == SwitchType::Breaker;
                    e.switches.push(Switch {
                        kind,
                        link,
                        ends: (a, b),
                        when: if kind == SwitchType::Contactor { expr_of(p, "close_when") } else { None },
                        welded: match kind {
                            SwitchType::Contactor => expr_of(p, "welded_when"),
                            SwitchType::Diode => expr_of(p, "shorted_when"),
                            SwitchType::Breaker => None,
                        },
                        delay: if kind == SwitchType::Contactor { p.num("close_delay") } else { 0.0 },
                        held: 0.0,
                        rating: if breaker { p.num("rating") } else { 0.0 },
                        trip_heat: if breaker { ((2.0 / BREAKER_HOLD).powi(2) - 1.0) * p.num("trip_time").max(0.01) } else { 0.0 },
                        instant: if breaker { p.num("instant").max(1.0) } else { 0.0 },
                        fuse: breaker && p.flag("fuse"),
                        replaced: if breaker { expr_of(p, "replaced_when") } else { None },
                        replaced_was: false,
                        heat: 0.0,
                        tripped: false,
                        conducting: false,
                        s: Slots::of(p, names),
                    });
                }
                "load" => {
                    let Some(&b) = p.part("bus").and_then(|x| e.bus_of.get(&x)) else { continue };
                    let min = p.num("min_voltage");
                    let min_voltage = if min > 0.0 { min } else { 0.5 * e.buses[b].nominal };
                    e.loads.push(Load {
                        bus: b,
                        node: e.buses[b].node,
                        power: 0.0,
                        power_e: expr_of(p, "power"),
                        when: expr_of(p, "on_when"),
                        priority: p.num("shed_priority"),
                        shed: false,
                        min_voltage,
                        dropped: false,
                        recover: 0.0,
                        g: 0.0,
                        kind: match p.text("type") {
                            "resistive" => LoadType::Resistive,
                            "motor" => LoadType::Motor,
                            _ => LoadType::Power,
                        },
                        inrush: p.num("inrush").max(1.0),
                        start_time: p.num("start_time").max(0.01),
                        started: 0.0,
                        power_factor: p.num("power_factor").clamp(0.05, 1.0),
                        s: Slots::of(p, &["powered", "current", "power", "shed", "failed"]),
                    });
                }
                "tru" | "inverter" | "charger" => {
                    let (Some(&ib), Some(&ob)) = (p.part("input").and_then(|x| e.bus_of.get(&x)), p.part("output").and_then(|x| e.bus_of.get(&x))) else { continue };
                    let v = p.num("voltage");
                    let charger = p.kind == "charger";
                    let amps = if charger { p.num("current").max(0.01) } else { 0.0 };
                    let r = if charger { amps * v } else { p.num("rating").max(1.0) };
                    e.converters.push(Converter {
                        input: ib,
                        in_node: e.buses[ib].node,
                        out_node: e.buses[ob].node,
                        out_bus: ob,
                        voltage: v,
                        frequency: if p.kind == "inverter" { p.num("frequency") } else { 0.0 },
                        rating: r,
                        amps,
                        eff: p.num("efficiency").clamp(0.05, 1.0),
                        g: r / (REGULATION * v * v).max(1e-9),
                        when: expr_of(p, "on_when"),
                        cooled: expr_of(p, "cooled_when"),
                        limit: if charger { f64::INFINITY } else { p.num("current_limit").max(0.1) },
                        overheat_at: p.num("overheat_temperature"),
                        drawn: 0.0,
                        e: v,
                        temp: CONVERTER_BAY,
                        hot: false,
                        s: Slots::of(p, &["current", "power", "active", "temperature", "overheat", "limiting", "failed"]),
                    });
                }
                _ => {}
            }
        }
        e.ports = crate::ports::Ports::new(c, "electrical", &mut net, |i| e.bus_of.get(&i).map(|&b| e.buses[b].node));
        e.net = net;
        e.shed_order = (0..e.loads.len()).filter(|&k| e.loads[k].priority > 0.0).collect();
        e.shed_order.sort_by(|&a, &b| e.loads[b].priority.total_cmp(&e.loads[a].priority));
        Some(e)
    }

    fn bus_powered(&self, slots: &[f64], b: usize) -> bool {
        self.buses[b].s.on(slots, "powered")
    }

    fn step_switches(&mut self, slots: &mut [f64], dt: f64) {
        for sw in &mut self.switches {
            let failed = sw.s.on(slots, "failed");
            let closed = match sw.kind {
                SwitchType::Breaker => {
                    let replaced = yes(&mut sw.replaced, slots, dt, false);
                    if sw.fuse && replaced && !sw.replaced_was {
                        sw.tripped = false;
                        sw.heat = 0.0;
                    }
                    sw.replaced_was = replaced;
                    !failed && !sw.tripped && !sw.s.on(slots, "pulled")
                }
                SwitchType::Contactor => {
                    let want = !failed && yes(&mut sw.when, slots, dt, true);
                    sw.held = if want { sw.held + dt } else { 0.0 };
                    (want && sw.held >= sw.delay) || yes(&mut sw.welded, slots, dt, false)
                }
                SwitchType::Diode => {
                    let forward = self.net.potential(sw.ends.0) - self.net.potential(sw.ends.1);
                    sw.conducting = !failed && if sw.conducting { self.net.flow(sw.link) >= -1e-5 } else { forward > 1e-3 };
                    sw.conducting || yes(&mut sw.welded, slots, dt, false)
                }
            };
            self.net.set_closed(sw.link, closed);
            sw.s.flag(slots, if sw.kind == SwitchType::Diode { "conducting" } else { "closed" }, closed);
        }
    }

    fn step_sources(&mut self, slots: &mut [f64], dt: f64) {
        for src in &mut self.sources {
            let failed = src.s.on(slots, "failed");
            if src.ext {
                src.on = !failed && yes(&mut src.when, slots, dt, false);
                src.f_now = src.frequency;
                src.s.flag(slots, "available", src.on);
            } else {
                if yes(&mut src.disconnect, slots, dt, false) {
                    src.gcu.disconnected = true;
                }
                let speed = eval(&mut src.speed, slots, dt, 1.0);
                let driven = !failed && !src.gcu.disconnected && yes(&mut src.when, slots, dt, true);
                let reset = yes(&mut src.reset, slots, dt, false);
                if !driven || (reset && !src.gcu.reset_was) {
                    src.gcu.tripped = false;
                    src.gcu.fault = 0.0;
                }
                src.gcu.reset_was = reset;
                let spinning = driven && speed >= src.min_speed;
                if src.protect && spinning && speed > src.max_speed {
                    src.gcu.os += dt;
                    if src.gcu.os > 1.0 {
                        src.trip(3.0);
                    }
                } else {
                    src.gcu.os = 0.0;
                }
                src.gcu.excited = if spinning { src.gcu.excited + dt } else { 0.0 };
                src.on = spinning && !src.gcu.tripped && src.gcu.excited >= src.excitation;
                if !src.on {
                    src.gcu.trim = 0.0;
                }
                src.f_now = if src.variable { src.frequency * speed.max(0.0) } else { src.frequency };
                let share = src.s.get(slots, "load") / (src.rating * src.power_factor);
                let cooled = yes(&mut src.oil_cooled, slots, dt, true);
                let amb = eval(&mut src.oil_ambient, slots, dt, 333.15);
                let target = if spinning { amb + src.oil_rise * (0.4 + 0.6 * share.clamp(0.0, 2.0).powi(2)) * if cooled { 1.0 } else { 3.0 } } else { amb };
                src.gcu.oil += (target - src.gcu.oil) * (dt / OIL_TAU).min(1.0);
                src.s.flag(slots, "ready", src.on);
                src.s.flag(slots, "tripped", src.gcu.tripped);
                src.s.put(slots, "fault", src.gcu.fault);
                src.s.flag(slots, "disconnected", src.gcu.disconnected);
                src.s.put(slots, "oil_temperature", src.gcu.oil);
                src.s.flag(slots, "oil_overheat", src.gcu.oil > src.oil_limit);
            }
            if src.on {
                self.net.stamp(src.node, src.g, src.g * src.set_point());
            }
        }
    }

    fn protect_sources(&mut self, slots: &mut [f64], dt: f64) {
        for src in &mut self.sources {
            let v = self.net.potential(src.node);
            let i = if src.on { src.g * (src.set_point() - v) } else { 0.0 };
            let load = i * v.max(0.0);
            src.s.put(slots, "voltage", if src.on { v.max(0.0) } else { 0.0 });
            src.s.put(slots, "frequency", if src.on { src.f_now } else { 0.0 });
            src.s.put(slots, "current", i);
            src.s.put(slots, "load", load);
            if src.on && !src.ext {
                src.gcu.trim = (src.gcu.trim + (src.voltage - v) / src.voltage * dt / src.regulator).clamp(0.0, TRIM_MAX);
            }
            if !src.protect || !src.on {
                src.gcu.ov = 0.0;
                src.gcu.uv = 0.0;
                src.gcu.oc = 0.0;
                src.gcu.heat = (src.gcu.heat - dt).max(0.0);
                continue;
            }
            let timer = |t: &mut f64, bad: bool| {
                *t = if bad { *t + dt } else { 0.0 };
                *t
            };
            if timer(&mut src.gcu.ov, v > 1.12 * src.voltage) > 0.25 {
                src.trip(1.0);
            }
            if timer(&mut src.gcu.uv, v < 0.85 * src.voltage) > 4.0 {
                src.trip(2.0);
            }
            if timer(&mut src.gcu.oc, i > 3.0 * src.rating / src.voltage) > 0.5 {
                src.trip(6.0);
            }
            let share = (load / (src.rating * src.power_factor)).min(3.0);
            src.gcu.heat = (src.gcu.heat + (share * share - 1.0) * dt).max(0.0);
            if src.gcu.heat > src.overload_heat {
                src.trip(5.0);
            }
        }
    }

    fn step_batteries(&mut self, slots: &mut [f64], dt: f64) {
        for bat in &mut self.batteries {
            let v = self.net.potential(bat.node);
            let ocv = ocv(&bat.chem, bat.nominal, bat.charge);
            let live = !bat.s.on(slots, "failed") && !bat.destroyed;
            let i = if live { bat.g_now * (ocv - v) } else { 0.0 };
            let usable = bat.capacity * (1.0 - bat.chem.cold_per_k * (T_REF - bat.temp).max(0.0)).max(0.3);
            let one_c = bat.capacity / 3600.0;
            let drain = if i > 0.0 { i * (i / one_c).max(1.0).powf(bat.chem.peukert - 1.0) } else { i * bat.chem.charge_eff };
            bat.charge = (bat.charge - drain * dt / usable).clamp(0.0, 1.0);
            let amb = bat.amb;
            let r = (0.02 * (T_REF - bat.temp)).exp().clamp(0.3, 8.0) / bat.g;
            let runaway = bat.temp > bat.chem.runaway;
            let exo = if runaway { bat.heat_capacity * 0.01 * (bat.temp - bat.chem.runaway + 1.0) } else { 0.0 };
            bat.temp += (i * i * r + exo - bat.cooling * (bat.temp - amb)) * dt / bat.heat_capacity;
            if bat.temp > bat.chem.runaway + DESTROYED_ABOVE {
                bat.destroyed = true;
            }
            bat.s.put(slots, "voltage", v.max(0.0));
            bat.s.put(slots, "current", i);
            bat.s.put(slots, "charge", bat.charge);
            bat.s.put(slots, "remaining", bat.charge * usable);
            bat.s.put(slots, "temperature", bat.temp);
            bat.s.flag(slots, "overheat", bat.temp > bat.chem.overheat);
            bat.s.flag(slots, "runaway", runaway || bat.destroyed);
        }
    }
}

impl Domain for Electrical {
    fn name(&self) -> &'static str {
        "electrical"
    }

    fn step(&mut self, slots: &mut [f64], dt: f64) {
        self.step_switches(slots, dt);
        self.net.begin_step();
        for sw in self.switches.iter().filter(|s| s.kind == SwitchType::Diode) {
            self.net.stamp(sw.ends.0, DIODE_LEAK, 0.0);
            self.net.stamp(sw.ends.1, DIODE_LEAK, 0.0);
        }
        for b in &mut self.buses {
            if b.s.on(slots, "failed") {
                self.net.stamp(b.node, easyacft_core::STIFF, 0.0);
            } else if yes(&mut b.short, slots, dt, false) {
                self.net.stamp(b.node, b.fault_g, 0.0);
            }
        }
        for bat in &mut self.batteries {
            bat.amb = eval(&mut bat.ambient, slots, dt, T_REF);
            if bat.temp.is_nan() {
                bat.temp = bat.amb;
            }
            if !bat.s.on(slots, "failed") && !bat.destroyed {
                let ocv = ocv(&bat.chem, bat.nominal, bat.charge);
                let g = bat.g / (0.02 * (T_REF - bat.temp)).exp().clamp(0.3, 8.0);
                let over = bat.s.get(slots, "voltage") - ocv;
                bat.g_now = if over > 0.0 { g.min(bat.limit / over) } else { g };
                self.net.stamp(bat.node, bat.g_now, bat.g_now * ocv);
            }
        }
        self.step_sources(slots, dt);
        let mut sc = std::mem::take(&mut self.scratch);
        sc.powered.clear();
        sc.powered.extend((0..self.buses.len()).map(|b| self.bus_powered(slots, b)));
        for cv in &mut self.converters {
            let cooled = yes(&mut cv.cooled, slots, dt, true);
            let share = cv.drawn / cv.rating;
            let target = CONVERTER_BAY + 50.0 * share * share * if cooled { 1.0 } else { 4.0 };
            cv.temp += (target - cv.temp) * (dt / CONVERTER_TAU).min(1.0);
            cv.hot = if cv.hot { cv.temp > cv.overheat_at - 20.0 } else { cv.temp > cv.overheat_at };
            let on = !cv.s.on(slots, "failed") && !cv.hot && sc.powered[cv.input] && yes(&mut cv.when, slots, dt, true);
            cv.s.flag(slots, "active", on);
            cv.s.put(slots, "temperature", cv.temp);
            cv.s.flag(slots, "overheat", cv.hot);
            let v_out = self.buses[cv.out_bus].s.get(slots, "voltage");
            if on && v_out < cv.e + 0.02 * cv.voltage {
                self.net.stamp(cv.out_node, cv.g, cv.g * cv.e);
            }
            if on {
                let vin = self.buses[cv.input].s.get(slots, "voltage").max(0.3 * self.buses[cv.input].nominal);
                self.net.stamp(cv.in_node, cv.drawn / cv.eff / (vin * vin), 0.0);
            }
        }
        sc.islands.clear();
        sc.islands.extend_from_slice(self.net.islands_of());
        let count = sc.islands.iter().max().map_or(0, |m| m + 1);
        for v in [&mut sc.capacity, &mut sc.demand, &mut sc.frequency] {
            v.clear();
            v.resize(count, 0.0);
        }
        sc.has_capacity.clear();
        sc.has_capacity.resize(count, false);
        let islands = &sc.islands;
        for src in &self.sources {
            if src.on {
                let isl = islands[src.node];
                sc.capacity[isl] += src.rating * src.power_factor;
                sc.has_capacity[isl] = true;
                sc.frequency[isl] = sc.frequency[isl].max(src.f_now);
            }
        }
        for cv in &self.converters {
            if cv.s.on(slots, "active") && cv.amps == 0.0 {
                let isl = islands[cv.out_node];
                sc.capacity[isl] += cv.rating;
                sc.has_capacity[isl] = true;
                sc.frequency[isl] = sc.frequency[isl].max(cv.frequency);
            }
        }
        sc.wanted.clear();
        for l in self.loads.iter_mut() {
            l.power = l.power_e.as_mut().map_or(0.0, |e| e.eval(slots, dt).max(0.0));
            let wanted = !l.s.on(slots, "failed") && yes(&mut l.when, slots, dt, true) && sc.powered[l.bus];
            sc.wanted.push(wanted);
            if wanted && !l.shed && !l.dropped {
                sc.demand[islands[l.node]] += l.power / l.power_factor;
            }
        }
        for cv in &self.converters {
            if cv.s.on(slots, "active") {
                sc.demand[islands[cv.in_node]] += cv.drawn / cv.eff;
            }
        }
        for &k in &self.shed_order {
            let isl = islands[self.loads[k].node];
            if !sc.has_capacity[isl] || !sc.wanted[k] {
                continue;
            }
            let (cap, dem) = (sc.capacity[isl], sc.demand[isl]);
            let l = &mut self.loads[k];
            let va = l.power / l.power_factor;
            if !l.shed && dem > cap {
                l.shed = true;
                sc.demand[isl] -= va;
            } else if l.shed && dem + va <= cap * UNSHED_MARGIN {
                l.shed = false;
                sc.demand[isl] += va;
            }
        }
        for (k, l) in self.loads.iter_mut().enumerate() {
            l.g = 0.0;
            if sc.wanted[k] && !l.shed && !l.dropped {
                let b = &self.buses[l.bus];
                let v = b.s.get(slots, "voltage").max(LINEARISE_FLOOR * b.nominal);
                l.g = match l.kind {
                    LoadType::Resistive => l.power / (b.nominal * b.nominal).max(1e-9),
                    LoadType::Power => l.power / (v * v),
                    LoadType::Motor => {
                        let starting = (1.0 - l.started / l.start_time).max(0.0);
                        l.started += dt;
                        l.power * (1.0 + (l.inrush - 1.0) * starting) / (v * v)
                    }
                };
                self.net.stamp(l.node, l.g, 0.0);
            } else {
                l.started = 0.0;
            }
        }
        self.ports.stamp(&mut self.net, slots);
        self.net.step(dt);
        self.ports.read(&self.net, slots);
        for b in &self.buses {
            let v = self.net.potential(b.node).max(0.0);
            let powered = v >= b.powered_at * b.nominal;
            b.s.put(slots, "voltage", v);
            b.s.flag(slots, "powered", powered);
            b.s.put(slots, "frequency", if powered { sc.frequency.get(islands[b.node]).copied().unwrap_or(0.0) } else { 0.0 });
        }
        self.step_batteries(slots, dt);
        self.protect_sources(slots, dt);
        for cv in &mut self.converters {
            let on = cv.s.on(slots, "active");
            let v = self.net.potential(cv.out_node);
            let i = if on { (cv.g * (cv.e - v)).max(0.0) } else { 0.0 };
            let i_lim = if cv.amps > 0.0 { cv.amps } else { cv.limit * cv.rating / v.max(0.3 * cv.voltage) };
            cv.e = if on { (cv.e + 0.5 * (i_lim - i) / cv.g).clamp(0.0, cv.voltage) } else { cv.voltage };
            cv.drawn = (i * v).max(0.0);
            cv.s.put(slots, "current", i);
            cv.s.put(slots, "power", cv.drawn);
            cv.s.flag(slots, "limiting", on && cv.e < 0.999 * cv.voltage);
        }
        for (k, l) in self.loads.iter_mut().enumerate() {
            let v = self.net.potential(l.node).max(0.0);
            let drawn = l.g * v * v;
            let wanted = sc.wanted[k] && !l.shed;
            if !wanted {
                l.dropped = false;
                l.recover = 0.0;
            } else if !l.dropped && v < l.min_voltage {
                l.dropped = true;
                l.recover = 0.0;
            } else if l.dropped {
                l.recover = if v >= l.min_voltage * RECOVER_RATIO { l.recover + dt } else { 0.0 };
                if l.recover >= RECOVER_S {
                    l.dropped = false;
                }
            }
            l.s.flag(slots, "powered", wanted && !l.dropped);
            l.s.flag(slots, "shed", l.shed && sc.wanted[k]);
            l.s.put(slots, "power", drawn);
            l.s.put(slots, "current", if v > 0.0 { drawn / v } else { 0.0 });
        }
        for sw in &mut self.switches {
            let i = self.net.flow(sw.link);
            sw.s.put(slots, "current", i);
            if sw.kind == SwitchType::Breaker {
                let r = (i / (BREAKER_HOLD * sw.rating)).powi(2);
                sw.heat = (sw.heat + (r - 1.0) * dt).max(0.0);
                if sw.heat > sw.trip_heat || i.abs() > sw.instant * sw.rating {
                    sw.tripped = true;
                }
                if sw.tripped && !sw.fuse && sw.s.on(slots, "pulled") {
                    sw.tripped = false;
                    sw.heat = 0.0;
                }
                sw.s.flag(slots, "tripped", sw.tripped);
            }
        }
        self.scratch = sc;
    }

    fn state(&self) -> Vec<f64> {
        let b = |x: bool| x as u8 as f64;
        let mut v = self.net.state();
        v.extend(self.batteries.iter().flat_map(|x| [x.charge, x.temp, b(x.destroyed)]));
        v.extend(self.sources.iter().flat_map(|s| {
            let g = &s.gcu;
            [g.heat, b(g.tripped), g.fault, g.ov, g.uv, g.oc, g.os, b(g.disconnected), g.oil, b(g.reset_was), g.excited, g.trim]
        }));
        v.extend(self.switches.iter().flat_map(|s| [s.heat, b(s.tripped), s.held, b(s.conducting), b(s.replaced_was)]));
        v.extend(self.loads.iter().flat_map(|l| [b(l.shed), b(l.dropped), l.recover, l.started]));
        v.extend(self.converters.iter().flat_map(|c| [c.drawn, c.e, c.temp, b(c.hot)]));
        v
    }

    fn restore(&mut self, s: &[f64]) {
        let n = self.net.state().len();
        if s.len() < n {
            return;
        }
        self.net.restore(&s[..n]);
        let mut it = s[n..].iter().copied();
        let mut next = || it.next().unwrap_or(0.0);
        for x in &mut self.batteries {
            x.charge = next();
            x.temp = next();
            x.destroyed = next() != 0.0;
        }
        for src in &mut self.sources {
            let g = &mut src.gcu;
            g.heat = next();
            g.tripped = next() != 0.0;
            g.fault = next();
            g.ov = next();
            g.uv = next();
            g.oc = next();
            g.os = next();
            g.disconnected = next() != 0.0;
            g.oil = next();
            g.reset_was = next() != 0.0;
            g.excited = next();
            g.trim = next();
        }
        for sw in &mut self.switches {
            sw.heat = next();
            sw.tripped = next() != 0.0;
            sw.held = next();
            sw.conducting = next() != 0.0;
            sw.replaced_was = next() != 0.0;
        }
        for l in &mut self.loads {
            l.shed = next() != 0.0;
            l.dropped = next() != 0.0;
            l.recover = next();
            l.started = next();
        }
        for c in &mut self.converters {
            c.drawn = next();
            c.e = next();
            c.temp = next();
            c.hot = next() != 0.0;
        }
    }
}
