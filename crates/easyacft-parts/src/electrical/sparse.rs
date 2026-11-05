//! Sparse LU for the nodal admittance matrix. The pattern is fixed when the network is built: every link that
//! can ever close is in it. A minimum-degree ordering keeps fill low (a bus with a hundred breakers eliminates
//! its leaves first), the symbolic pass turns the factor into flat index lists, and each step's numeric factor
//! and solve are straight passes over them.

use super::complex::{C, ZERO};
use std::collections::HashMap;

#[derive(Clone, Debug, Default)]
pub struct Sparse {
    n: usize,
    /// Unknown eliminated at each position.
    perm: Vec<u32>,
    diag: Vec<u32>,
    lo_start: Vec<u32>,
    lo_pos: Vec<u32>,
    lo_slot: Vec<u32>,
    up_slot: Vec<u32>,
    /// Slot (i, j) updated by each pair in a pivot's column, row by row.
    upd: Vec<u32>,
    map: HashMap<(u32, u32), u32>,
    pub vals: Vec<C>,
    y: Vec<C>,
}

fn insert(v: &mut Vec<u32>, x: u32) {
    if let Err(k) = v.binary_search(&x) {
        v.insert(k, x);
    }
}

fn remove(v: &mut Vec<u32>, x: u32) {
    if let Ok(k) = v.binary_search(&x) {
        v.remove(k);
    }
}

impl Sparse {
    pub fn build(n: usize, entries: &[(u32, u32)]) -> Self {
        let mut adj: Vec<Vec<u32>> = vec![Vec::new(); n];
        for &(i, j) in entries {
            if i != j && (i as usize) < n && (j as usize) < n {
                insert(&mut adj[i as usize], j);
                insert(&mut adj[j as usize], i);
            }
        }
        // Minimum degree, ties to the lowest index so the ordering is repeatable.
        let mut done = vec![false; n];
        let mut order = Vec::with_capacity(n);
        let mut clique: Vec<Vec<u32>> = vec![Vec::new(); n];
        let mut degree: Vec<usize> = adj.iter().map(|a| a.len()).collect();
        for _ in 0..n {
            let mut best = usize::MAX;
            let mut v = 0;
            for x in 0..n {
                if !done[x] && degree[x] < best {
                    best = degree[x];
                    v = x;
                    if best == 0 {
                        break;
                    }
                }
            }
            done[v] = true;
            let nb = std::mem::take(&mut adj[v]);
            for &a in &nb {
                remove(&mut adj[a as usize], v as u32);
            }
            for (ia, &a) in nb.iter().enumerate() {
                for &b in &nb[ia + 1..] {
                    insert(&mut adj[a as usize], b);
                    insert(&mut adj[b as usize], a);
                }
            }
            for &a in &nb {
                degree[a as usize] = adj[a as usize].len();
            }
            clique[v] = nb;
            order.push(v as u32);
        }
        let mut pos = vec![0u32; n];
        for (k, &u) in order.iter().enumerate() {
            pos[u as usize] = k as u32;
        }
        let mut s = Sparse { n, perm: order, ..Default::default() };
        let mut next = 0u32;
        let mut slot = |map: &mut HashMap<(u32, u32), u32>, i: u32, j: u32| -> u32 {
            *map.entry((i, j)).or_insert_with(|| {
                next += 1;
                next - 1
            })
        };
        for k in 0..n {
            let u = s.perm[k];
            s.diag.push(slot(&mut s.map, u, u));
        }
        s.lo_start.push(0);
        for k in 0..n {
            let u = s.perm[k];
            let mut later: Vec<u32> = clique[u as usize].iter().map(|&x| pos[x as usize]).collect();
            later.sort_unstable();
            for &p in &later {
                let w = s.perm[p as usize];
                let lo = slot(&mut s.map, w, u);
                let up = slot(&mut s.map, u, w);
                s.lo_pos.push(p);
                s.lo_slot.push(lo);
                s.up_slot.push(up);
            }
            s.lo_start.push(s.lo_pos.len() as u32);
        }
        for k in 0..n {
            let (a, b) = (s.lo_start[k] as usize, s.lo_start[k + 1] as usize);
            for t in a..b {
                let wi = s.perm[s.lo_pos[t] as usize];
                for r in a..b {
                    let wj = s.perm[s.lo_pos[r] as usize];
                    let at = s.map[&(wi, wj)];
                    s.upd.push(at);
                }
            }
        }
        s.vals = vec![ZERO; next as usize];
        s.y = vec![ZERO; n];
        s
    }

    pub fn nonzeros(&self) -> usize {
        self.vals.len()
    }

    /// Slot of entry (row, column) in unknown numbering, if it is in the pattern.
    pub fn slot(&self, i: usize, j: usize) -> Option<u32> {
        self.map.get(&(i as u32, j as u32)).copied()
    }

    pub fn clear(&mut self) {
        self.vals.iter_mut().for_each(|v| *v = ZERO);
    }

    pub fn add_diagonal(&mut self, v: C) {
        for &d in &self.diag {
            self.vals[d as usize] += v;
        }
    }

    /// Factors in place; a vanishing pivot is replaced by a tiny one so a dead node solves to zero.
    pub fn factor(&mut self) {
        let mut u = 0usize;
        for k in 0..self.n {
            let dk = self.diag[k] as usize;
            if !(self.vals[dk].abs2() > 1e-280) {
                self.vals[dk] = C::real(1e-140);
            }
            let inv = self.vals[dk].inv();
            let (a, b) = (self.lo_start[k] as usize, self.lo_start[k + 1] as usize);
            for t in a..b {
                let s = self.lo_slot[t] as usize;
                self.vals[s] = self.vals[s] * inv;
            }
            let w = b - a;
            for t in a..b {
                let l = self.vals[self.lo_slot[t] as usize];
                if l.is_zero() {
                    u += w;
                    continue;
                }
                for r in a..b {
                    let up = self.vals[self.up_slot[r] as usize];
                    if !up.is_zero() {
                        let at = self.upd[u] as usize;
                        self.vals[at] -= l * up;
                    }
                    u += 1;
                }
            }
        }
    }

    /// Solves with the current factor; `b` holds the right side in unknown numbering and gets the answer.
    pub fn solve(&mut self, b: &mut [C]) {
        let n = self.n;
        for k in 0..n {
            self.y[k] = b[self.perm[k] as usize];
        }
        for k in 0..n {
            let yk = self.y[k];
            if yk.is_zero() {
                continue;
            }
            for t in self.lo_start[k] as usize..self.lo_start[k + 1] as usize {
                let p = self.lo_pos[t] as usize;
                self.y[p] -= self.vals[self.lo_slot[t] as usize] * yk;
            }
        }
        for k in (0..n).rev() {
            let mut s = self.y[k];
            for t in self.lo_start[k] as usize..self.lo_start[k + 1] as usize {
                s -= self.vals[self.up_slot[t] as usize] * self.y[self.lo_pos[t] as usize];
            }
            self.y[k] = s / self.vals[self.diag[k] as usize];
        }
        for k in 0..n {
            b[self.perm[k] as usize] = self.y[k];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dense_solve(a: &[Vec<C>], b: &[C]) -> Vec<C> {
        let n = b.len();
        let mut m: Vec<Vec<C>> = a.to_vec();
        let mut x = b.to_vec();
        for c in 0..n {
            let p = (c..n).max_by(|&i, &j| m[i][c].abs().total_cmp(&m[j][c].abs())).unwrap();
            m.swap(c, p);
            x.swap(c, p);
            for r in c + 1..n {
                let f = m[r][c] / m[c][c];
                for k in c..n {
                    let v = m[c][k];
                    m[r][k] -= f * v;
                }
                let v = x[c];
                x[r] -= f * v;
            }
        }
        for c in (0..n).rev() {
            let mut s = x[c];
            for k in c + 1..n {
                s -= m[c][k] * x[k];
            }
            x[c] = s / m[c][c];
        }
        x
    }

    #[test]
    fn matches_a_dense_solve_on_a_meshed_network() {
        let n = 12;
        let mut edges = vec![];
        for i in 0..n - 1 {
            edges.push((i, i + 1));
        }
        edges.extend([(0, 5), (2, 9), (3, 11), (7, 1)]);
        let mut sp = Sparse::build(n, &edges.iter().map(|&(a, b)| (a as u32, b as u32)).collect::<Vec<_>>());
        let mut dense = vec![vec![ZERO; n]; n];
        for (k, &(a, b)) in edges.iter().enumerate() {
            let y = C::new(1.0 + k as f64 * 0.3, -2.0 + 0.1 * k as f64);
            for (i, j, v) in [(a, a, y), (b, b, y), (a, b, -y), (b, a, -y)] {
                dense[i][j] += v;
                let s = sp.slot(i, j).unwrap();
                sp.vals[s as usize] += v;
            }
        }
        for i in 0..n {
            let y = C::new(0.05 * (i + 1) as f64, 0.0);
            dense[i][i] += y;
            let at = sp.slot(i, i).unwrap() as usize;
            sp.vals[at] += y;
        }
        let b: Vec<C> = (0..n).map(|i| C::new(i as f64, 1.0 - i as f64)).collect();
        let want = dense_solve(&dense, &b);
        sp.factor();
        let mut got = b.clone();
        sp.solve(&mut got);
        for i in 0..n {
            assert!((got[i] - want[i]).abs() < 1e-9, "{i}: {:?} vs {:?}", got[i], want[i]);
        }
    }

    #[test]
    fn a_star_of_leaves_has_no_fill() {
        let n = 101;
        let edges: Vec<(u32, u32)> = (1..n as u32).map(|i| (0, i)).collect();
        let sp = Sparse::build(n, &edges);
        assert_eq!(sp.nonzeros(), n + 2 * (n - 1));
    }

    #[test]
    fn an_isolated_node_solves_to_zero() {
        let mut sp = Sparse::build(2, &[(0, 1)]);
        let s = sp.slot(0, 0).unwrap();
        sp.vals[s as usize] = C::real(2.0);
        sp.factor();
        let mut b = vec![C::real(4.0), ZERO];
        sp.solve(&mut b);
        assert!((b[0].re - 2.0).abs() < 1e-12 && b[1].abs() < 1e-12);
    }
}
