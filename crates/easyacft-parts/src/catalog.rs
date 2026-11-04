//! Built-in part kinds.

use easyacft_core::Dim;
use easyacft_lang::schema::{opt, req, Catalog, Kind, PartSpec};
use easyacft_lang::Type;

const B: Type = Type::Bool;
const V: Type = Type::Num(Dim::VOLTAGE);
const A: Type = Type::Num(Dim::CURRENT);
const W: Type = Type::Num(Dim::POWER);
const HZ: Type = Type::Num(Dim::FREQUENCY);
const N: Type = Type::Num(Dim::NONE);
const BUS: &[&str] = &["electrical.bus"];
const WHEN: Kind = Kind::Expr(Type::Bool);

pub static CORE: &[PartSpec] = &[
    PartSpec {
        domain: "cockpit",
        kind: "switch",
        help: "A cockpit switch or push-button; set `<id>.on` from a binding or the API.",
        fields: &[opt("initial", Kind::Bool, "false", "position at start")],
        outputs: &[("on", B), ("failed", B)],
    },
    PartSpec {
        domain: "cockpit",
        kind: "guarded_switch",
        help: "A switch under a guard; `<id>.on` only moves while `<id>.guard_open` is true.",
        fields: &[opt("initial", Kind::Bool, "false", "position at start")],
        outputs: &[("on", B), ("guard_open", B), ("failed", B)],
    },
    PartSpec {
        domain: "cockpit",
        kind: "pushbutton",
        help: "A latching push-button with its lights.",
        fields: &[
            opt("initial", Kind::Bool, "false", "latched at start"),
            opt("fault_when", WHEN, "false", "when the FAULT light shows"),
            opt("test", Kind::Ref(&["cockpit.*"]), "", "a switch whose `on` lights every light"),
        ],
        outputs: &[("pressed", B), ("on_light", B), ("fault_light", B), ("failed", B)],
    },
    PartSpec {
        domain: "cockpit",
        kind: "selector",
        help: "A rotary selector with numbered positions from 0.",
        fields: &[opt("positions", Kind::Number, "3", "how many positions"), opt("initial", Kind::Number, "0", "position at start")],
        outputs: &[("position", N), ("failed", B)],
    },
    PartSpec {
        domain: "cockpit",
        kind: "annunciator",
        help: "A warning or caution light.",
        fields: &[req("when", WHEN, "when it lights"), opt("test", Kind::Ref(&["cockpit.*"]), "", "a switch whose `on` lights it")],
        outputs: &[("lit", B), ("failed", B)],
    },
    PartSpec {
        domain: "weight",
        kind: "station",
        help: "A mass at an arm: empty aircraft, crew, cargo, payload.",
        fields: &[
            req("mass", Kind::Quantity(Dim::MASS), "for example \"120 kg\""),
            req("arm", Kind::Quantity(Dim::LENGTH), "distance aft of the datum"),
            opt("lateral", Kind::Quantity(Dim::LENGTH), "0 m", "right of the centreline"),
        ],
        outputs: &[("mass", Type::Num(Dim::MASS)), ("arm", Type::Num(Dim::LENGTH))],
    },
    PartSpec {
        domain: "weight",
        kind: "balance",
        help: "Total mass and centre of gravity of the stations listed.",
        fields: &[
            req("stations", Kind::Refs(&["weight.station", "fuel.tank"]), "station and tank ids"),
            opt("mac_leading_edge", Kind::Quantity(Dim::LENGTH), "0 m", "arm of the mean aerodynamic chord's leading edge"),
            opt("mac", Kind::Quantity(Dim::LENGTH), "0 m", "mean aerodynamic chord length"),
        ],
        outputs: &[("mass", Type::Num(Dim::MASS)), ("cg", Type::Num(Dim::LENGTH)), ("cg_lateral", Type::Num(Dim::LENGTH)), ("cg_mac", N)],
    },
    PartSpec {
        domain: "input",
        kind: "number",
        help: "A number set from outside: a binding, the API or Studio.",
        fields: &[opt("initial", Kind::Number, "0", "value at start")],
        outputs: &[("value", N)],
    },
    PartSpec {
        domain: "logic",
        kind: "flag",
        help: "A named true/false from an expression.",
        fields: &[req("when", WHEN, "for example \"delay(dc1.voltage < 25 V, 3 s)\"")],
        outputs: &[("on", B)],
    },
    PartSpec {
        domain: "electrical",
        kind: "bus",
        help: "A busbar.",
        fields: &[req("nominal", Kind::Quantity(Dim::VOLTAGE), "for example \"28 V\""), opt("powered_at", Kind::Number, "0.8", "share of nominal counted as powered")],
        outputs: &[("voltage", V), ("powered", B), ("failed", B)],
    },
    PartSpec {
        domain: "electrical",
        kind: "battery",
        help: "A battery on a bus.",
        fields: &[
            req("bus", Kind::Ref(BUS), "the bus it feeds"),
            req("voltage", Kind::Quantity(Dim::VOLTAGE), "nominal voltage, for example \"24 V\""),
            req("capacity", Kind::Quantity(Dim::CHARGE), "for example \"44 Ah\""),
            opt("resistance", Kind::Quantity(Dim::RESISTANCE), "0.02 ohm", "internal resistance"),
            opt("charge", Kind::Number, "1.0", "state of charge at start, 0 to 1"),
            opt("charge_limit", Kind::Quantity(Dim::CURRENT), "40 A", "most current it takes when charging"),
        ],
        outputs: &[("voltage", V), ("current", A), ("charge", N), ("failed", B)],
    },
    PartSpec {
        domain: "electrical",
        kind: "generator",
        help: "An engine- or APU-driven generator.",
        fields: &[
            req("bus", Kind::Ref(BUS), "the bus at its output"),
            req("voltage", Kind::Quantity(Dim::VOLTAGE), "regulated voltage"),
            opt("frequency", Kind::Quantity(Dim::FREQUENCY), "0 Hz", "0 for DC"),
            req("rating", Kind::Quantity(Dim::POWER), "apparent power, for example \"90 kVA\""),
            opt("power_factor", Kind::Number, "1", "real power it delivers per VA of rating; 0.75 to 1 for aircraft AC generators"),
            opt("driven_when", WHEN, "true", "when its drive turns it fast enough"),
        ],
        outputs: &[("voltage", V), ("frequency", HZ), ("current", A), ("load", W), ("ready", B), ("failed", B)],
    },
    PartSpec {
        domain: "electrical",
        kind: "ext_power",
        help: "A ground power receptacle.",
        fields: &[
            req("bus", Kind::Ref(BUS), "the bus at its output"),
            req("voltage", Kind::Quantity(Dim::VOLTAGE), "supply voltage"),
            opt("frequency", Kind::Quantity(Dim::FREQUENCY), "0 Hz", "0 for DC"),
            req("rating", Kind::Quantity(Dim::POWER), "supply rating, apparent power"),
            opt("power_factor", Kind::Number, "1", "real power it delivers per VA of rating"),
            opt("connected_when", WHEN, "false", "when the ground unit is plugged in"),
        ],
        outputs: &[("voltage", V), ("current", A), ("load", W), ("available", B), ("failed", B)],
    },
    PartSpec {
        domain: "electrical",
        kind: "contactor",
        help: "A contactor or relay between two buses.",
        fields: &[req("between", Kind::Refs(BUS), "two bus ids"), opt("close_when", WHEN, "true", "when it closes")],
        outputs: &[("closed", B), ("current", A), ("failed", B)],
    },
    PartSpec {
        domain: "electrical",
        kind: "breaker",
        help: "A circuit breaker between two buses.",
        fields: &[req("between", Kind::Refs(BUS), "two bus ids"), req("rating", Kind::Quantity(Dim::CURRENT), "for example \"10 A\"")],
        outputs: &[("closed", B), ("tripped", B), ("pulled", B), ("current", A), ("failed", B)],
    },
    PartSpec {
        domain: "electrical",
        kind: "load",
        help: "A consumer drawing constant power, dropping out below its minimum voltage.",
        fields: &[
            req("bus", Kind::Ref(BUS), "the bus it draws from"),
            req("power", Kind::Expr(W), "for example \"250 W\", or an expression such as \"heater.value * 1 kW\""),
            opt("on_when", WHEN, "true", "when it is switched on"),
            opt("shed_priority", Kind::Number, "0", "0 never shed; higher numbers are shed first"),
            opt("min_voltage", Kind::Quantity(Dim::VOLTAGE), "0 V", "below this it switches itself off, back on 0.5 s after the bus is 10 % above it; 0 V means half the bus nominal"),
        ],
        outputs: &[("powered", B), ("current", A), ("power", W), ("shed", B), ("failed", B)],
    },
    PartSpec {
        domain: "electrical",
        kind: "tru",
        help: "A transformer-rectifier unit, AC bus to DC bus.",
        fields: &[
            req("input", Kind::Ref(BUS), "AC bus"),
            req("output", Kind::Ref(BUS), "DC bus"),
            req("voltage", Kind::Quantity(Dim::VOLTAGE), "DC output voltage"),
            req("rating", Kind::Quantity(Dim::POWER), "output rating"),
            opt("efficiency", Kind::Number, "0.9", "0 to 1"),
        ],
        outputs: &[("current", A), ("power", W), ("active", B), ("failed", B)],
    },
    PartSpec {
        domain: "electrical",
        kind: "inverter",
        help: "A static inverter, DC bus to AC bus.",
        fields: &[
            req("input", Kind::Ref(BUS), "DC bus"),
            req("output", Kind::Ref(BUS), "AC bus"),
            req("voltage", Kind::Quantity(Dim::VOLTAGE), "AC output voltage"),
            opt("frequency", Kind::Quantity(Dim::FREQUENCY), "400 Hz", "output frequency"),
            req("rating", Kind::Quantity(Dim::POWER), "output rating"),
            opt("efficiency", Kind::Number, "0.85", "0 to 1"),
            opt("on_when", WHEN, "true", "when it runs"),
        ],
        outputs: &[("current", A), ("power", W), ("active", B), ("failed", B)],
    },
];

pub fn all() -> &'static [PartSpec] {
    static ALL: std::sync::OnceLock<Vec<PartSpec>> = std::sync::OnceLock::new();
    ALL.get_or_init(|| [CORE, crate::hydraulic::SPECS, crate::fuel::SPECS, crate::pneumatic::SPECS, crate::systems::AIR, crate::gear::SPECS, crate::cabin::SPECS, crate::ice::SPECS, crate::oxygen::SPECS, crate::engine::SPECS, crate::control::SPECS, crate::apu::SPECS, crate::extras::SPECS, crate::avionics::SPECS].concat())
}

pub struct Builtin;

impl Catalog for Builtin {
    fn specs(&self) -> &[PartSpec] {
        all()
    }
}

/// The built-in kinds plus an aircraft's own.
pub struct With(pub Vec<PartSpec>);

impl With {
    pub fn new(extra: &[PartSpec]) -> Self {
        With(all().iter().chain(extra).copied().collect())
    }
}

impl Catalog for With {
    fn specs(&self) -> &[PartSpec] {
        &self.0
    }
}

fn registry() -> &'static std::sync::RwLock<Vec<PartSpec>> {
    static R: std::sync::OnceLock<std::sync::RwLock<Vec<PartSpec>>> = std::sync::OnceLock::new();
    R.get_or_init(Default::default)
}

/// Remembers custom kinds so tools can look them up; the newest declaration of a name wins.
pub fn register(specs: &[PartSpec]) {
    let mut r = registry().write().unwrap_or_else(|e| e.into_inner());
    for s in specs {
        r.retain(|x| !(x.domain == s.domain && x.kind == s.kind));
        r.push(*s);
    }
}

/// A built-in or registered kind.
pub fn find(domain: &str, kind: &str) -> Option<PartSpec> {
    all().iter().find(|s| s.domain == domain && s.kind == kind).copied().or_else(|| registry().read().ok()?.iter().find(|s| s.domain == domain && s.kind == kind).copied())
}

/// Every built-in and registered kind.
pub fn list() -> Vec<PartSpec> {
    let mut v = all().to_vec();
    if let Ok(r) = registry().read() {
        v.extend(r.iter().copied());
    }
    v
}

/// Slot of output `name` of part `p`.
pub fn out(p: &easyacft_lang::CheckedPart, name: &str) -> usize {
    let spec = all().iter().find(|s| s.domain == p.domain && s.kind == p.kind).expect("checked part has a spec");
    p.slot + spec.outputs.iter().position(|(n, _)| *n == name).unwrap_or_else(|| panic!("{}.{} has no output {name}", p.domain, p.kind))
}
