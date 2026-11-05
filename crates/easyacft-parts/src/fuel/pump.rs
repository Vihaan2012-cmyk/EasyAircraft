//! Boost and transfer pumps: centrifugal, so pressure falls off with flow along a head curve that follows speed
//! squared; they need inlet pressure above the fuel's vapour pressure or they cavitate, which bites with hot fuel
//! or a low ullage at altitude; cold thick fuel costs head; an uncovered inlet runs them dry until their motor
//! overheats and its thermal cutout opens; free water and ice at the sump go where they go.

use super::tank::Tank;
use super::{eval, flag, put, yes, B, MF, N, P, T, W, WHEN};
use crate::hydraulic::net::Net;
use crate::{expr_of, Expr};
use easyacft_core::Dim;
use easyacft_lang::schema::{opt, req, Kind, PartSpec};
use easyacft_lang::CheckedPart;

pub const SPEC: PartSpec = PartSpec {
    domain: "fuel",
    kind: "pump",
    help: "A centrifugal boost or transfer pump from a tank into a line or another tank: a head curve with speed, cavitation when its inlet nears the fuel's vapour pressure, head lost to cold fuel, dry running that overheats it into its thermal cutout, and the free water it pulls from the sump.",
    fields: &[
        req("tank", Kind::Ref(&["fuel.tank"]), "the tank it draws from"),
        req("outlet", Kind::Ref(&["fuel.tank", "fuel.line"]), "the line or tank it pushes into"),
        opt("pressure", Kind::Quantity(Dim::PRESSURE), "25 psi", "outlet pressure at no flow"),
        req("flow", Kind::Expr(MF), "flow at full output, for example \"1.5 kg/s\", or an expression"),
        opt("running_when", WHEN, "true", "when it runs"),
        opt("efficiency", Kind::Number, "0.3", "hydraulic power out per electrical power in"),
        opt("inlet_uncovered_when", WHEN, "false", "its inlet is out of the fuel, as in a steep attitude"),
        opt("speed", Kind::Expr(N), "1", "motor speed as a share of rated, as with a variable-frequency supply"),
        opt("npsh", Kind::Quantity(Dim::PRESSURE), "1.5 psi", "inlet pressure it needs above vapour pressure at full flow"),
        opt("dry_limit", Kind::Quantity(Dim::TIME), "120 s", "how long it runs dry before its thermal cutout opens"),
        opt("impeller_damaged_when", WHEN, "false", "a damaged impeller: much less head"),
    ],
    outputs: OUT,
};

outputs!(OUT, po { running: B, flow: MF, power: W, low_pressure: B, failed: B, pressure: P, cavitating: B, dry: B, overheat: B, motor_temperature: T, speed: N });

const G0: f64 = 9.80665;
/// Rated flow sits at this share of shutoff head; the curve is a parabola through both.
const RATED_HEAD: f64 = 0.7;
const COOL_S: f64 = 300.0;

pub struct Pump {
    base: usize,
    pub tank: usize,
    pub outlet: usize,
    flow_e: Option<Expr>,
    speed_e: Option<Expr>,
    when: Option<Expr>,
    uncovered_e: Option<Expr>,
    damaged_e: Option<Expr>,
    shutoff: f64,
    eff: f64,
    npsh: f64,
    dry_limit: f64,
    rated: f64,
    speed: f64,
    pub on: bool,
    damaged: bool,
    dry: f64,
    cutout: bool,
    motor_t: f64,
    cav: f64,
    /// Volume flow this substep and the linearisation it was stamped with.
    pub q: f64,
    g: f64,
    e: f64,
    sum_m: f64,
    n_sub: f64,
    last_p: f64,
}

impl Pump {
    pub fn new(p: &CheckedPart, tank: usize, outlet: usize) -> Self {
        Pump {
            base: p.slot,
            tank,
            outlet,
            flow_e: expr_of(p, "flow"),
            speed_e: expr_of(p, "speed"),
            when: expr_of(p, "running_when"),
            uncovered_e: expr_of(p, "inlet_uncovered_when"),
            damaged_e: expr_of(p, "impeller_damaged_when"),
            shutoff: p.num("pressure").max(1.0),
            eff: p.num("efficiency").clamp(0.01, 1.0),
            npsh: p.num("npsh").max(0.0),
            dry_limit: p.num("dry_limit").max(1.0),
            rated: 0.0,
            speed: 1.0,
            on: false,
            damaged: false,
            dry: 0.0,
            cutout: false,
            motor_t: f64::NAN,
            cav: 1.0,
            q: 0.0,
            g: 0.0,
            e: 0.0,
            sum_m: 0.0,
            n_sub: 0.0,
            last_p: 0.0,
        }
    }

    pub fn begin(&mut self, slots: &mut [f64], dt: f64, t: &Tank) {
        let failed = slots[self.base + po::failed] != 0.0;
        self.rated = eval(&mut self.flow_e, slots, dt, 0.0).max(0.0);
        self.speed = eval(&mut self.speed_e, slots, dt, 1.0).max(0.0);
        self.damaged = yes(&mut self.damaged_e, slots, dt, false);
        let commanded = !failed && yes(&mut self.when, slots, dt, true);
        let uncovered = t.uncovered() || yes(&mut self.uncovered_e, slots, dt, false);
        if self.motor_t.is_nan() {
            self.motor_t = t.temp;
        }
        // Running dry there is nothing to carry the motor's heat away.
        if commanded && !self.cutout && uncovered {
            self.dry += dt;
        } else {
            self.dry = (self.dry - dt * 0.2).max(0.0);
        }
        let hot = self.dry > self.dry_limit;
        self.cutout = if self.cutout { self.dry > 0.5 * self.dry_limit } else { hot };
        let target = t.temp + if self.on { 25.0 } else { 0.0 } + 120.0 * (self.dry / self.dry_limit).min(1.5);
        self.motor_t += (target - self.motor_t) * (dt / COOL_S).min(1.0) * if self.dry > 0.0 { 6.0 } else { 1.0 };
        self.on = commanded && !self.cutout && !uncovered && t.quantity > t.unusable;
        // Suction: ullage and head over vapour pressure, against what the pump needs at its flow.
        let share = (self.q * t.density() / self.rated.max(1e-6)).clamp(0.0, 2.0);
        let p_in = t.ullage + t.density() * G0 * t.inlet_depth;
        let margin = p_in - t.fuel.vapour(t.temp);
        let need = self.npsh * share * share + 0.2 * self.npsh;
        self.cav = if margin <= 0.0 { 0.0 } else { (margin / need.max(1.0)).clamp(0.0, 1.0) };
        self.sum_m = 0.0;
        self.n_sub = 0.0;
    }

    /// Shutoff head and rated volume flow at this speed, after cold fuel, cavitation and impeller damage.
    fn curve(&self, t: &Tank) -> (f64, f64) {
        let rho = t.density();
        let nu = t.fuel.kinematic(t.temp);
        let thick = (1.0 + 0.04 * (nu / 1.5e-6 - 1.0).max(0.0)).min(2.0);
        let damage = if self.damaged { 0.35 } else { 1.0 };
        // Below the cloud point wax gels the fuel at the inlet screen and the pump moves less and less.
        let wax = ((t.temp - t.fuel.freeze) / 4.0).clamp(0.0, 1.0);
        let s = self.speed;
        let p0 = self.shutoff * s * s * self.cav * damage * wax / thick;
        let q_r = self.rated / rho * s * wax;
        (p0, q_r)
    }

    /// Stamps the pump at its outlet, linearised on its head curve at last substep's flow.
    pub fn stamp(&mut self, net: &mut Net, t: &Tank) {
        if !self.on {
            self.g = 0.0;
            self.e = 0.0;
            return;
        }
        let (p0, q_r) = self.curve(t);
        if p0 <= 1.0 || q_r <= 0.0 {
            self.g = 0.0;
            return;
        }
        // Head: p = p0 (1 - (1 - RATED_HEAD) (q / q_r)^2), shifted up by the tank head it starts from.
        let k = (1.0 - RATED_HEAD) * p0 / (q_r * q_r);
        let q0 = self.q.max(0.2 * q_r);
        let slope = 2.0 * k * q0;
        let p_at = p0 - k * q0 * q0;
        self.g = 1.0 / slope;
        self.e = p_at + q0 * slope + t.head;
        net.stamp(self.outlet, self.g, self.g * self.e);
    }

    pub fn after(&mut self, net: &Net, rho: f64) {
        self.last_p = net.p[self.outlet];
        self.q = if self.on { (self.g * (self.e - self.last_p)).max(0.0) } else { 0.0 };
        self.sum_m += self.q * rho;
        self.n_sub += 1.0;
    }

    /// Mass flow averaged over the step.
    pub fn mass_flow(&self) -> f64 {
        self.sum_m / self.n_sub.max(1.0)
    }

    pub fn finish(&self, slots: &mut [f64], t: &Tank) {
        let m = self.mass_flow();
        let rho = t.density();
        let b = self.base;
        let hydraulic = m / rho * self.last_p.max(0.0);
        // The motor turns its impeller whatever the flow; a dry pump still spins.
        let spin = 0.3 * self.rated / rho * self.shutoff * RATED_HEAD * self.speed.powi(3) / self.eff;
        flag(slots, b + po::running, self.on);
        put(slots, b + po::flow, m);
        put(slots, b + po::power, if self.on || self.dry > 0.0 { spin + hydraulic / self.eff } else { 0.0 });
        flag(slots, b + po::low_pressure, self.on && self.last_p < 0.5 * self.shutoff || (!self.on && self.dry > 0.0));
        put(slots, b + po::pressure, if self.on { self.last_p.max(0.0) } else { 0.0 });
        flag(slots, b + po::cavitating, self.on && self.cav < 0.9);
        flag(slots, b + po::dry, self.dry > 0.0);
        flag(slots, b + po::overheat, self.cutout);
        put(slots, b + po::motor_temperature, self.motor_t);
        put(slots, b + po::speed, self.speed);
    }

    pub fn state(&self, v: &mut Vec<f64>) {
        v.extend([self.q, self.dry, self.cutout as u8 as f64, self.motor_t, self.on as u8 as f64]);
    }

    pub fn restore(&mut self, it: &mut dyn Iterator<Item = f64>) {
        let mut n = || it.next().unwrap_or(0.0);
        self.q = n();
        self.dry = n();
        self.cutout = n() != 0.0;
        self.motor_t = n();
        self.on = n() != 0.0;
    }
}
