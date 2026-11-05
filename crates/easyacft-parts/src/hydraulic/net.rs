//! A small liquid network for one hydraulic circuit: nodes store fluid in their compliance, links carry it with
//! resistance and inertance, so a valve slamming shut or a pump starting sends a pressure wave down the line.
//! Stepped implicitly: unconditionally stable, and at millisecond substeps a wave still rings a few times.

const THETA: f64 = 1.0;

#[derive(Clone, Debug, Default)]
pub struct Link {
    pub a: usize,
    pub b: usize,
    /// Linear resistance this substep, Pa s/m3, from the caller's linearisation of its pressure drop.
    pub r: f64,
    /// Inertance, Pa s2/m3: fluid density times length over area.
    pub l: f64,
    /// Flow from a to b, m3/s.
    pub q: f64,
    /// One way only, a to b: shuts when flow would reverse.
    pub check: bool,
    pub open: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Net {
    pub p: Vec<f64>,
    /// Compliance of each node, m3/Pa.
    pub cap: Vec<f64>,
    pub links: Vec<Link>,
    g: Vec<f64>,
    inj: Vec<f64>,
    g_old: Vec<f64>,
    inj_old: Vec<f64>,
    a: Vec<f64>,
    rhs: Vec<f64>,
}

impl Net {
    pub fn node(&mut self, cap: f64) -> usize {
        self.p.push(0.0);
        self.cap.push(cap.max(1e-16));
        for v in [&mut self.g, &mut self.inj, &mut self.g_old, &mut self.inj_old] {
            v.push(0.0);
        }
        self.p.len() - 1
    }

    pub fn link(&mut self, a: usize, b: usize, r: f64, l: f64, check: bool) -> usize {
        self.links.push(Link { a, b, r, l, q: 0.0, check, open: true });
        self.links.len() - 1
    }

    pub fn len(&self) -> usize {
        self.p.len()
    }

    /// Clears this substep's sinks and sources, keeping the last ones for the theta weighting.
    pub fn begin(&mut self) {
        std::mem::swap(&mut self.g, &mut self.g_old);
        std::mem::swap(&mut self.inj, &mut self.inj_old);
        self.g.iter_mut().for_each(|v| *v = 0.0);
        self.inj.iter_mut().for_each(|v| *v = 0.0);
    }

    /// A conductance to the return line and a flow injected, at a node.
    pub fn stamp(&mut self, n: usize, g: f64, inj: f64) {
        self.g[n] += g;
        self.inj[n] += inj;
    }

    /// One substep of `h`; false when the answer is not finite.
    pub fn step(&mut self, h: f64) -> bool {
        let n = self.p.len();
        if n == 0 {
            return true;
        }
        self.a.clear();
        self.a.resize(n * n, 0.0);
        self.rhs.clear();
        self.rhs.resize(n, 0.0);
        for i in 0..n {
            let c = self.cap[i] / h;
            self.a[i * n + i] += c + THETA * self.g[i] + 1e-14;
            self.rhs[i] += c * self.p[i] + THETA * self.inj[i] + (1.0 - THETA) * (self.inj_old[i] - self.g_old[i] * self.p[i]);
        }
        // Each link: q+ = k (pa+ - pb+) + c0, from L dq/dt = dp - R q weighted by theta.
        let mut coeff = Vec::with_capacity(self.links.len());
        for l in &mut self.links {
            let dp = self.p[l.a] - self.p[l.b];
            if l.check && l.open && l.q <= 0.0 && dp < 0.0 {
                l.open = false;
            } else if l.check && !l.open && dp > 0.0 {
                l.open = true;
            }
            if !l.open {
                l.q = 0.0;
                coeff.push((0.0, 0.0));
                continue;
            }
            let lh = l.l / h;
            let den = lh + THETA * l.r;
            let (k, c0) = if den <= 0.0 { (1e3, 0.0) } else { (THETA / den, (lh * l.q + (1.0 - THETA) * (dp - l.r * l.q)) / den) };
            coeff.push((k, c0));
            let (a, b) = (l.a, l.b);
            self.a[a * n + a] += k;
            self.a[b * n + b] += k;
            self.a[a * n + b] -= k;
            self.a[b * n + a] -= k;
            self.rhs[a] -= c0;
            self.rhs[b] += c0;
        }
        if !solve(&mut self.a, &mut self.rhs, n) {
            return false;
        }
        self.p.copy_from_slice(&self.rhs);
        for (l, &(k, c0)) in self.links.iter_mut().zip(&coeff) {
            if l.open {
                l.q = k * (self.p[l.a] - self.p[l.b]) + c0;
                if l.check && l.q < 0.0 {
                    l.q = 0.0;
                    l.open = false;
                }
            }
        }
        self.p.iter().all(|v| v.is_finite()) && self.links.iter().all(|l| l.q.is_finite())
    }

    pub fn state(&self, v: &mut Vec<f64>) {
        v.extend_from_slice(&self.p);
        v.extend(self.links.iter().flat_map(|l| [l.q, l.open as u8 as f64]));
    }

    pub fn restore(&mut self, it: &mut dyn Iterator<Item = f64>) {
        for p in &mut self.p {
            *p = it.next().unwrap_or(0.0);
        }
        for l in &mut self.links {
            l.q = it.next().unwrap_or(0.0);
            l.open = it.next().unwrap_or(1.0) != 0.0;
        }
    }
}

/// Gaussian elimination with partial pivoting; false on a singular matrix.
fn solve(a: &mut [f64], b: &mut [f64], n: usize) -> bool {
    for c in 0..n {
        let mut best = c;
        for r in c + 1..n {
            if a[r * n + c].abs() > a[best * n + c].abs() {
                best = r;
            }
        }
        if !(a[best * n + c].abs() > 1e-300) {
            return false;
        }
        if best != c {
            for k in 0..n {
                a.swap(c * n + k, best * n + k);
            }
            b.swap(c, best);
        }
        let d = a[c * n + c];
        for r in c + 1..n {
            let f = a[r * n + c] / d;
            if f != 0.0 {
                for k in c..n {
                    a[r * n + k] -= f * a[c * n + k];
                }
                b[r] -= f * b[c];
            }
        }
    }
    for c in (0..n).rev() {
        let mut s = b[c];
        for k in c + 1..n {
            s -= a[c * n + k] * b[k];
        }
        b[c] = s / a[c * n + c];
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_node_charges_through_a_resistance() {
        let mut n = Net::default();
        let a = n.node(1e-10);
        let b = n.node(1e-10);
        n.link(a, b, 1e10, 0.0, false);
        for _ in 0..8000 {
            n.begin();
            n.stamp(a, 1.0, 1.0 * 2e7);
            n.step(1e-3);
        }
        assert!((n.p[b] - 2e7).abs() < 1e5, "{}", n.p[b]);
    }

    #[test]
    fn a_check_valve_holds_pressure_behind_it() {
        let mut n = Net::default();
        let src = n.node(1e-10);
        let acc = n.node(1e-9);
        n.link(src, acc, 1e9, 0.0, true);
        for _ in 0..6000 {
            n.begin();
            n.stamp(src, 1e-6, 1e-6 * 2e7);
            n.step(1e-3);
        }
        let held = n.p[acc];
        for _ in 0..500 {
            n.begin();
            n.stamp(src, 1e-6, 0.0);
            n.step(1e-3);
        }
        assert!(held > 1.5e7 && (n.p[acc] - held).abs() < 0.005 * held && n.p[src] < 1e6, "{held} {} {}", n.p[acc], n.p[src]);
    }

    #[test]
    fn closing_a_flowing_line_overshoots_then_settles() {
        let mut n = Net::default();
        let up = n.node(1e-13);
        let mid = n.node(5e-13);
        let end = n.node(5e-13);
        n.link(up, mid, 2e9, 1e8, false);
        n.link(mid, end, 2e9, 1e8, false);
        let mut peak: f64 = 0.0;
        let mut flowing = 0.0;
        for k in 0..20000 {
            n.begin();
            n.stamp(up, 1e-5, 1e-5 * 2e7);
            if k < 5000 {
                n.stamp(end, 1e-12, 0.0);
            }
            n.step(1e-4);
            if k == 4999 {
                flowing = n.p[end];
            }
            if k >= 5000 {
                peak = peak.max(n.p[end]);
            }
        }
        assert!(flowing < 1.995e7 && peak > 2.0e7 + 5e4 && (n.p[end] - 2e7).abs() < 2e4, "flowing {flowing:.4e}, peak {peak:.4e}, settled {:.4e}", n.p[end]);
    }
}
