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
            req("rating", Kind::Quantity(Dim::POWER), "for example \"90 kVA\""),
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
            req("rating", Kind::Quantity(Dim::POWER), "supply rating"),
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
        help: "A consumer drawing constant power.",
        fields: &[
            req("bus", Kind::Ref(BUS), "the bus it draws from"),
            req("power", Kind::Quantity(Dim::POWER), "for example \"250 W\""),
            opt("on_when", WHEN, "true", "when it is switched on"),
            opt("shed_priority", Kind::Number, "0", "0 never shed; higher numbers are shed first"),
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
    ALL.get_or_init(|| [CORE, crate::hydraulic::SPECS, crate::fuel::SPECS, crate::pneumatic::SPECS, crate::systems::SPECS, crate::engine::SPECS, crate::control::SPECS].concat())
}

pub struct Builtin;

impl Catalog for Builtin {
    fn specs(&self) -> &[PartSpec] {
        all()
    }
}

/// Slot of output `name` of part `p`.
pub fn out(p: &easyacft_lang::CheckedPart, name: &str) -> usize {
    let spec = all().iter().find(|s| s.domain == p.domain && s.kind == p.kind).expect("checked part has a spec");
    p.slot + spec.outputs.iter().position(|(n, _)| *n == name).unwrap_or_else(|| panic!("{}.{} has no output {name}", p.domain, p.kind))
}
