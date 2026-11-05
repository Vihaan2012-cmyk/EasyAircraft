//! A stiff ODE stepper for the mechanisms: a two-stage L-stable Rosenbrock method (ROS2) with a numerical
//! Jacobian, an embedded first-order estimate for error control, step halving on rejection and a finite check, so
//! hydraulic chambers a thousand times stiffer than the frame rate still step without blowing up.

/// Most substeps one call may take before it accepts what it has.
const MAX_SUBSTEPS: u32 = 256;
/// The ROS2 stage coefficient, 1 + 1/sqrt(2).
const GAMMA: f64 = 1.0 + std::f64::consts::FRAC_1_SQRT_2;

/// Workspace for one system size, reused between steps so stepping does not allocate.
#[derive(Clone, Debug, Default)]
pub struct Stepper {
    n: usize,
    jac: Vec<f64>,
    lu: Vec<f64>,
    piv: Vec<usize>,
    f0: Vec<f64>,
    f1: Vec<f64>,
    k1: Vec<f64>,
    k2: Vec<f64>,
    xt: Vec<f64>,
    x0: Vec<f64>,
    /// Substep the last call ended on, kept so the next call starts from a size that worked.
    pub h: f64,
    /// Substeps taken over the stepper's life, for profiling.
    pub substeps: u64,
    /// Steps where error control gave up and accepted a step at the smallest size.
    pub forced: u64,
    /// Steps a Jacobian is reused for: ROS2 keeps its order with an approximate one, and a rejected step
    /// always gets a fresh one.
    pub refresh: u32,
    /// Relative tolerance added to each state's absolute one.
    pub rtol: f64,
    jac_age: u32,
    jac_ok: bool,
}

/// Outcome of one call.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Outcome {
    Ok,
    /// The state went non-finite and was put back to where the call started.
    Restored,
}

impl Stepper {
    pub fn new(n: usize) -> Self {
        let mut s = Stepper { refresh: 4, rtol: 0.01, ..Stepper::default() };
        s.resize(n);
        s
    }

    fn resize(&mut self, n: usize) {
        if self.n == n && !self.jac.is_empty() {
            return;
        }
        self.n = n;
        self.jac_ok = false;
        self.jac = vec![0.0; n * n];
        self.lu = vec![0.0; n * n];
        self.piv = vec![0; n];
        for v in [&mut self.f0, &mut self.f1, &mut self.k1, &mut self.k2, &mut self.xt, &mut self.x0] {
            *v = vec![0.0; n];
        }
    }

    /// Advances `x` by `dt` under `f(x, dxdt)`. `scale` gives each state's absolute tolerance: the size of change
    /// that counts as one unit of error.
    pub fn step(&mut self, x: &mut [f64], dt: f64, scale: &[f64], tol: f64, mut f: impl FnMut(&[f64], &mut [f64])) -> Outcome {
        let n = x.len();
        self.resize(n);
        if n == 0 || dt <= 0.0 {
            return Outcome::Ok;
        }
        self.x0.copy_from_slice(x);
        let mut t = 0.0;
        let mut h = if self.h > 0.0 { self.h.min(dt) } else { dt };
        let h_min = dt / MAX_SUBSTEPS as f64;
        let mut taken = 0u32;
        while t < dt * (1.0 - 1e-12) {
            h = h.min(dt - t).max(h_min.min(dt - t));
            let err = self.attempt(x, h, scale, &mut f) / tol.max(1e-12);
            taken += 1;
            let ok = err.is_finite() && err <= 1.0;
            if ok || h <= h_min * 1.0001 || taken > MAX_SUBSTEPS * 2 {
                if !ok {
                    self.forced += 1;
                }
                if !err.is_finite() || x.iter().any(|v| !v.is_finite()) {
                    x.copy_from_slice(&self.x0);
                    self.h = h_min;
                    self.jac_ok = false;
                    return Outcome::Restored;
                }
                x.copy_from_slice(&self.xt);
                t += h;
                self.substeps += 1;
                // Grows gently after a good step, as error scales with h squared.
                let grow = if err > 1e-10 { (0.9 / err.sqrt()).clamp(0.2, 4.0) } else { 4.0 };
                h *= grow;
            } else {
                if self.jac_age > 0 {
                    // A stale Jacobian may be why: retry at the same size with a fresh one first.
                    self.jac_ok = false;
                    continue;
                }
                h *= (0.9 / err.sqrt()).clamp(0.1, 0.5);
            }
        }
        self.h = h.min(dt);
        Outcome::Ok
    }

    /// One ROS2 step of size `h` from `x` into `xt`; returns the scaled error norm.
    fn attempt(&mut self, x: &[f64], h: f64, scale: &[f64], f: &mut impl FnMut(&[f64], &mut [f64])) -> f64 {
        let n = x.len();
        f(x, &mut self.f0);
        if !self.jac_ok || self.jac_age >= self.refresh.max(1) {
            self.jacobian(x, scale, f);
            self.jac_ok = true;
            self.jac_age = 0;
        } else {
            self.jac_age += 1;
        }
        for i in 0..n {
            for j in 0..n {
                self.lu[i * n + j] = if i == j { 1.0 } else { 0.0 } - GAMMA * h * self.jac[i * n + j];
            }
        }
        if !lu_decompose(&mut self.lu, &mut self.piv, n) {
            return f64::INFINITY;
        }
        self.k1.copy_from_slice(&self.f0);
        lu_solve(&self.lu, &self.piv, n, &mut self.k1);
        for i in 0..n {
            self.xt[i] = x[i] + h * self.k1[i];
        }
        f(&self.xt, &mut self.f1);
        for i in 0..n {
            self.k2[i] = self.f1[i] - 2.0 * self.k1[i];
        }
        lu_solve(&self.lu, &self.piv, n, &mut self.k2);
        let mut err = 0.0f64;
        for i in 0..n {
            let step = h * (1.5 * self.k1[i] + 0.5 * self.k2[i]);
            let low = h * self.k1[i];
            self.xt[i] = x[i] + step;
            let s = (scale[i].abs() + self.rtol * x[i].abs().max(self.xt[i].abs())).max(1e-30);
            err = err.max(((step - low) / s).abs());
        }
        if self.xt.iter().any(|v| !v.is_finite()) {
            return f64::INFINITY;
        }
        err
    }

    fn jacobian(&mut self, x: &[f64], scale: &[f64], f: &mut impl FnMut(&[f64], &mut [f64])) {
        let n = x.len();
        self.xt.copy_from_slice(x);
        for j in 0..n {
            let e = 1e-6 * x[j].abs().max(scale[j].abs()).max(1e-12);
            self.xt[j] = x[j] + e;
            f(&self.xt, &mut self.f1);
            self.xt[j] = x[j];
            for i in 0..n {
                let d = (self.f1[i] - self.f0[i]) / e;
                self.jac[i * n + j] = if d.is_finite() { d } else { 0.0 };
            }
        }
    }
}

/// In-place LU decomposition with partial pivoting; false when the matrix is singular.
pub fn lu_decompose(a: &mut [f64], piv: &mut [usize], n: usize) -> bool {
    for (i, p) in piv.iter_mut().enumerate().take(n) {
        *p = i;
    }
    for k in 0..n {
        let mut best = k;
        let mut big = a[k * n + k].abs();
        for i in k + 1..n {
            let v = a[i * n + k].abs();
            if v > big {
                big = v;
                best = i;
            }
        }
        if !(big > 1e-300) {
            return false;
        }
        if best != k {
            for j in 0..n {
                a.swap(k * n + j, best * n + j);
            }
            piv.swap(k, best);
        }
        let d = a[k * n + k];
        for i in k + 1..n {
            let m = a[i * n + k] / d;
            a[i * n + k] = m;
            if m != 0.0 {
                for j in k + 1..n {
                    a[i * n + j] -= m * a[k * n + j];
                }
            }
        }
    }
    true
}

/// Solves with a factor from `lu_decompose`; `b` holds the right side and gets the answer.
pub fn lu_solve(a: &[f64], piv: &[usize], n: usize, b: &mut [f64]) {
    let mut y = [0.0f64; 64];
    let tmp: &mut [f64] = if n <= 64 { &mut y[..n] } else { return lu_solve_big(a, piv, n, b) };
    for i in 0..n {
        tmp[i] = b[piv[i]];
    }
    for i in 0..n {
        let mut s = tmp[i];
        for j in 0..i {
            s -= a[i * n + j] * tmp[j];
        }
        tmp[i] = s;
    }
    for i in (0..n).rev() {
        let mut s = tmp[i];
        for j in i + 1..n {
            s -= a[i * n + j] * tmp[j];
        }
        tmp[i] = s / a[i * n + i];
    }
    b[..n].copy_from_slice(tmp);
}

fn lu_solve_big(a: &[f64], piv: &[usize], n: usize, b: &mut [f64]) {
    let mut tmp: Vec<f64> = (0..n).map(|i| b[piv[i]]).collect();
    for i in 0..n {
        let s: f64 = (0..i).map(|j| a[i * n + j] * tmp[j]).sum();
        tmp[i] -= s;
    }
    for i in (0..n).rev() {
        let s: f64 = (i + 1..n).map(|j| a[i * n + j] * tmp[j]).sum();
        tmp[i] = (tmp[i] - s) / a[i * n + i];
    }
    b[..n].copy_from_slice(&tmp);
}

/// A smooth sign: tanh(v / width), so friction and check valves stay differentiable for the stepper.
pub fn soft_sign(v: f64, width: f64) -> f64 {
    (v / width.max(1e-12)).tanh()
}

/// max(v, 0) with a rounded corner `width` wide.
pub fn soft_pos(v: f64, width: f64) -> f64 {
    let w = width.max(1e-12);
    0.5 * (v + (v * v + w * w).sqrt()) - 0.5 * w
}

/// A smooth 0 to 1 step centred on `at`, `width` wide.
pub fn smooth_step(v: f64, at: f64, width: f64) -> f64 {
    let s = ((v - at) / width.max(1e-12) + 0.5).clamp(0.0, 1.0);
    s * s * (3.0 - 2.0 * s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stiff_decay_is_stable_at_huge_steps() {
        let mut s = Stepper::new(1);
        let mut x = [1.0];
        for _ in 0..100 {
            s.step(&mut x, 0.1, &[1e-3], 1.0, |x, d| d[0] = -1e6 * (x[0] - 0.5));
        }
        assert!((x[0] - 0.5).abs() < 1e-6, "{}", x[0]);
    }

    #[test]
    fn oscillator_keeps_its_energy_when_resolved() {
        let mut s = Stepper::new(2);
        let mut x = [1.0, 0.0];
        let w = 2.0 * std::f64::consts::PI;
        for _ in 0..100 {
            s.step(&mut x, 0.01, &[1e-4, 1e-4], 1.0, |x, d| {
                d[0] = x[1];
                d[1] = -w * w * x[0];
            });
        }
        let e = x[0] * x[0] + (x[1] / w).powi(2);
        assert!((e - 1.0).abs() < 0.05, "energy {e}");
    }

    #[test]
    fn non_finite_rhs_puts_the_state_back() {
        let mut s = Stepper::new(1);
        let mut x = [1.0];
        let o = s.step(&mut x, 0.01, &[1e-3], 1.0, |_, d| d[0] = f64::NAN);
        assert_eq!(o, Outcome::Restored);
        assert_eq!(x[0], 1.0);
    }

    #[test]
    fn lu_solves_a_pivoting_system() {
        let mut a = vec![0.0, 2.0, 1.0, 1.0];
        let mut p = vec![0; 2];
        assert!(lu_decompose(&mut a, &mut p, 2));
        let mut b = vec![4.0, 3.0];
        lu_solve(&a, &p, 2, &mut b);
        assert!((b[0] - 1.0).abs() < 1e-12 && (b[1] - 2.0).abs() < 1e-12, "{b:?}");
    }
}
