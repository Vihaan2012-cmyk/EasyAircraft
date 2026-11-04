//! The air-conditioning system controller: pack outlet demand from the coldest zone, pack flow scheduling
//! (single pack, APU bleed, economy, engine start), trim air and hot air commands, and lane redundancy with a
//! fixed-temperature backup when both lanes are lost.

use super::thermo::{lag, Confirm};
use super::yes;
use crate::util::Slots;
use crate::Expr;
use easyacft_core::Dim;
use easyacft_lang::schema::{opt, req, Kind, PartSpec};
use easyacft_lang::{CheckedPart, Type};

const B: Type = Type::Bool;
const N: Type = Type::Num(Dim::NONE);
const T: Type = Type::Num(Dim::TEMPERATURE);
const WHEN: Kind = Kind::Expr(Type::Bool);

pub const SPEC: PartSpec = PartSpec {
    domain: "aircon",
    kind: "controller",
    help: "System controller: sets pack outlet demand and flow, inhibits packs during engine start and ditching, and commands hot air; two lanes with a backup mode.",
    fields: &[
        req("packs", Kind::Refs(&["aircon.pack"]), "packs it controls"),
        req("zones", Kind::Refs(&["aircon.zone"]), "zones it serves"),
        opt("hot_air", Kind::Refs(&["aircon.hot_air"]), "", "hot air manifolds it commands"),
        opt("engine_start_when", WHEN, "false", "an engine is starting: packs close to give the starter its air"),
        opt("apu_bleed_when", WHEN, "false", "on APU bleed: pack flow goes high"),
        opt("ditching_when", WHEN, "false", "ditching selected: packs and ram air close"),
        opt("economy_when", WHEN, "false", "economy flow with a light cabin"),
        opt("lane_a_failed_when", WHEN, "false", "lane A lost"),
        opt("lane_b_failed_when", WHEN, "false", "lane B lost"),
        opt("margin", Kind::Quantity(Dim::TEMPERATURE), "2 K", "how far below the coldest zone's duct demand the packs aim, so trim air has something to trim"),
        opt("start_delay", Kind::Quantity(Dim::TIME), "30 s", "how long packs stay closed after an engine start ends"),
    ],
    outputs: &[("pack_demand", T), ("flow_factor", N), ("inhibit", B), ("hot_air_command", B), ("operating_packs", N), ("lane", N), ("backup", B), ("mode", N), ("fault", B)],
};

pub mod mode {
    pub const NORMAL: f64 = 0.0;
    pub const SINGLE_PACK: f64 = 1.0;
    pub const APU: f64 = 2.0;
    pub const ECONOMY: f64 = 3.0;
    pub const ENGINE_START: f64 = 4.0;
    pub const DITCHING: f64 = 5.0;
    pub const BACKUP: f64 = 6.0;
}

pub struct Controller {
    pub packs: Vec<usize>,
    pub zones: Vec<usize>,
    pub hot_air: Vec<usize>,
    start: Option<Expr>,
    apu: Option<Expr>,
    ditch: Option<Expr>,
    economy: Option<Expr>,
    lane_a: Option<Expr>,
    lane_b: Option<Expr>,
    margin: f64,
    start_delay: f64,
    since_start: f64,
    demand: f64,
    factor: f64,
    single: Confirm,
    s: Slots,
}

impl Controller {
    pub fn new(p: &CheckedPart, packs: Vec<usize>, zones: Vec<usize>, hot_air: Vec<usize>) -> Self {
        Controller {
            packs,
            zones,
            hot_air,
            start: crate::expr_of(p, "engine_start_when"),
            apu: crate::expr_of(p, "apu_bleed_when"),
            ditch: crate::expr_of(p, "ditching_when"),
            economy: crate::expr_of(p, "economy_when"),
            lane_a: crate::expr_of(p, "lane_a_failed_when"),
            lane_b: crate::expr_of(p, "lane_b_failed_when"),
            margin: p.num("margin"),
            start_delay: p.num("start_delay"),
            since_start: f64::INFINITY,
            demand: 288.0,
            factor: 1.0,
            single: Confirm::default(),
            s: Slots::of(p, &["pack_demand", "flow_factor", "inhibit", "hot_air_command", "operating_packs", "lane", "backup", "mode", "fault"]),
        }
    }

    /// `zone_demands` are the zones' duct demands; `operating` how many of its packs deliver flow.
    pub fn step(&mut self, slots: &mut [f64], dt: f64, zone_demands: &[f64], operating: usize) {
        let lane_a = !yes(&mut self.lane_a, slots, dt, false);
        let lane_b = !yes(&mut self.lane_b, slots, dt, false);
        let backup = !lane_a && !lane_b;
        let starting = yes(&mut self.start, slots, dt, false);
        self.since_start = if starting { 0.0 } else { self.since_start + dt };
        let start_hold = starting || self.since_start < self.start_delay;
        let ditching = yes(&mut self.ditch, slots, dt, false);
        let apu = yes(&mut self.apu, slots, dt, false);
        let economy = yes(&mut self.economy, slots, dt, false);
        let single = self.single.hold(operating == 1 && self.packs.len() > 1, 3.0, dt);
        let coldest = zone_demands.iter().copied().fold(f64::INFINITY, f64::min);
        let target = if backup {
            288.0
        } else if coldest.is_finite() {
            (coldest - self.margin).clamp(275.0, 343.0)
        } else {
            288.0
        };
        self.demand = lag(self.demand, target, 10.0, dt);
        let (factor, m) = if ditching {
            (0.0, mode::DITCHING)
        } else if start_hold {
            (0.0, mode::ENGINE_START)
        } else if backup {
            (1.0, mode::BACKUP)
        } else if single {
            (1.2, mode::SINGLE_PACK)
        } else if apu {
            (1.2, mode::APU)
        } else if economy {
            (0.8, mode::ECONOMY)
        } else {
            (1.0, mode::NORMAL)
        };
        self.factor = lag(self.factor, factor, 3.0, dt);
        let inhibit = ditching || start_hold;
        let s = &self.s;
        s.put(slots, "pack_demand", self.demand);
        s.put(slots, "flow_factor", if inhibit { 0.0 } else { self.factor });
        s.flag(slots, "inhibit", inhibit);
        s.flag(slots, "hot_air_command", !backup && !ditching && operating > 0);
        s.put(slots, "operating_packs", operating as f64);
        s.put(slots, "lane", if lane_a { 1.0 } else if lane_b { 2.0 } else { 0.0 });
        s.flag(slots, "backup", backup);
        s.put(slots, "mode", m);
        s.flag(slots, "fault", !lane_a || !lane_b);
    }

    pub fn state(&self, v: &mut Vec<f64>) {
        v.extend([self.since_start.min(1e9), self.demand, self.factor, self.single.elapsed]);
    }

    pub fn restore(&mut self, it: &mut impl Iterator<Item = f64>) {
        let mut n = || it.next().unwrap_or(0.0);
        self.since_start = n();
        self.demand = n();
        self.factor = n();
        self.single.elapsed = n();
    }
}
