//! Complex phasors, three-phase sets and their symmetrical components.

use std::ops::{Add, AddAssign, Div, Mul, MulAssign, Neg, Sub, SubAssign};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct C {
    pub re: f64,
    pub im: f64,
}

pub const ZERO: C = C { re: 0.0, im: 0.0 };
pub const ONE: C = C { re: 1.0, im: 0.0 };
pub const J: C = C { re: 0.0, im: 1.0 };
/// The 120 degree operator.
pub const A: C = C { re: -0.5, im: 0.866_025_403_784_438_6 };
/// a squared, 240 degrees.
pub const A2: C = C { re: -0.5, im: -0.866_025_403_784_438_6 };

impl C {
    pub const fn new(re: f64, im: f64) -> Self {
        C { re, im }
    }
    pub const fn real(re: f64) -> Self {
        C { re, im: 0.0 }
    }
    pub fn polar(r: f64, th: f64) -> Self {
        let (s, c) = th.sin_cos();
        C { re: r * c, im: r * s }
    }
    pub fn abs(self) -> f64 {
        self.re.hypot(self.im)
    }
    pub fn abs2(self) -> f64 {
        self.re * self.re + self.im * self.im
    }
    pub fn arg(self) -> f64 {
        self.im.atan2(self.re)
    }
    pub fn conj(self) -> Self {
        C { re: self.re, im: -self.im }
    }
    pub fn scale(self, k: f64) -> Self {
        C { re: self.re * k, im: self.im * k }
    }
    pub fn inv(self) -> Self {
        let d = self.abs2();
        if d > 0.0 {
            C { re: self.re / d, im: -self.im / d }
        } else {
            ZERO
        }
    }
    pub fn is_finite(self) -> bool {
        self.re.is_finite() && self.im.is_finite()
    }
    pub fn is_zero(self) -> bool {
        self.re == 0.0 && self.im == 0.0
    }
    /// Unit phasor in the same direction, or 1 for zero.
    pub fn unit(self) -> Self {
        let m = self.abs();
        if m > 1e-12 {
            self.scale(1.0 / m)
        } else {
            ONE
        }
    }
}

impl Add for C {
    type Output = C;
    fn add(self, o: C) -> C {
        C { re: self.re + o.re, im: self.im + o.im }
    }
}
impl Sub for C {
    type Output = C;
    fn sub(self, o: C) -> C {
        C { re: self.re - o.re, im: self.im - o.im }
    }
}
impl Mul for C {
    type Output = C;
    fn mul(self, o: C) -> C {
        C { re: self.re * o.re - self.im * o.im, im: self.re * o.im + self.im * o.re }
    }
}
impl Div for C {
    type Output = C;
    fn div(self, o: C) -> C {
        self * o.inv()
    }
}
impl Mul<f64> for C {
    type Output = C;
    fn mul(self, k: f64) -> C {
        self.scale(k)
    }
}
impl Div<f64> for C {
    type Output = C;
    fn div(self, k: f64) -> C {
        self.scale(1.0 / k)
    }
}
impl Neg for C {
    type Output = C;
    fn neg(self) -> C {
        C { re: -self.re, im: -self.im }
    }
}
impl AddAssign for C {
    fn add_assign(&mut self, o: C) {
        self.re += o.re;
        self.im += o.im;
    }
}
impl SubAssign for C {
    fn sub_assign(&mut self, o: C) {
        self.re -= o.re;
        self.im -= o.im;
    }
}
impl MulAssign for C {
    fn mul_assign(&mut self, o: C) {
        *self = *self * o;
    }
}

/// Phase a, b, c values.
pub type Abc = [C; 3];
/// A 3x3 phase-frame admittance or impedance.
pub type M3 = [[C; 3]; 3];

pub const ZERO3: Abc = [ZERO; 3];

/// A positive-sequence set with phase a at `e`: b lags 120 degrees, c leads.
pub fn balanced(e: C) -> Abc {
    [e, e * A2, e * A]
}

/// Zero, positive and negative sequence components of a phase set.
pub fn to_sequence(v: &Abc) -> [C; 3] {
    let third = 1.0 / 3.0;
    [
        (v[0] + v[1] + v[2]).scale(third),
        (v[0] + A * v[1] + A2 * v[2]).scale(third),
        (v[0] + A2 * v[1] + A * v[2]).scale(third),
    ]
}

pub fn from_sequence(s: &[C; 3]) -> Abc {
    [s[0] + s[1] + s[2], s[0] + A2 * s[1] + A * s[2], s[0] + A * s[1] + A2 * s[2]]
}

pub fn diag3(y: C) -> M3 {
    let mut m = [[ZERO; 3]; 3];
    for (i, row) in m.iter_mut().enumerate() {
        row[i] = y;
    }
    m
}

/// Phase-frame matrix of an element given its zero, positive and negative sequence values:
/// T diag(y0, y1, y2) T^-1.
pub fn from_sequence_diag(y0: C, y1: C, y2: C) -> M3 {
    let t = [[ONE, ONE, ONE], [ONE, A2, A], [ONE, A, A2]];
    let ti = [[ONE, ONE, ONE], [ONE, A, A2], [ONE, A2, A]];
    let y = [y0, y1, y2];
    let mut m = [[ZERO; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            let mut s = ZERO;
            for k in 0..3 {
                s += t[i][k] * y[k] * ti[k][j];
            }
            m[i][j] = s.scale(1.0 / 3.0);
        }
    }
    m
}

pub fn mul3(m: &M3, v: &Abc) -> Abc {
    let mut out = ZERO3;
    for i in 0..3 {
        out[i] = m[i][0] * v[0] + m[i][1] * v[1] + m[i][2] * v[2];
    }
    out
}

/// Mean of the phase magnitudes over the first `n` phases.
pub fn mean_abs(v: &Abc, n: usize) -> f64 {
    let n = n.clamp(1, 3);
    v[..n].iter().map(|x| x.abs()).sum::<f64>() / n as f64
}

pub fn max_abs(v: &Abc, n: usize) -> f64 {
    v[..n.clamp(1, 3)].iter().map(|x| x.abs()).fold(0.0, f64::max)
}

pub fn min_abs(v: &Abc, n: usize) -> f64 {
    v[..n.clamp(1, 3)].iter().map(|x| x.abs()).fold(f64::INFINITY, f64::min)
}

/// Complex power V conj(I) summed over `n` phases.
pub fn power(v: &Abc, i: &Abc, n: usize) -> C {
    let mut s = ZERO;
    for k in 0..n.clamp(1, 3) {
        s += v[k] * i[k].conj();
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: C, b: C) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn sequence_round_trips_and_a_balanced_set_is_pure_positive() {
        let v = balanced(C::polar(115.0, 0.3));
        let s = to_sequence(&v);
        assert!(s[0].abs() < 1e-9 && s[2].abs() < 1e-9 && (s[1].abs() - 115.0).abs() < 1e-9);
        let back = from_sequence(&s);
        for k in 0..3 {
            assert!(near(back[k], v[k]));
        }
    }

    #[test]
    fn equal_sequence_admittances_give_a_diagonal_matrix() {
        let y = C::new(2.0, -3.0);
        let m = from_sequence_diag(y, y, y);
        for i in 0..3 {
            for j in 0..3 {
                assert!(near(m[i][j], if i == j { y } else { ZERO }), "{i}{j} {:?}", m[i][j]);
            }
        }
    }

    #[test]
    fn a_sequence_matrix_acts_on_each_sequence_alone() {
        let (y0, y1, y2) = (C::new(1.0, 0.0), C::new(0.0, -5.0), C::new(0.5, -2.0));
        let m = from_sequence_diag(y0, y1, y2);
        let v = from_sequence(&[C::new(3.0, 1.0), C::new(100.0, 0.0), C::new(-4.0, 2.0)]);
        let i = to_sequence(&mul3(&m, &v));
        let s = to_sequence(&v);
        assert!(near(i[0], y0 * s[0]) && near(i[1], y1 * s[1]) && near(i[2], y2 * s[2]));
    }
}
