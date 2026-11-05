//! A fixed-displacement hydraulic motor behind its directional control valve, with a pressure-off brake that clamps
//! it whenever its valve is not pressurised, and the speed-summing differential gearbox two motors drive a flap,
//! slat or stabiliser through: lose one motor and the output runs at half speed.

use super::fluid::{area_for, Fluid, P_ATM};
use super::ode::soft_sign;
use super::valve::{Oil, CD};
use crate::util::Slots;
use crate::{expr_of, Expr};
use easyacft_core::Dim;
use easyacft_lang::schema::{opt, Kind, PartSpec};
use easyacft_lang::{Checked, CheckedPart, Type};

const B: Type = Type::Bool;
const N: Type = Type::Num(Dim::NONE);
const WHEN: Kind = Kind::Expr(Type::Bool);
const PSI: f64 = 6_894.757;
const TAU: f64 = std::f64::consts::TAU;

pub static SPEC: PartSpec = PartSpec {
    domain: "actuator",
    kind: "motor",
    help: "A hydraulic motor with its directional control valve and pressure-off brake, for a flap or slat power control unit or a stabiliser trim actuator.",
    fields: &[
        opt("circuit", Kind::Ref(super::linear::CIRCUIT), "", "hydraulic circuit that powers it"),
        opt("displacement", Kind::Quantity(Dim::VOLUME), "6 mL", "volume per revolution"),
        opt("rated_speed", Kind::Quantity(Dim::FREQUENCY), "4000 rpm", "speed at full valve opening, no load, nominal pressure"),
        opt("enabled_when", WHEN, "true", "its valve's solenoids have power"),
        opt("brake_torque", Kind::Quantity(Dim::TORQUE), "0 N*m", "pressure-off brake holding torque; 0 sizes it at 1.5 times stall torque"),
        opt("brake_release", Kind::Quantity(Dim::PRESSURE), "800 psi", "valve pressure that releases the brake"),
        opt("valve_time", Kind::Quantity(Dim::TIME), "0.15 s", "valve travel time from closed to fully open"),
        opt("efficiency", Kind::Number, "0.9", "mechanical efficiency"),
        opt("volumetric_efficiency", Kind::Number, "0.95", "share of the flow that turns it"),
        opt("jam_when", WHEN, "false", "motor seized"),
        opt("brake_failed_when", WHEN, "false", "brake stays released"),
        opt("valve_failed_when", WHEN, "false", "valve stuck closed"),
    ],
    outputs: &[
        ("speed", Type::Num(Dim::FREQUENCY)),
        ("torque", Type::Num(Dim::TORQUE)),
        ("valve", N),
        ("braked", B),
        ("circuit_flow", Type::Num(Dim::VOLUME_FLOW)),
        ("circuit_held", Type::Num(Dim::VOLUME)),
        ("failed", B),
    ],
};

#[derive(Clone, Debug)]
pub struct Motor {
    /// Displacement per radian.
    pub disp: f64,
    pub w_rated: f64,
    pub area: f64,
    pub leak: f64,
    pub brake: f64,
    pub release: f64,
    pub valve_time: f64,
    pub eff: f64,
    pub vol_eff: f64,
    pub inertia: f64,
    pub nominal: f64,
    pub fluid: Fluid,
}

/// Inputs held over one mechanism step.
#[derive(Clone, Copy, Debug)]
pub struct MotorIo {
    pub ps: f64,
    pub pr: f64,
    pub oil: Oil,
    /// Valve position, -1 to 1.
    pub valve: f64,
    pub braked: bool,
    pub jammed: bool,
}

impl Motor {
    pub fn from_part(p: &CheckedPart, nominal: f64) -> Self {
        let nominal = if nominal > 1e5 { nominal } else { 3000.0 * PSI };
        let fluid = Fluid::new(super::fluid::Kind::PhosphateEster);
        let rho = fluid.density(313.15, 0.0);
        let disp = p.num("displacement").max(1e-7) / TAU;
        let w_rated = p.num("rated_speed").max(1.0) * TAU;
        let vol_eff = p.num("volumetric_efficiency").clamp(0.5, 1.0);
        let q = disp * w_rated / vol_eff;
        // Valve sized so the supply drops across its two metering edges at rated flow with no load.
        let area = area_for(CD, q, nominal / 2.0, rho);
        let eff = p.num("efficiency").clamp(0.3, 1.0);
        let stall = eff * disp * nominal;
        let given = p.num("brake_torque");
        Motor {
            disp,
            w_rated,
            area,
            leak: area_for(CD, 0.05e-3 / 60.0, nominal, rho),
            brake: if given > 0.0 { given } else { 1.5 * stall },
            release: p.num("brake_release"),
            valve_time: p.num("valve_time").max(0.01),
            eff,
            vol_eff,
            inertia: 2e-4 * (disp * 1e6 * TAU / 6.0).max(0.2),
            nominal,
            fluid,
        }
    }

    pub fn stall_torque(&self) -> f64 {
        self.eff * self.disp * self.nominal
    }

    /// Motor torque at speed `w` and the flow it draws.
    pub fn torque(&self, io: &MotorIo, w: f64) -> (f64, f64) {
        if io.jammed {
            return (-w * self.brake * 50.0 / self.w_rated.max(1.0), 0.0);
        }
        let open = io.valve.abs().min(1.0);
        let a = self.area * open + self.leak;
        let q = self.disp * w / self.vol_eff;
        // Two metering edges in series: each drops rho/2 (q / Cd a)^2.
        let edge = io.oil.rho * 0.5 * (q / (CD * a)).powi(2) * soft_sign(q, 1e-7);
        let dp_supply = (io.ps - io.pr) * io.valve.signum() * (open / 0.05).min(1.0);
        let dp = dp_supply - 2.0 * edge;
        let mut t = self.eff * self.disp * dp - 0.01 * self.stall_torque() * soft_sign(w, 2.0);
        if io.braked {
            t -= self.brake * soft_sign(w, 0.5);
        }
        let q_supply = if open > 0.02 { q.abs() + io.oil.q(self.leak, io.ps - io.pr) } else { io.oil.q(self.leak, io.ps - io.pr) };
        (t, q_supply)
    }
}

/// A motor part: configuration, expressions and its valve state.
pub struct Part {
    pub motor: Motor,
    pub circuit: Option<(usize, usize, usize)>,
    pub e: [Option<Expr>; 4],
    pub valve: f64,
    pub s: Slots,
    pub flow: f64,
}

impl Part {
    pub fn new(c: &Checked, p: &CheckedPart) -> Self {
        let circuit = p.part("circuit").and_then(|i| {
            let id = &c.parts[i].id;
            let g = |o: &str| c.slots.index.get(&format!("{id}.{o}")).copied();
            Some((g("pressure")?, g("temperature")?, g("return_pressure")?))
        });
        let nominal = p.part("circuit").map_or(0.0, |i| c.parts[i].num("nominal"));
        Part {
            motor: Motor::from_part(p, nominal),
            circuit,
            e: ["enabled_when", "jam_when", "brake_failed_when", "valve_failed_when"].map(|k| expr_of(p, k)),
            valve: 0.0,
            s: Slots::of(p, &["speed", "torque", "valve", "braked", "circuit_flow", "circuit_held", "failed"]),
            flow: 0.0,
        }
    }

    /// Moves the valve toward `cmd` and builds this step's inputs.
    pub fn prepare(&mut self, slots: &[f64], dt: f64, cmd: f64) -> MotorIo {
        let mut y = |k: usize, or: bool| self.e[k].as_mut().map_or(or, |x| x.yes(slots, dt));
        let enabled = y(0, true);
        let jammed = y(1, false);
        let brake_failed = y(2, false);
        let valve_failed = y(3, false);
        let failed = self.s.on(slots, "failed");
        let (ps, pr, t) = match self.circuit {
            Some((p, t, r)) => {
                let pr = slots[r].max(P_ATM);
                (slots[p].max(0.0) + pr, pr, slots[t])
            }
            None => (P_ATM, P_ATM, 288.15),
        };
        let want = if enabled && !valve_failed && !failed { cmd.clamp(-1.0, 1.0) } else { 0.0 };
        let rate = dt / self.motor.valve_time;
        self.valve += (want - self.valve).clamp(-rate, rate);
        let valve_p = (ps - pr) * (self.valve.abs() / 0.1).min(1.0);
        let braked = !brake_failed && valve_p < self.motor.release;
        let m = &self.motor;
        let rho = m.fluid.density(t, ps);
        let mu = m.fluid.dynamic(t, ps);
        MotorIo { ps, pr, oil: Oil { rho, nu: mu / rho }, valve: self.valve, braked, jammed }
    }

    pub fn finish(&mut self, slots: &mut [f64], io: &MotorIo, w: f64, t: f64, q: f64) {
        self.flow = q;
        let s = &self.s;
        s.put(slots, "speed", w / TAU);
        s.put(slots, "torque", t);
        s.put(slots, "valve", io.valve);
        s.flag(slots, "braked", io.braked);
        s.put(slots, "circuit_flow", if self.circuit.is_some() { q } else { 0.0 });
        s.put(slots, "circuit_held", 0.0);
    }
}

/// Two inputs into a speed-summing differential driving an output of inertia `out_inertia` through `ratio`
/// (output rad per input rad, averaged). States: output angle, motor 1 speed, motor 2 speed.
/// Returns (output acceleration, motor 1 acceleration, motor 2 acceleration).
pub fn differential(t1: f64, t2: f64, j: f64, ratio: f64, out_inertia: f64, out_load: f64) -> (f64, f64, f64) {
    // Reaction torque the differential puts back on each input, from both motors' equations and the output's.
    let r = ratio;
    let tau = (out_inertia * r * r * (t1 + t2) / (2.0 * j) + r * out_load) / (2.0 + out_inertia * r * r / j);
    let a1 = (t1 - tau) / j;
    let a2 = (t2 - tau) / j;
    (r * (a1 + a2) / 2.0, a1, a2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_braked_motor_halves_the_output_speed() {
        let j = 1e-3;
        let mut w = [0.0, 0.0];
        let mut wb = [0.0, 0.0];
        for _ in 0..20000 {
            let t = |w: f64| 10.0 - 0.01 * w;
            let (_, a1, a2) = differential(t(w[0]), t(w[1]), j, 0.01, 50.0, 0.0);
            w[0] += a1 * 1e-4;
            w[1] += a2 * 1e-4;
            let braked = -0.2 * wb[1];
            let (_, b1, b2) = differential(t(wb[0]), braked, j, 0.01, 50.0, 0.0);
            wb[0] += b1 * 1e-4;
            wb[1] += b2 * 1e-4;
        }
        let both = (w[0] + w[1]) / 2.0;
        let one = (wb[0] + wb[1]) / 2.0;
        assert!(both > 0.0 && (one / both - 0.5).abs() < 0.1, "both {both}, one {one}");
    }
}
