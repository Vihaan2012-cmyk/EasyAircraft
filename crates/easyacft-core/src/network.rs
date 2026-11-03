//! Implicit flow-network solver shared by every domain.

pub type NodeId = usize;
pub type LinkId = usize;

/// Closed switches and ideal sources stand in as this conductance.
pub const STIFF: f64 = 1e9;
/// Leak to reference so isolated nodes settle at zero.
const LEAK: f64 = 1e-12;

#[derive(Clone, Debug)]
struct Node {
    capacitance: f64,
}

#[derive(Clone, Debug)]
struct Link {
    a: NodeId,
    b: NodeId,
    resistance: f64,
    inertance: f64,
    closed: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Network {
    nodes: Vec<Node>,
    links: Vec<Link>,
    potential: Vec<f64>,
    flow: Vec<f64>,
    shunt: Vec<f64>,
    inject: Vec<f64>,
    islands: Vec<Vec<NodeId>>,
    islands_dirty: bool,
    last: Vec<Option<Vec<f64>>>,
    pub solves: usize,
    pub skips: usize,
}

impl Network {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_node(&mut self, capacitance: f64) -> NodeId {
        self.nodes.push(Node { capacitance: capacitance.max(0.0) });
        self.potential.push(0.0);
        self.shunt.push(0.0);
        self.inject.push(0.0);
        self.islands_dirty = true;
        self.nodes.len() - 1
    }

    pub fn add_link(&mut self, a: NodeId, b: NodeId, resistance: f64, inertance: f64) -> LinkId {
        self.links.push(Link { a, b, resistance: resistance.max(0.0), inertance: inertance.max(0.0), closed: true });
        self.flow.push(0.0);
        self.islands_dirty = true;
        self.links.len() - 1
    }

    /// A pipe, duct or cable split into `n` equal segments.
    pub fn add_segmented(&mut self, a: NodeId, b: NodeId, n: usize, resistance: f64, inertance: f64, capacitance: f64) -> Vec<LinkId> {
        let n = n.max(1);
        let mut prev = a;
        let mut out = Vec::with_capacity(n);
        for k in 0..n {
            let next = if k + 1 == n { b } else { self.add_node(capacitance / (n - 1).max(1) as f64) };
            out.push(self.add_link(prev, next, resistance / n as f64, inertance / n as f64));
            prev = next;
        }
        out
    }

    pub fn set_closed(&mut self, link: LinkId, closed: bool) {
        if self.links[link].closed != closed {
            self.links[link].closed = closed;
            self.islands_dirty = true;
            if !closed {
                self.flow[link] = 0.0;
            }
        }
    }

    pub fn set_resistance(&mut self, link: LinkId, resistance: f64) {
        let r = resistance.max(0.0);
        if self.links[link].resistance != r {
            self.links[link].resistance = r;
            self.last.iter_mut().for_each(|v| *v = None);
        }
    }

    pub fn potential(&self, node: NodeId) -> f64 {
        self.potential[node]
    }

    pub fn flow(&self, link: LinkId) -> f64 {
        self.flow[link]
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    pub fn link_count(&self) -> usize {
        self.links.len()
    }

    /// Clears the source and load stamps for the next step.
    pub fn begin_step(&mut self) {
        self.shunt.iter_mut().for_each(|v| *v = 0.0);
        self.inject.iter_mut().for_each(|v| *v = 0.0);
    }

    /// Shunt conductance to reference plus an injected flow; ideal source = `(STIFF, STIFF * v)`.
    pub fn stamp(&mut self, node: NodeId, conductance: f64, injection: f64) {
        self.shunt[node] += conductance;
        self.inject[node] += injection;
    }

    fn rebuild_islands(&mut self) {
        let n = self.nodes.len();
        let mut parent: Vec<usize> = (0..n).collect();
        fn find(p: &mut [usize], mut x: usize) -> usize {
            while p[x] != x {
                p[x] = p[p[x]];
                x = p[x];
            }
            x
        }
        for l in self.links.iter().filter(|l| l.closed) {
            let (ra, rb) = (find(&mut parent, l.a), find(&mut parent, l.b));
            if ra != rb {
                parent[ra] = rb;
            }
        }
        let mut groups: std::collections::BTreeMap<usize, Vec<NodeId>> = Default::default();
        for i in 0..n {
            let r = find(&mut parent, i);
            groups.entry(r).or_default().push(i);
        }
        self.islands = groups.into_values().collect();
        self.last = vec![None; self.islands.len()];
        self.islands_dirty = false;
    }

    pub fn island_count(&mut self) -> usize {
        if self.islands_dirty {
            self.rebuild_islands();
        }
        self.islands.len()
    }

    /// Island index of every node.
    pub fn islands_of(&mut self) -> Vec<usize> {
        if self.islands_dirty {
            self.rebuild_islands();
        }
        let mut of = vec![0; self.nodes.len()];
        for (i, island) in self.islands.iter().enumerate() {
            for &n in island {
                of[n] = i;
            }
        }
        of
    }

    pub fn step(&mut self, dt: f64) {
        if self.islands_dirty {
            self.rebuild_islands();
        }
        let dt = dt.max(1e-9);
        let mut index = vec![usize::MAX; self.nodes.len()];
        let mut link_of: Vec<Vec<LinkId>> = vec![Vec::new(); self.islands.len()];
        for island in &self.islands {
            for (k, &n) in island.iter().enumerate() {
                index[n] = k;
            }
        }
        let mut island_of = vec![0usize; self.nodes.len()];
        for (i, island) in self.islands.iter().enumerate() {
            for &n in island {
                island_of[n] = i;
            }
        }
        for (li, l) in self.links.iter().enumerate() {
            if l.closed {
                link_of[island_of[l.a]].push(li);
            }
        }
        for i in 0..self.islands.len() {
            let island = self.islands[i].clone();
            let m = island.len();
            let dynamic = island.iter().any(|&n| self.nodes[n].capacitance > 0.0) || link_of[i].iter().any(|&l| self.links[l].inertance > 0.0);
            let mut key = Vec::with_capacity(2 * m);
            for &n in &island {
                key.push(self.shunt[n]);
                key.push(self.inject[n]);
            }
            if !dynamic && self.last[i].as_ref() == Some(&key) {
                self.skips += 1;
                continue;
            }
            let mut a = vec![0.0; m * m];
            let mut rhs = vec![0.0; m];
            for (k, &n) in island.iter().enumerate() {
                let c = self.nodes[n].capacitance / dt;
                a[k * m + k] += self.shunt[n] + LEAK + c;
                rhs[k] += self.inject[n] + c * self.potential[n];
            }
            let mut coeff = Vec::with_capacity(link_of[i].len());
            for &li in &link_of[i] {
                let l = &self.links[li];
                let lt = l.inertance / dt;
                let denom = lt + l.resistance;
                let (g, c0) = if denom <= 0.0 { (STIFF, 0.0) } else { (1.0 / denom, lt * self.flow[li] / denom) };
                coeff.push((li, g, c0));
                let (pa, pb) = (index[l.a], index[l.b]);
                a[pa * m + pa] += g;
                a[pb * m + pb] += g;
                a[pa * m + pb] -= g;
                a[pb * m + pa] -= g;
                rhs[pa] -= c0;
                rhs[pb] += c0;
            }
            solve(&mut a, &mut rhs, m);
            for (k, &n) in island.iter().enumerate() {
                self.potential[n] = rhs[k];
            }
            for (li, g, c0) in coeff {
                let l = &self.links[li];
                self.flow[li] = g * (self.potential[l.a] - self.potential[l.b]) + c0;
            }
            self.last[i] = if dynamic { None } else { Some(key) };
            self.solves += 1;
        }
    }

    /// Potentials then flows, for snapshots.
    pub fn state(&self) -> Vec<f64> {
        self.potential.iter().chain(self.flow.iter()).copied().collect()
    }

    pub fn restore(&mut self, state: &[f64]) -> bool {
        let (n, l) = (self.potential.len(), self.flow.len());
        if state.len() != n + l {
            return false;
        }
        self.potential.copy_from_slice(&state[..n]);
        self.flow.copy_from_slice(&state[n..]);
        self.last.iter_mut().for_each(|v| *v = None);
        true
    }
}

/// Gaussian elimination with partial pivoting; result in `b`.
fn solve(a: &mut [f64], b: &mut [f64], m: usize) {
    for col in 0..m {
        let pivot = (col..m).max_by(|&x, &y| a[x * m + col].abs().total_cmp(&a[y * m + col].abs())).unwrap_or(col);
        if pivot != col {
            for k in 0..m {
                a.swap(col * m + k, pivot * m + k);
            }
            b.swap(col, pivot);
        }
        let d = a[col * m + col];
        if d.abs() < 1e-300 {
            continue;
        }
        for row in col + 1..m {
            let f = a[row * m + col] / d;
            if f != 0.0 {
                for k in col..m {
                    a[row * m + k] -= f * a[col * m + k];
                }
                b[row] -= f * b[col];
            }
        }
    }
    for col in (0..m).rev() {
        let mut s = b[col];
        for k in col + 1..m {
            s -= a[col * m + k] * b[k];
        }
        let d = a[col * m + col];
        b[col] = if d.abs() < 1e-300 { 0.0 } else { s / d };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_source_and_two_loads_divide_like_ohms_law() {
        let mut n = Network::new();
        let bus = n.add_node(0.0);
        let load = n.add_node(0.0);
        let feeder = n.add_link(bus, load, 0.1, 0.0);
        n.begin_step();
        n.stamp(bus, STIFF, STIFF * 28.0);
        n.stamp(load, 1.0 / 2.7, 0.0);
        n.step(0.01);
        assert!((n.potential(load) - 28.0 * 2.7 / 2.8).abs() < 1e-6);
        assert!((n.flow(feeder) - 10.0).abs() < 1e-6);
    }

    #[test]
    fn an_open_switch_splits_the_network_and_the_far_side_goes_dead() {
        let mut n = Network::new();
        let bus = n.add_node(0.0);
        let far = n.add_node(0.0);
        let sw = n.add_link(bus, far, 0.0, 0.0);
        let run = |n: &mut Network| {
            n.begin_step();
            n.stamp(bus, STIFF, STIFF * 115.0);
            n.stamp(far, 0.5, 0.0);
            n.step(0.01);
        };
        run(&mut n);
        assert!((n.potential(far) - 115.0).abs() < 1e-3);
        assert_eq!(n.island_count(), 1);
        n.set_closed(sw, false);
        run(&mut n);
        assert_eq!(n.island_count(), 2);
        assert!(n.potential(far).abs() < 1e-9);
        assert_eq!(n.flow(sw), 0.0);
    }

    #[test]
    fn an_unchanged_static_island_is_not_solved_again() {
        let mut n = Network::new();
        let a = n.add_node(0.0);
        let b = n.add_node(0.0);
        n.add_link(a, b, 1.0, 0.0);
        for _ in 0..10 {
            n.begin_step();
            n.stamp(a, STIFF, STIFF * 10.0);
            n.stamp(b, 1.0, 0.0);
            n.step(0.01);
        }
        assert_eq!(n.solves, 1);
        assert_eq!(n.skips, 9);
    }

    #[test]
    fn a_capacitor_charges_with_its_time_constant() {
        let (r, c) = (10.0, 0.1);
        let mut n = Network::new();
        let src = n.add_node(0.0);
        let cap = n.add_node(c);
        n.add_link(src, cap, r, 0.0);
        let dt = 1e-4;
        for _ in 0..(r * c / dt) as usize {
            n.begin_step();
            n.stamp(src, STIFF, STIFF * 1.0);
            n.step(dt);
        }
        let expected = 1.0 - (-1.0f64).exp();
        assert!((n.potential(cap) - expected).abs() < 2e-3, "{}", n.potential(cap));
    }

    #[test]
    fn a_segmented_line_delays_a_step_and_carries_a_wave() {
        let mut n = Network::new();
        let inlet = n.add_node(0.0);
        let outlet = n.add_node(0.05);
        let segs = n.add_segmented(inlet, outlet, 20, 0.05, 0.05, 1.0);
        assert_eq!(segs.len(), 20);
        assert_eq!(n.node_count(), 2 + 19);
        let dt = 1e-3;
        let mut first_rise = None;
        let mut peak: f64 = 0.0;
        for k in 0..20_000 {
            n.begin_step();
            n.stamp(inlet, STIFF, STIFF * 100.0);
            n.step(dt);
            let p = n.potential(outlet);
            peak = peak.max(p);
            if first_rise.is_none() && p > 1.0 {
                first_rise = Some(k as f64 * dt);
            }
        }
        let t = first_rise.expect("the step reached the outlet");
        assert!(t > 0.15, "the outlet rose after only {t:.3} s");
        assert!(peak > 100.0, "an inertive line overshoots, peak {peak:.1}");
        assert!((n.potential(outlet) - 100.0).abs() < 5.0, "settles near the inlet, {:.1}", n.potential(outlet));
    }

    #[test]
    fn state_round_trips() {
        let mut n = Network::new();
        let a = n.add_node(1.0);
        let b = n.add_node(0.0);
        n.add_link(a, b, 1.0, 0.1);
        n.begin_step();
        n.stamp(a, 1.0, 5.0);
        n.step(0.1);
        let s = n.state();
        let mut m = n.clone();
        m.begin_step();
        m.stamp(a, 1.0, 50.0);
        m.step(0.1);
        assert!(m.restore(&s));
        assert_eq!(m.state(), s);
        assert!(!m.restore(&s[1..]));
    }
}
