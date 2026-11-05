//! Position and speed transducers as the computers see them: a dual-coil LVDT or RVDT whose demodulated output
//! carries gain error, offset, noise, a filter lag and the converter's quantisation, with a secondary-sum monitor
//! that catches open coils; and an inductive wheel-speed tachometer that counts teeth over a window, so it loses
//! the wheel below a few knots and steps in quanta.

/// How one transducer channel has failed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Fault {
    None,
    /// Primary open: no output, and the sum check sees it.
    OpenPrimary,
    /// One secondary open: reads full scale one way, and the sum check sees it.
    OpenSecondary,
    /// Core come off its rod: holds where it was; the sum still looks healthy.
    Detached,
    /// Offset drifting with time: the sum still looks healthy.
    Drift,
}

/// One LVDT or RVDT channel, reading a position in [lo, hi].
#[derive(Clone, Debug)]
pub struct Lvdt {
    pub lo: f64,
    pub hi: f64,
    gain: f64,
    offset: f64,
    noise: f64,
    tau: f64,
    bits: u32,
    pub value: f64,
    pub sum_ok: bool,
    held: f64,
    last: f64,
    drift: f64,
    rng: u64,
}

impl Lvdt {
    /// `seed` makes each channel's errors its own; gain error and offset are a few tenths of a percent.
    pub fn new(lo: f64, hi: f64, seed: u64) -> Self {
        let mut s = Lvdt { lo, hi, gain: 1.0, offset: 0.0, noise: 2e-4, tau: 0.004, bits: 14, value: 0.5 * (lo + hi), sum_ok: true, held: f64::NAN, last: 0.5, drift: 0.0, rng: seed | 1 };
        s.gain = 1.0 + 0.003 * (s.uniform() - 0.5);
        s.offset = 0.002 * (s.uniform() - 0.5);
        s
    }

    fn uniform(&mut self) -> f64 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Reads `truth`; returns the converted value.
    pub fn read(&mut self, truth: f64, fault: Fault, dt: f64) -> f64 {
        let span = (self.hi - self.lo).max(1e-9);
        let share = (truth - self.lo) / span;
        let raw = match fault {
            Fault::OpenPrimary => {
                self.sum_ok = false;
                0.5
            }
            Fault::OpenSecondary => {
                self.sum_ok = false;
                1.1
            }
            Fault::Detached => {
                self.sum_ok = true;
                if self.held.is_nan() {
                    self.held = self.last;
                }
                self.held
            }
            Fault::Drift => {
                self.sum_ok = true;
                self.drift += 0.01 * dt;
                share + self.drift
            }
            Fault::None => {
                self.sum_ok = true;
                self.held = f64::NAN;
                self.drift = 0.0;
                self.last = share;
                share
            }
        };
        let n = (self.uniform() + self.uniform() - 1.0) * self.noise * 1.7;
        let demod = (raw * self.gain + self.offset + n).clamp(-0.1, 1.15);
        let q = (1u64 << self.bits) as f64;
        let converted = (demod * q).round() / q;
        let out = self.lo + converted * span;
        self.value += (out - self.value) * (dt / self.tau).min(1.0);
        self.value
    }
}

/// A pair of channels and their comparison, as an actuator's or surface's computer reads them.
#[derive(Clone, Debug)]
pub struct Dual {
    pub a: Lvdt,
    pub b: Lvdt,
    /// Channel difference beyond which they disagree, share of span.
    pub threshold: f64,
}

impl Dual {
    pub fn new(lo: f64, hi: f64, seed: u64) -> Self {
        Dual { a: Lvdt::new(lo, hi, seed), b: Lvdt::new(lo, hi, seed.rotate_left(17) ^ 0x5bd1_e995), threshold: 0.03 }
    }

    /// Reads both; returns (channel 1, channel 2, the value a monitor would use, disagree).
    pub fn read(&mut self, truth: f64, f1: Fault, f2: Fault, dt: f64) -> (f64, f64, f64, bool) {
        let v1 = self.a.read(truth, f1, dt);
        let v2 = self.b.read(truth, f2, dt);
        let span = (self.a.hi - self.a.lo).max(1e-9);
        let disagree = (v1 - v2).abs() > self.threshold * span || !self.a.sum_ok || !self.b.sum_ok;
        // A failed sum drops that channel; with both healthy but apart the computer cannot tell and keeps channel 1.
        let used = if !self.a.sum_ok && self.b.sum_ok { v2 } else { v1 };
        (v1, v2, used, disagree)
    }
}

/// An inductive wheel-speed transducer on a toothed exciter ring.
#[derive(Clone, Debug)]
pub struct Tacho {
    pub teeth: f64,
    pub window: f64,
    count: f64,
    elapsed: f64,
    pub speed: f64,
}

impl Tacho {
    pub fn new(teeth: f64, window: f64) -> Self {
        Tacho { teeth: teeth.max(4.0), window: window.max(0.002), count: 0.0, elapsed: 0.0, speed: 0.0 }
    }

    /// Counts teeth passing; refreshes the reading each window. Below the transducer's minimum signal the
    /// voltage is too small to count, so it reads nothing.
    pub fn update(&mut self, omega: f64, radius: f64, failed: bool, dt: f64) -> f64 {
        let v_rim = omega.abs() * radius;
        let signal = v_rim > 1.0;
        if signal && !failed {
            self.count += omega.abs() * dt * self.teeth / std::f64::consts::TAU;
        }
        self.elapsed += dt;
        if self.elapsed >= self.window {
            let pulses = self.count.floor();
            self.count -= pulses;
            self.speed = pulses / self.teeth * std::f64::consts::TAU / self.elapsed * radius;
            self.elapsed = 0.0;
        }
        if failed {
            self.speed = 0.0;
        }
        self.speed
    }
}

/// Fault from a numeric code: 0 none, 1 open primary, 2 open secondary, 3 core detached, 4 drifting.
pub fn fault_code(n: f64) -> Fault {
    match n.round() as i64 {
        1 => Fault::OpenPrimary,
        2 => Fault::OpenSecondary,
        3 => Fault::Detached,
        4 => Fault::Drift,
        _ => Fault::None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lvdt_reads_close_and_quantised() {
        let mut l = Lvdt::new(0.0, 0.1, 42);
        let mut v = 0.0;
        for _ in 0..200 {
            v = l.read(0.05, Fault::None, 0.005);
        }
        assert!((v - 0.05).abs() < 0.0005, "{v}");
        assert!(l.sum_ok);
    }

    #[test]
    fn an_open_coil_trips_the_sum_and_the_pair_uses_the_other() {
        let mut d = Dual::new(0.0, 1.0, 7);
        let mut r = (0.0, 0.0, 0.0, false);
        for _ in 0..100 {
            r = d.read(0.3, Fault::OpenPrimary, Fault::None, 0.005);
        }
        assert!(r.3, "disagree flagged");
        assert!((r.2 - 0.3).abs() < 0.01, "used {}", r.2);
    }

    #[test]
    fn a_detached_core_looks_healthy_but_disagrees() {
        let mut d = Dual::new(0.0, 1.0, 9);
        for _ in 0..100 {
            d.read(0.2, Fault::None, Fault::None, 0.005);
        }
        let mut r = (0.0, 0.0, 0.0, false);
        for _ in 0..100 {
            r = d.read(0.7, Fault::Detached, Fault::None, 0.005);
        }
        assert!(d.a.sum_ok && r.3 && (r.0 - 0.7).abs() > 0.3, "{r:?}");
    }

    #[test]
    fn tacho_loses_the_wheel_when_slow() {
        let mut t = Tacho::new(48.0, 0.02);
        let mut v = 0.0;
        for _ in 0..100 {
            v = t.update(50.0 / 0.6, 0.6, false, 0.005);
        }
        assert!((v - 50.0).abs() < 2.0, "{v}");
        for _ in 0..100 {
            v = t.update(0.5 / 0.6, 0.6, false, 0.005);
        }
        assert_eq!(v, 0.0);
    }
}
