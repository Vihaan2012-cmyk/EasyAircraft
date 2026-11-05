//! Valves of the actuators: the two-stage electro-hydraulic servo valve with its spool dynamics, null leakage,
//! lap and failure modes; the selector (directional) valve with closed or floating centre; the solenoid mode valve;
//! chamber relief and anti-cavitation check valves; the pressure-control valve brakes use; and a shuttle valve.
//! Flows are smooth in pressure and spool position so the implicit stepper can differentiate them.

use super::fluid::{area_for, orifice};
use super::ode::{smooth_step, soft_pos};

/// Discharge coefficient of a sharp metering edge.
pub const CD: f64 = 0.65;
const PSI: f64 = 6_894.757;

/// Fluid state a valve sees at one instant.
#[derive(Clone, Copy, Debug)]
pub struct Oil {
    pub rho: f64,
    pub nu: f64,
}

impl Oil {
    pub fn q(&self, area: f64, dp: f64) -> f64 {
        orifice(CD, area, dp, self.rho, self.nu)
    }
}

/// Flows at a four-way valve's ports: into chamber A and B, out of supply, into return.
#[derive(Clone, Copy, Debug, Default)]
pub struct PortFlows {
    pub a: f64,
    pub b: f64,
    pub supply: f64,
    pub ret: f64,
}

/// How a servo valve has failed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SpoolFault {
    None,
    /// Hard over: the first stage saturates to one side.
    HardOver(f64),
    /// Seized where it is.
    Jammed,
    /// Silted: input below this share does nothing.
    Silted(f64),
}

/// A flapper-nozzle two-stage servo valve: spool position follows the torque-motor current as a second-order lag.
#[derive(Clone, Copy, Debug)]
pub struct ServoValve {
    /// Natural frequency of the spool, rad/s.
    pub wn: f64,
    pub zeta: f64,
    /// Metering area of one edge at full spool.
    pub area: f64,
    /// Leak area per edge at null: the first stage's bleed and the spool's radial clearance.
    pub leak: f64,
    /// Lap as a share of spool travel: positive for overlap (dead zone), negative for underlap.
    pub lap: f64,
    /// Input bias, share of full current: null shift from wear or temperature.
    pub bias: f64,
    pub fault: SpoolFault,
}

impl ServoValve {
    /// Sized from its rated flow at a 1000 psi valve drop and its null leakage at `supply`.
    pub fn rated(flow: f64, null_leak: f64, supply: f64, rho: f64, bandwidth_hz: f64) -> Self {
        let area = area_for(CD, flow, 500.0 * PSI, rho);
        let leak = area_for(CD, null_leak.max(0.0) * 0.5, 0.5 * supply.max(1e5), rho);
        ServoValve { wn: 2.0 * std::f64::consts::PI * bandwidth_hz.max(5.0), zeta: 0.7, area, leak, lap: 0.0, bias: 0.0, fault: SpoolFault::None }
    }

    /// Spool acceleration for input `u` (share of full current), spool at `x` moving at `v`.
    pub fn accel(&self, u: f64, x: f64, v: f64) -> f64 {
        let u = match self.fault {
            SpoolFault::HardOver(s) => s.signum(),
            SpoolFault::Jammed => return -v * 4.0 * self.wn,
            SpoolFault::Silted(d) => {
                let u = u + self.bias;
                if u.abs() < d {
                    0.0
                } else {
                    u - d * u.signum()
                }
            }
            SpoolFault::None => u + self.bias,
        };
        let u = u.clamp(-1.0, 1.0);
        let stop = if x.abs() > 1.05 { -(x.abs() - 1.05) * x.signum() * 50.0 } else { 0.0 };
        self.wn * self.wn * (u - x + stop) - 2.0 * self.zeta * self.wn * v
    }

    fn opening(&self, x: f64) -> f64 {
        let width = 0.01;
        self.area * soft_pos(x - self.lap, width).min(1.0 + self.lap.abs())
    }

    /// Port flows with the spool at `x`.
    pub fn flows(&self, x: f64, ps: f64, pr: f64, pa: f64, pb: f64, oil: Oil) -> PortFlows {
        let pa_in = self.opening(x) + self.leak;
        let ar = self.opening(-x) + self.leak;
        let pb_in = self.opening(-x) + self.leak;
        let br = self.opening(x) + self.leak;
        let (q1, q2, q3, q4) = (oil.q(pa_in, ps - pa), oil.q(ar, pa - pr), oil.q(pb_in, ps - pb), oil.q(br, pb - pr));
        PortFlows { a: q1 - q2, b: q3 - q4, supply: q1 + q3, ret: q2 + q4 }
    }
}

/// What a selector valve's centre does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Centre {
    /// Ports blocked: holds the actuator.
    Closed,
    /// Both ports to return: the actuator floats, as gear selector valves do in free fall.
    Float,
}

/// A solenoid-pilot three-position directional valve: extend, centre, retract.
#[derive(Clone, Copy, Debug)]
pub struct Selector {
    pub area: f64,
    pub leak: f64,
    pub centre: Centre,
    /// Time to move between positions.
    pub time: f64,
}

impl Selector {
    pub fn rated(flow: f64, drop: f64, rho: f64, centre: Centre, time: f64) -> Self {
        Selector { area: area_for(CD, flow, drop.max(1e4), rho), leak: area_for(CD, 1e-7, 1e7, rho), centre, time: time.max(0.005) }
    }

    /// Valve position moves toward `cmd` in -1..1 at its switching speed.
    pub fn rate(&self, cmd: f64, pos: f64) -> f64 {
        let k = 4.0 / self.time;
        k * (cmd.clamp(-1.0, 1.0) - pos)
    }

    pub fn flows(&self, pos: f64, ps: f64, pr: f64, pa: f64, pb: f64, oil: Oil) -> PortFlows {
        let ext = self.area * soft_pos(pos, 0.02).min(1.0);
        let ret = self.area * soft_pos(-pos, 0.02).min(1.0);
        let float = if self.centre == Centre::Float { self.area * (1.0 - pos.abs()).clamp(0.0, 1.0) } else { 0.0 };
        let q_pa = oil.q(ext + self.leak, ps - pa);
        let q_ar = oil.q(ret + float + self.leak, pa - pr);
        let q_pb = oil.q(ret + self.leak, ps - pb);
        let q_br = oil.q(ext + float + self.leak, pb - pr);
        PortFlows { a: q_pa - q_ar, b: q_pb - q_br, supply: q_pa + q_pb, ret: q_ar + q_br }
    }
}

/// Relief valve on a chamber: cracks at `crack`, full open 10 % above, to return.
#[derive(Clone, Copy, Debug)]
pub struct Relief {
    pub crack: f64,
    pub area: f64,
}

impl Relief {
    pub fn sized(crack: f64, flow: f64, rho: f64) -> Self {
        Relief { crack, area: area_for(CD, flow, 0.1 * crack.max(1e5), rho) }
    }
    pub fn flow(&self, p: f64, pr: f64, oil: Oil) -> f64 {
        if self.crack <= 0.0 {
            return 0.0;
        }
        let open = smooth_step(p, self.crack + 0.05 * self.crack, 0.1 * self.crack);
        if open <= 0.0 {
            return 0.0;
        }
        oil.q(self.area * open, p - pr).max(0.0)
    }
}

/// Anti-cavitation check valve refilling a chamber from return when it drops below return pressure.
#[derive(Clone, Copy, Debug)]
pub struct Refill {
    pub area: f64,
}

impl Refill {
    pub fn flow(&self, p: f64, pr: f64, oil: Oil) -> f64 {
        let open = smooth_step(pr - p, 10_000.0, 20_000.0);
        if open <= 0.0 {
            return 0.0;
        }
        oil.q(self.area * open, pr - p).max(0.0)
    }
}

/// A check valve from `a` to `b`, cracking at `crack`.
pub fn check(area: f64, crack: f64, pa: f64, pb: f64, oil: Oil) -> f64 {
    let open = smooth_step(pa - pb, crack, crack.max(5_000.0));
    if open <= 0.0 {
        return 0.0;
    }
    oil.q(area * open, pa - pb).max(0.0)
}

/// A pressure-control servo valve: its spool balances the commanded pressure against the output pressure fed back
/// onto it, metering supply in or bleeding the output to return. Brakes use one per wheel group.
#[derive(Clone, Copy, Debug)]
pub struct PressureControl {
    pub area: f64,
    pub leak: f64,
    /// Pressure at full input.
    pub p_max: f64,
    /// Spool time constant.
    pub tau: f64,
    /// Feedback gain: spool travel per share of pressure error.
    pub gain: f64,
}

impl PressureControl {
    pub fn rated(flow: f64, p_max: f64, rho: f64) -> Self {
        PressureControl { area: area_for(CD, flow, 0.5 * p_max.max(1e5), rho), leak: area_for(CD, 0.05e-3 / 60.0, p_max.max(1e5), rho), p_max: p_max.max(1e5), tau: 0.008, gain: 8.0 }
    }

    /// Where the spool wants to be for input `u` (0..1) with the output at `p`.
    pub fn target(&self, u: f64, p: f64) -> f64 {
        (self.gain * (u.clamp(0.0, 1.0) - p / self.p_max)).clamp(-1.0, 1.0)
    }

    pub fn flow(&self, x: f64, ps: f64, pr: f64, p: f64, oil: Oil) -> (f64, f64, f64) {
        let fill = self.area * soft_pos(x, 0.01).min(1.0) + self.leak;
        let dump = self.area * soft_pos(-x, 0.01).min(1.0) + self.leak;
        let qi = oil.q(fill, ps - p);
        let qo = oil.q(dump, p - pr);
        (qi - qo, qi.max(0.0), qo.max(0.0))
    }
}

/// Shuttle valve: the output follows whichever input is higher, with a small hysteresis band.
#[derive(Clone, Copy, Debug, Default)]
pub struct Shuttle {
    /// 0 on the first input, 1 on the second.
    pub pos: f64,
}

impl Shuttle {
    pub fn update(&mut self, p1: f64, p2: f64, dt: f64) {
        let band = 30.0 * PSI;
        let want = if p2 > p1 + band { 1.0 } else if p1 > p2 + band { 0.0 } else { self.pos };
        let rate = dt / 0.02;
        self.pos += (want - self.pos).clamp(-rate, rate);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OIL: Oil = Oil { rho: 1000.0, nu: 10e-6 };

    #[test]
    fn servo_valve_passes_its_rated_flow() {
        let ps = 3000.0 * PSI;
        let sv = ServoValve::rated(40e-3 / 60.0, 0.5e-3 / 60.0, ps, 1000.0, 80.0);
        let f = sv.flows(1.0, ps, 0.0, ps - 500.0 * PSI, 500.0 * PSI, OIL);
        assert!((f.a * 60e3 - 40.0).abs() < 3.0, "{} L/min", f.a * 60e3);
        let null = sv.flows(0.0, ps, 0.0, 0.5 * ps, 0.5 * ps, OIL);
        assert!(null.supply * 60e3 < 1.0 && null.supply > 0.0, "null {} L/min", null.supply * 60e3);
    }

    #[test]
    fn blocked_selector_holds_and_floating_one_lets_go() {
        let closed = Selector::rated(1e-3, 1e6, 1000.0, Centre::Closed, 0.05);
        let float = Selector { centre: Centre::Float, ..closed };
        let ps = 2e7;
        assert!(closed.flows(0.0, ps, 3e5, 1e7, 1e7, OIL).a.abs() < 1e-5);
        assert!(float.flows(0.0, ps, 3e5, 1e7, 1e7, OIL).a < -1e-4);
    }

    #[test]
    fn pressure_control_valve_settles_on_its_command() {
        let v = PressureControl::rated(10e-3 / 60.0, 3000.0 * PSI, 1000.0);
        let mut p = 0.0;
        let c = 1e-13;
        for _ in 0..20000 {
            let x = v.target(0.5, p);
            let (q, _, _) = v.flow(x, 3000.0 * PSI, 0.0, p, OIL);
            p += q / c * 1e-6;
        }
        assert!((p / PSI - 1500.0).abs() < 150.0, "{} psi", p / PSI);
    }
}
