//! What a FADEC channel sees of its engine: each speed, temperature and pressure through two probes, one per
//! channel, with lag, quantisation and a little offset, failed probes dropped, and the two compared so a
//! disagreement beyond tolerance is flagged and the plausible one kept.

/// Probe failure codes, as a part's `*_sensor_failed` value: 0 healthy, 1 channel A's probe, 2 channel B's,
/// 3 both.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Lost {
    None,
    A,
    B,
    Both,
}

pub fn lost(code: f64) -> Lost {
    match code.round() as i64 {
        1 => Lost::A,
        2 => Lost::B,
        n if n >= 3 => Lost::Both,
        _ => Lost::None,
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Probe {
    tau: f64,
    step: f64,
    offset: [f64; 2],
    read: [f64; 2],
    /// Tolerance between the two before they disagree.
    tol: f64,
    primed: bool,
}

/// One measurement as the channels have it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Reading {
    pub value: f64,
    pub valid: bool,
    pub disagree: bool,
}

impl Probe {
    pub fn new(tau: f64, step: f64, offset: f64, tol: f64) -> Self {
        Probe { tau, step, offset: [offset, -0.6 * offset], read: [0.0; 2], tol, primed: false }
    }

    pub fn sense(&mut self, truth: f64, lost: Lost, dt: f64) -> Reading {
        if !self.primed {
            self.read = [truth; 2];
            self.primed = truth.abs() > 1.0;
        }
        let a = (dt / self.tau).min(1.0);
        for k in 0..2 {
            let seen = truth + self.offset[k];
            self.read[k] += (seen - self.read[k]) * a;
        }
        let q = |v: f64| if self.step > 0.0 { (v / self.step).round() * self.step } else { v };
        let (ra, rb) = (q(self.read[0]), q(self.read[1]));
        match lost {
            Lost::None => {
                let disagree = (ra - rb).abs() > self.tol;
                Reading { value: 0.5 * (ra + rb), valid: true, disagree }
            }
            Lost::A => Reading { value: rb, valid: true, disagree: false },
            Lost::B => Reading { value: ra, valid: true, disagree: false },
            Lost::Both => Reading { value: 0.0, valid: false, disagree: false },
        }
    }

    pub fn state(&self, v: &mut Vec<f64>) {
        v.extend([self.read[0], self.read[1], self.primed as u8 as f64]);
    }

    pub fn restore(&mut self, it: &mut dyn Iterator<Item = f64>) {
        self.read = [it.next().unwrap_or(0.0), it.next().unwrap_or(0.0)];
        self.primed = it.next().unwrap_or(0.0) != 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lost_probe_hands_over_to_the_other_channel() {
        let mut p = Probe::new(0.05, 0.1, 0.2, 2.0);
        for _ in 0..100 {
            p.sense(80.0, Lost::None, 0.01);
        }
        let both = p.sense(80.0, Lost::None, 0.01);
        let one = p.sense(80.0, Lost::A, 0.01);
        assert!(both.valid && one.valid && (both.value - 80.0).abs() < 0.2 && (one.value - 80.0).abs() < 0.3);
        assert!(!p.sense(80.0, Lost::Both, 0.01).valid);
    }
}
