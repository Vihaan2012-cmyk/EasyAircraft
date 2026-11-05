//! Generator control unit protection. Each function watches one measurement against its limit and time,
//! and the first to run out trips the generator with its fault code:
//! 1 overvoltage, 2 undervoltage, 3 overfrequency or overspeed, 4 underfrequency, 5 overload, 6 overcurrent,
//! 7 differential (a fault inside the generator's zone), 8 open phase, 9 unbalanced current,
//! 10 loss of excitation, 12 reverse current.

/// What the control unit measures this step, per unit of the generator's rating.
#[derive(Clone, Copy, Debug, Default)]
pub struct Measure {
    pub v_max: f64,
    pub v_min: f64,
    pub v_avg: f64,
    pub freq: f64,
    pub i_max: f64,
    pub i_min: f64,
    pub i_neg: f64,
    pub differential: f64,
    pub load_share: f64,
    pub reverse: f64,
    pub online: bool,
    pub excited: bool,
    pub ac: bool,
    pub constant_frequency: bool,
    pub three_phase: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub overload_heat: f64,
    pub differential: bool,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Gcu {
    pub heat: f64,
    pub tripped: bool,
    pub fault: f64,
    /// Differential trips lock out: only a deliberate reset clears them.
    pub lockout: bool,
    ov: f64,
    ov_fast: f64,
    uv: f64,
    oc: f64,
    os: f64,
    of: f64,
    uf: f64,
    dp: f64,
    open: f64,
    unbal: f64,
    rev: f64,
    pub reset_was: bool,
}

fn timer(t: &mut f64, bad: bool, dt: f64) -> f64 {
    *t = if bad { *t + dt } else { 0.0 };
    *t
}

impl Gcu {
    pub fn trip(&mut self, code: f64) {
        if !self.tripped {
            self.tripped = true;
            self.fault = code;
            if code == 7.0 {
                self.lockout = true;
            }
        }
    }

    /// Clears a trip when the generator stops being driven or on the rising edge of its reset.
    pub fn reset(&mut self, driven: bool, reset: bool) {
        let edge = reset && !self.reset_was;
        if edge {
            self.lockout = false;
        }
        if (!driven && !self.lockout) || edge {
            self.tripped = false;
            self.fault = 0.0;
        }
        self.reset_was = reset;
    }

    /// The drive's own overspeed watch, which runs whether or not the generator is on line.
    pub fn overspeed(&mut self, spinning: bool, speed: f64, max_speed: f64, dt: f64) {
        if timer(&mut self.os, spinning && speed > max_speed, dt) > 1.0 {
            self.trip(3.0);
        }
    }

    pub fn protect(&mut self, m: &Measure, lim: &Limits, dt: f64) {
        if !m.online {
            for t in [&mut self.ov, &mut self.ov_fast, &mut self.uv, &mut self.oc, &mut self.of, &mut self.uf, &mut self.dp, &mut self.open, &mut self.unbal, &mut self.rev] {
                *t = 0.0;
            }
            self.heat = (self.heat - dt).max(0.0);
            // Overvoltage still trips off line: a regulator failed high must not close onto the bus.
            if m.excited && timer(&mut self.ov_fast, m.v_max > 1.25, dt) > 0.1 {
                self.trip(1.0);
            }
            return;
        }
        if timer(&mut self.ov, m.v_max > 1.12, dt) > 0.25 || timer(&mut self.ov_fast, m.v_max > 1.35, dt) > 0.03 {
            self.trip(1.0);
        }
        if timer(&mut self.uv, m.v_avg < 0.85, dt) > 4.0 {
            self.trip(2.0);
        }
        if m.constant_frequency {
            if timer(&mut self.of, m.freq > 1.05, dt) > 1.0 {
                self.trip(3.0);
            }
            if timer(&mut self.uf, m.freq < 0.93, dt) > 1.0 {
                self.trip(4.0);
            }
        }
        if timer(&mut self.oc, m.i_max > 3.0, dt) > 0.5 {
            self.trip(6.0);
        }
        if lim.differential && timer(&mut self.dp, m.differential > 0.15, dt) > 0.03 {
            self.trip(7.0);
        }
        let open_v = m.v_min < 0.5 && m.v_max > 0.9;
        if m.three_phase && timer(&mut self.open, open_v || (m.i_max > 0.3 && m.i_min < 0.1 * m.i_max), dt) > 4.0 {
            self.trip(8.0);
        }
        if m.three_phase && timer(&mut self.unbal, m.i_neg > 0.2, dt) > 5.0 {
            self.trip(9.0);
        }
        if !m.ac && timer(&mut self.rev, m.reverse > 0.05, dt) > 0.2 {
            self.trip(12.0);
        }
        let share = m.load_share.min(3.0);
        self.heat = (self.heat + (share * share - 1.0) * dt).max(0.0);
        if self.heat > lim.overload_heat {
            self.trip(5.0);
        }
    }

    pub fn state(&self, v: &mut Vec<f64>) {
        let b = |x: bool| x as u8 as f64;
        v.extend([self.heat, b(self.tripped), self.fault, b(self.lockout), self.ov, self.ov_fast, self.uv, self.oc, self.os, self.of, self.uf, self.dp, self.open, self.unbal, self.rev, b(self.reset_was)]);
    }

    pub fn restore(&mut self, it: &mut dyn Iterator<Item = f64>) {
        let mut n = || it.next().unwrap_or(0.0);
        self.heat = n();
        self.tripped = n() != 0.0;
        self.fault = n();
        self.lockout = n() != 0.0;
        for t in [&mut self.ov, &mut self.ov_fast, &mut self.uv, &mut self.oc, &mut self.os, &mut self.of, &mut self.uf, &mut self.dp, &mut self.open, &mut self.unbal, &mut self.rev] {
            *t = n();
        }
        self.reset_was = n() != 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on() -> Measure {
        Measure { v_max: 1.0, v_min: 1.0, v_avg: 1.0, freq: 1.0, i_max: 0.5, i_min: 0.5, online: true, excited: true, ac: true, three_phase: true, ..Default::default() }
    }

    const LIM: Limits = Limits { overload_heat: 375.0, differential: true };

    #[test]
    fn a_differential_trip_locks_out_until_reset() {
        let mut g = Gcu::default();
        let m = Measure { differential: 0.5, ..on() };
        for _ in 0..10 {
            g.protect(&m, &LIM, 0.01);
        }
        assert!(g.tripped && g.fault == 7.0 && g.lockout);
        g.reset(false, false);
        assert!(g.tripped, "stopping the drive does not clear a lockout");
        g.reset(true, true);
        assert!(!g.tripped && !g.lockout);
    }

    #[test]
    fn healthy_measurements_never_trip() {
        let mut g = Gcu::default();
        for _ in 0..10_000 {
            g.protect(&on(), &LIM, 0.01);
        }
        assert!(!g.tripped);
    }

    #[test]
    fn an_open_phase_trips_after_four_seconds() {
        let mut g = Gcu::default();
        let m = Measure { i_max: 0.8, i_min: 0.0, ..on() };
        for _ in 0..390 {
            g.protect(&m, &LIM, 0.01);
        }
        assert!(!g.tripped);
        for _ in 0..20 {
            g.protect(&m, &LIM, 0.01);
        }
        assert_eq!(g.fault, 8.0);
    }
}
