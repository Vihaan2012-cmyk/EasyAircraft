//! The electrical network as one nodal admittance system: a DC bus is one unknown, a three-phase AC bus three
//! (phase to neutral), a single-phase AC bus one. Elements ask for the matrix entries they will ever use while
//! the network is built, then stamp straight into their slots each step.

use super::complex::{Abc, C, M3, ZERO, ZERO3};
use super::sparse::Sparse;

/// Conductance from every unknown to structure, so a dead node settles at zero.
const LEAK: f64 = 1e-9;

/// Where an element connects: the unknowns of the phases it touches.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Term {
    pub u: [u32; 3],
    pub n: u8,
}

impl Term {
    pub fn phases(&self) -> usize {
        self.n as usize
    }
    /// Just one phase of this terminal.
    pub fn phase(&self, k: usize) -> Term {
        Term { u: [self.u[k.min(self.phases() - 1)], 0, 0], n: 1 }
    }
}

/// First of an element's requested entries.
#[derive(Clone, Copy, Debug, Default)]
pub struct H(u32);

#[derive(Default)]
pub struct Grid {
    n: usize,
    want: Vec<(u32, u32)>,
    slots: Vec<u32>,
    pub sp: Sparse,
    pub rhs: Vec<C>,
    pub x: Vec<C>,
    built: bool,
    pub factors: u64,
}

impl Grid {
    pub fn unknowns(&mut self, k: usize) -> Term {
        let base = self.n as u32;
        self.n += k;
        let mut u = [base; 3];
        for (i, x) in u.iter_mut().enumerate().take(k) {
            *x = base + i as u32;
        }
        Term { u, n: k as u8 }
    }

    pub fn size(&self) -> usize {
        self.n
    }

    pub fn request(&mut self, e: &[(u32, u32)]) -> H {
        let h = H(self.want.len() as u32);
        self.want.extend_from_slice(e);
        h
    }

    pub fn finish(&mut self) {
        let mut all = self.want.clone();
        all.extend((0..self.n as u32).map(|i| (i, i)));
        self.sp = Sparse::build(self.n, &all);
        self.slots = self.want.iter().map(|&(i, j)| self.sp.slot(i as usize, j as usize).expect("requested entry")).collect();
        self.rhs = vec![ZERO; self.n];
        self.x = vec![ZERO; self.n];
        self.built = true;
    }

    #[inline]
    pub fn add(&mut self, h: H, k: usize, v: C) {
        let s = self.slots[h.0 as usize + k] as usize;
        self.sp.vals[s] += v;
    }

    #[inline]
    pub fn inject(&mut self, u: u32, i: C) {
        self.rhs[u as usize] += i;
    }

    pub fn begin(&mut self) {
        self.sp.clear();
        self.rhs.iter_mut().for_each(|v| *v = ZERO);
    }

    /// Factors and solves; false when the answer is not finite.
    pub fn solve(&mut self) -> bool {
        if !self.built {
            return true;
        }
        self.sp.add_diagonal(C::real(LEAK));
        self.sp.factor();
        self.factors += 1;
        self.x.copy_from_slice(&self.rhs);
        self.sp.solve(&mut self.x);
        self.x.iter().all(|v| v.is_finite())
    }

    /// Solves again with the same factor after the right side changed.
    pub fn resolve(&mut self) -> bool {
        if !self.built {
            return true;
        }
        self.x.copy_from_slice(&self.rhs);
        self.sp.solve(&mut self.x);
        self.x.iter().all(|v| v.is_finite())
    }

    pub fn v(&self, t: &Term) -> Abc {
        let mut out = ZERO3;
        for k in 0..t.phases() {
            out[k] = self.x.get(t.u[k] as usize).copied().unwrap_or(ZERO);
        }
        out
    }

    pub fn v1(&self, u: u32) -> C {
        self.x.get(u as usize).copied().unwrap_or(ZERO)
    }

    pub fn state(&self) -> Vec<f64> {
        self.x.iter().flat_map(|c| [c.re, c.im]).collect()
    }

    pub fn restore(&mut self, s: &[f64]) {
        for (k, v) in self.x.iter_mut().enumerate() {
            *v = C::new(s.get(2 * k).copied().unwrap_or(0.0), s.get(2 * k + 1).copied().unwrap_or(0.0));
        }
    }
}

/// An element from one terminal to structure: a full n x n block, so sequence-coupled sources fit.
#[derive(Clone, Copy, Debug, Default)]
pub struct Shunt {
    pub t: Term,
    h: H,
}

impl Shunt {
    pub fn new(g: &mut Grid, t: Term) -> Self {
        let n = t.phases();
        let mut e = Vec::with_capacity(n * n);
        for i in 0..n {
            for j in 0..n {
                e.push((t.u[i], t.u[j]));
            }
        }
        Shunt { t, h: g.request(&e) }
    }

    pub fn diag(&self, g: &mut Grid, y: &Abc) {
        let n = self.t.phases();
        for k in 0..n {
            if !y[k].is_zero() {
                g.add(self.h, k * n + k, y[k]);
            }
        }
    }

    pub fn each(&self, g: &mut Grid, y: C) {
        self.diag(g, &[y; 3]);
    }

    pub fn full(&self, g: &mut Grid, m: &M3) {
        let n = self.t.phases();
        for i in 0..n {
            for j in 0..n {
                if !m[i][j].is_zero() {
                    g.add(self.h, i * n + j, m[i][j]);
                }
            }
        }
    }

    /// Between two phases of this terminal, as for a phase-to-phase fault.
    pub fn across(&self, g: &mut Grid, a: usize, b: usize, y: C) {
        let n = self.t.phases();
        if a >= n || b >= n || a == b {
            return;
        }
        g.add(self.h, a * n + a, y);
        g.add(self.h, b * n + b, y);
        g.add(self.h, a * n + b, -y);
        g.add(self.h, b * n + a, -y);
    }

    pub fn inject(&self, g: &mut Grid, i: &Abc) {
        for k in 0..self.t.phases() {
            g.inject(self.t.u[k], i[k]);
        }
    }

    pub fn v(&self, g: &Grid) -> Abc {
        g.v(&self.t)
    }
}

/// An element between two terminals, phase to matching phase.
#[derive(Clone, Copy, Debug, Default)]
pub struct Series {
    pub a: Term,
    pub b: Term,
    h: H,
}

impl Series {
    pub fn new(g: &mut Grid, a: Term, b: Term) -> Self {
        let n = a.phases().min(b.phases());
        let (a, b) = (Term { n: n as u8, ..a }, Term { n: n as u8, ..b });
        let mut e = Vec::with_capacity(4 * n);
        for k in 0..n {
            let (x, y) = (a.u[k], b.u[k]);
            e.extend([(x, x), (x, y), (y, x), (y, y)]);
        }
        Series { a, b, h: g.request(&e) }
    }

    pub fn phases(&self) -> usize {
        self.a.phases()
    }

    /// General two-port stamp on phase `k`.
    pub fn stamp(&self, g: &mut Grid, k: usize, yaa: C, yab: C, yba: C, ybb: C) {
        let o = 4 * k;
        g.add(self.h, o, yaa);
        g.add(self.h, o + 1, yab);
        g.add(self.h, o + 2, yba);
        g.add(self.h, o + 3, ybb);
    }

    pub fn series(&self, g: &mut Grid, y: &Abc) {
        for k in 0..self.phases() {
            if !y[k].is_zero() {
                self.stamp(g, k, y[k], -y[k], -y[k], y[k]);
            }
        }
    }

    /// A source in the branch pushing current from a to b: i = y (va - vb - e) leaves a.
    pub fn emf(&self, g: &mut Grid, y: &Abc, e: &Abc) {
        for k in 0..self.phases() {
            let i = y[k] * e[k];
            g.inject(self.a.u[k], i);
            g.inject(self.b.u[k], -i);
        }
    }

    pub fn va(&self, g: &Grid) -> Abc {
        g.v(&self.a)
    }

    pub fn vb(&self, g: &Grid) -> Abc {
        g.v(&self.b)
    }

    /// Current from a to b through a series admittance `y` with emf `e`.
    pub fn current(&self, g: &Grid, y: &Abc, e: &Abc) -> Abc {
        let (va, vb) = (self.va(g), self.vb(g));
        let mut out = ZERO3;
        for k in 0..self.phases() {
            out[k] = y[k] * (va[k] - vb[k] - e[k]);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_three_phase_source_feeds_a_load_through_a_feeder() {
        let mut g = Grid::default();
        let a = g.unknowns(3);
        let b = g.unknowns(3);
        let src = Shunt::new(&mut g, a);
        let load = Shunt::new(&mut g, b);
        let line = Series::new(&mut g, a, b);
        g.finish();
        g.begin();
        let ys = C::real(1e4);
        src.each(&mut g, ys);
        let e = super::super::complex::balanced(C::real(115.0));
        src.inject(&mut g, &[e[0] * ys, e[1] * ys, e[2] * ys]);
        line.series(&mut g, &[C::new(0.0, -10.0); 3]);
        load.each(&mut g, C::real(1.0 / 1.32));
        assert!(g.solve());
        let vb = g.v(&b);
        for k in 0..3 {
            assert!((vb[k].abs() - 115.0).abs() < 1.0, "{k} {}", vb[k].abs());
        }
        let ang = (vb[1] / vb[0]).arg().to_degrees();
        assert!((ang + 120.0).abs() < 0.01, "b lags a by 120: {ang}");
    }
}
