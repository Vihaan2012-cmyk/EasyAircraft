//! A valve on a circuit's line to some of its pumps and users: a fire shutoff valve on an engine pump's suction,
//! a priority valve ahead of the users that can wait, an isolation valve. Shut, the pumps behind it starve and
//! the users behind it stop; a motorised valve travels, and a failed one stays where it is.

use super::{flag, put, yes, B, N, WHEN};
use crate::Expr;
use easyacft_core::Dim;
use easyacft_lang::schema::{opt, req, Kind, PartSpec};
use easyacft_lang::CheckedPart;

pub const SPEC: PartSpec = PartSpec {
    domain: "hydraulic",
    kind: "valve",
    help: "A valve on a circuit's line to some of its pumps and users, such as a fire shutoff or priority valve: shut, its pumps starve and its users stop; it travels, needs power, and a failed one stays where it is.",
    fields: &[
        req("circuit", Kind::Ref(&["hydraulic.circuit"]), "the circuit it is on"),
        opt("serves", Kind::Refs(&["hydraulic.pump", "hydraulic.consumer"]), "", "the pumps and users behind it"),
        opt("open_when", WHEN, "true", "when it is commanded open"),
        opt("open_time", Kind::Quantity(Dim::TIME), "1 s", "travel time from shut to open"),
        opt("stuck_when", WHEN, "false", "it has failed where it is"),
        opt("powered_when", WHEN, "true", "its motor has power; without it, it stays where it is"),
    ],
    outputs: OUT,
};

outputs!(OUT, vo { open: B, position: N, stuck: B, failed: B });

pub struct Valve {
    base: usize,
    pub serves: Vec<usize>,
    when: Option<Expr>,
    stuck_e: Option<Expr>,
    powered_e: Option<Expr>,
    travel: f64,
    pub position: f64,
}

impl Valve {
    pub fn new(p: &CheckedPart) -> Self {
        Valve {
            base: p.slot,
            serves: p.parts("serves"),
            when: crate::expr_of(p, "open_when"),
            stuck_e: crate::expr_of(p, "stuck_when"),
            powered_e: crate::expr_of(p, "powered_when"),
            travel: p.num("open_time").max(0.0),
            position: 1.0,
        }
    }

    /// Moves it toward its command and returns how far open it is, 0 to 1.
    pub fn step(&mut self, slots: &mut [f64], dt: f64) -> f64 {
        let failed = slots[self.base + vo::failed] != 0.0;
        let stuck = failed || yes(&mut self.stuck_e, slots, dt, false);
        let powered = yes(&mut self.powered_e, slots, dt, true);
        let target = if yes(&mut self.when, slots, dt, true) { 1.0 } else { 0.0 };
        if !stuck && powered {
            let rate = if self.travel > 0.0 { dt / self.travel } else { 1.0 };
            self.position += (target - self.position).clamp(-rate, rate);
        }
        let b = self.base;
        flag(slots, b + vo::open, self.position > 0.5);
        put(slots, b + vo::position, self.position);
        flag(slots, b + vo::stuck, stuck);
        self.position
    }
}
