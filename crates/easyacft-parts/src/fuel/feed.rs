//! Engine and APU fuel inlets. The engine's own pump takes the fuel it asks for while boost pressure holds the
//! inlet up; without boost it can suck it up a gravity path only while the ullage and head stay above the fuel's
//! vapour pressure plus what the pump needs, which runs out with altitude and hot fuel. On its way to the engine
//! the fuel picks up oil heat, and its filter collects ice from the water in cold fuel until it bypasses.

use super::{eval, flag, put, yes, B, MF, N, P, T, W, WHEN};
use crate::hydraulic::net::Net;
use crate::{expr_of, Expr};
use easyacft_core::Dim;
use easyacft_lang::schema::{opt, req, Kind, PartSpec};
use easyacft_lang::CheckedPart;

pub const SPEC: PartSpec = PartSpec {
    domain: "fuel",
    kind: "feed",
    help: "An engine or APU fuel inlet: boost-fed, or suction-fed up a gravity path while the ullage and head stay above vapour pressure; heated by the engine's oil cooler; its filter ices from the water in cold fuel and bypasses.",
    fields: &[
        req("line", Kind::Ref(&["fuel.line"]), "the line it draws from"),
        req("demand", Kind::Expr(MF), "fuel it asks for, for example \"eng1.fuel_flow\""),
        opt("minimum", Kind::Quantity(Dim::PRESSURE), "1 psi", "below this the inlet starves"),
        opt("suction_when", WHEN, "false", "the engine pump can suck fuel with the boost pumps off, as at low altitude"),
        opt("ambient", Kind::Expr(P), "101325 Pa", "outside static pressure, which sets the suction ceiling"),
        opt("npsh", Kind::Quantity(Dim::PRESSURE), "5 psi", "inlet pressure above vapour pressure the engine pump needs to suck"),
        opt("oil_heat", Kind::Expr(W), "0 W", "engine oil heat its fuel-oil heat exchanger puts into the fuel"),
        opt("filter_dp", Kind::Quantity(Dim::PRESSURE), "3 psi", "clean filter drop at full demand"),
        opt("filter_bypass", Kind::Quantity(Dim::PRESSURE), "25 psi", "filter drop that opens its bypass"),
    ],
    outputs: OUT,
};

outputs!(OUT, fo { flow: MF, starved: B, temperature: T, failed: B, inlet_pressure: P, suction: B, filter_dp: P, filter_clogged: B, filter_bypass: B, ice: N });

/// Suction an engine-driven pump pulls on its line when it feeds without boost pressure, Pa below ambient.
const SUCTION: f64 = 30_000.0;

pub struct Feed {
    base: usize,
    pub line: usize,
    pub node: usize,
    demand: Expr,
    minimum: f64,
    suction_e: Option<Expr>,
    ambient_e: Option<Expr>,
    oil_e: Option<Expr>,
    npsh: f64,
    dp0: f64,
    bypass_at: f64,
    pub want: f64,
    sucking: bool,
    boosted: bool,
    can_suck: bool,
    g: f64,
    ice: f64,
    amb: f64,
    heat: f64,
    sum_q: f64,
    n_sub: f64,
    pub drawn: f64,
    starved: bool,
}

impl Feed {
    pub fn new(p: &CheckedPart, line: usize, node: usize, demand: Expr) -> Self {
        Feed {
            base: p.slot,
            line,
            node,
            demand,
            minimum: p.num("minimum"),
            suction_e: expr_of(p, "suction_when"),
            ambient_e: expr_of(p, "ambient"),
            oil_e: expr_of(p, "oil_heat"),
            npsh: p.num("npsh").max(0.0),
            dp0: p.num("filter_dp").max(0.0),
            bypass_at: p.num("filter_bypass").max(1.0),
            want: 0.0,
            sucking: false,
            boosted: false,
            can_suck: false,
            g: 0.0,
            ice: 0.0,
            amb: 101_325.0,
            heat: 0.0,
            sum_q: 0.0,
            n_sub: 0.0,
            drawn: 0.0,
            starved: false,
        }
    }

    /// `p_line` is last step's line pressure, `vapour` the fuel's vapour pressure, `rho` its density.
    pub fn begin(&mut self, slots: &[f64], dt: f64, p_line: f64, vapour: f64, rho: f64) {
        let p_line = p_line.max(0.0);
        let failed = slots[self.base + fo::failed] != 0.0;
        self.want = if failed { 0.0 } else { self.demand.eval(slots, dt).max(0.0) / rho };
        self.amb = eval(&mut self.ambient_e, slots, dt, 101_325.0).max(0.0);
        self.heat = eval(&mut self.oil_e, slots, dt, 0.0).max(0.0);
        let permitted = yes(&mut self.suction_e, slots, dt, false);
        // Suction feed needs the absolute inlet pressure it can pull down to, above vapour pressure plus margin.
        self.can_suck = permitted && self.amb - SUCTION > vapour + self.npsh;
        // Once sucking it keeps on until boost pressure is plainly back.
        let low = p_line < if self.sucking { 3.0 * self.minimum } else { self.minimum };
        self.sucking = self.want > 0.0 && low && self.can_suck;
        self.boosted = !self.sucking && p_line >= self.minimum;
        self.sum_q = 0.0;
        self.n_sub = 0.0;
    }

    /// The engine pump takes what it asks for: a conductance re-aimed each substep at last substep's pressure,
    /// so the flow the network removes is the flow counted as burned.
    pub fn stamp(&mut self, net: &mut Net) {
        let p = net.p[self.node];
        if self.sucking {
            self.g = self.want / (p + SUCTION).max(0.1 * SUCTION);
            net.stamp(self.node, self.g, -self.g * SUCTION);
        } else if self.want > 0.0 && self.boosted {
            // With boost pressure up the engine pump takes exactly what it asks for.
            self.g = 0.0;
            net.stamp(self.node, 0.0, -self.want);
        } else {
            self.g = 0.0;
        }
    }

    pub fn after(&mut self, net: &Net) -> f64 {
        let p = net.p[self.node];
        let q = if self.sucking {
            (self.g * (p + SUCTION)).max(0.0)
        } else if self.boosted {
            self.want
        } else {
            0.0
        };
        self.sum_q += q;
        self.n_sub += 1.0;
        q
    }

    /// Outputs; returns the mass flow the engine actually burns.
    #[allow(clippy::too_many_arguments)]
    pub fn finish(&mut self, net: &Net, slots: &mut [f64], dt: f64, rho: f64, temp: f64, water: f64, cp: f64) -> f64 {
        let p = net.p[self.node];
        let q = self.sum_q / self.n_sub.max(1.0);
        self.starved = if self.sucking { q < 0.8 * self.want } else { p < self.minimum };
        let m = q * rho;
        self.drawn = m;
        // Ice: water in fuel below freezing collects on the filter; warm fuel or oil heat melts it.
        let t_out = temp + if m > 1e-4 { self.heat / (m * cp) } else { 0.0 };
        let freezing = temp < 273.15 && t_out < 273.15;
        self.ice = if freezing { self.ice + water * m * dt * 4.0 } else { (self.ice - dt / 120.0).max(0.0) };
        let share = (self.want / self.want.max(1e-9)).min(1.0);
        let dp = self.dp0 * share * (1.0 + 400.0 * self.ice);
        let b = self.base;
        flag(slots, b + fo::starved, self.starved);
        put(slots, b + fo::flow, m);
        put(slots, b + fo::temperature, t_out.min(420.0));
        put(slots, b + fo::inlet_pressure, p);
        flag(slots, b + fo::suction, self.sucking);
        put(slots, b + fo::filter_dp, dp.min(self.bypass_at));
        flag(slots, b + fo::filter_clogged, dp > 0.6 * self.bypass_at);
        flag(slots, b + fo::filter_bypass, dp >= self.bypass_at);
        put(slots, b + fo::ice, self.ice);
        m
    }

    pub fn state(&self, v: &mut Vec<f64>) {
        v.push(self.ice);
    }

    pub fn restore(&mut self, it: &mut dyn Iterator<Item = f64>) {
        self.ice = it.next().unwrap_or(0.0);
    }
}
