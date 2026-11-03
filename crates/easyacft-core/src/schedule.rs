//! Fixed-rate stepping on sim time, per domain.

pub trait Stepped {
    fn step(&mut self, dt: f64);
}

#[derive(Clone, Debug)]
pub struct Rate {
    period: f64,
    owed: f64,
    max_steps: usize,
}

impl Rate {
    /// `max_steps` caps catch-up after a long frame.
    pub fn new(period_s: f64, max_steps: usize) -> Self {
        Self { period: period_s.max(1e-6), owed: 0.0, max_steps: max_steps.max(1) }
    }

    pub fn period(&self) -> f64 {
        self.period
    }

    pub fn set_period(&mut self, period_s: f64) {
        self.period = period_s.max(1e-6);
    }

    /// How many steps of `period` to run for `elapsed` seconds of sim time.
    pub fn due(&mut self, elapsed: f64) -> usize {
        self.owed += elapsed.max(0.0);
        let n = (self.owed / self.period + 1e-9).floor() as usize;
        let run = n.min(self.max_steps);
        self.owed = if n > self.max_steps { 0.0 } else { self.owed - run as f64 * self.period };
        run
    }

    pub fn run(&mut self, elapsed: f64, target: &mut dyn Stepped) -> usize {
        let n = self.due(elapsed);
        for _ in 0..n {
            target.step(self.period);
        }
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Count(usize, f64);
    impl Stepped for Count {
        fn step(&mut self, dt: f64) {
            self.0 += 1;
            self.1 += dt;
        }
    }

    #[test]
    fn steps_follow_sim_time_whatever_the_frame_split() {
        let mut a = (Rate::new(0.002, 1000), Count(0, 0.0));
        let mut b = (Rate::new(0.002, 1000), Count(0, 0.0));
        for _ in 0..60 {
            a.0.run(1.0 / 60.0, &mut a.1);
        }
        for _ in 0..25 {
            b.0.run(1.0 / 25.0, &mut b.1);
        }
        assert_eq!(a.1 .0, 500);
        assert_eq!(b.1 .0, 500);
    }

    #[test]
    fn slow_domains_step_rarely_and_a_long_frame_is_bounded() {
        let mut thermal = (Rate::new(0.1, 5), Count(0, 0.0));
        thermal.0.run(0.05, &mut thermal.1);
        assert_eq!(thermal.1 .0, 0);
        thermal.0.run(0.05, &mut thermal.1);
        assert_eq!(thermal.1 .0, 1);
        thermal.0.run(10.0, &mut thermal.1);
        assert_eq!(thermal.1 .0, 6);
        thermal.0.run(0.1, &mut thermal.1);
        assert_eq!(thermal.1 .0, 7);
    }
}
