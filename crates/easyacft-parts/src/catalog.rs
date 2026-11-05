//! Built-in part kinds.

use easyacft_core::Dim;
use easyacft_lang::schema::{opt, req, Catalog, Kind, PartSpec};
use easyacft_lang::Type;

const B: Type = Type::Bool;
const N: Type = Type::Num(Dim::NONE);
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
];

pub fn all() -> &'static [PartSpec] {
    static ALL: std::sync::OnceLock<Vec<PartSpec>> = std::sync::OnceLock::new();
    ALL.get_or_init(|| [CORE, crate::electrical::SPECS, crate::hydraulic::SPECS, crate::fuel::SPECS, crate::pneumatic::SPECS, crate::environment::SPECS, crate::gear::SPECS, crate::cabin::SPECS, crate::aircon::SPECS, crate::actuation::SPECS, crate::ice::SPECS, crate::oxygen::SPECS, crate::engine::SPECS, crate::control::SPECS, crate::apu::SPECS, crate::extras::SPECS, crate::avionics::SPECS].concat())
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
