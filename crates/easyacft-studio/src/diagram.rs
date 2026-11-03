//! Automatic system diagrams: hubs in levels from their sources, attached parts around them.

use easyacft_lang::{Checked, CheckedPart};
use std::collections::{HashMap, VecDeque};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Role {
    Hub,
    Source,
    Sink,
}

#[derive(Clone, Debug)]
pub struct Item {
    pub part: usize,
    pub role: Role,
    pub pos: (f32, f32),
    pub size: (f32, f32),
}

impl Item {
    pub fn top(&self) -> f32 {
        self.pos.1 - self.size.1 / 2.0
    }
    pub fn bottom(&self) -> f32 {
        self.pos.1 + self.size.1 / 2.0
    }
}

#[derive(Clone, Debug)]
pub struct Edge {
    pub part: usize,
    pub path: Vec<(f32, f32)>,
    pub mark: (f32, f32),
    pub vertical: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Diagram {
    pub items: Vec<Item>,
    pub edges: Vec<Edge>,
    pub links: Vec<(usize, [(f32, f32); 2])>,
}

pub const DOMAINS: &[&str] = &["electrical", "hydraulic", "fuel", "pneumatic", "engine", "cockpit", "computer"];

/// Approximate width of a label at the diagram's 11 px font.
pub fn text_w(s: &str) -> f32 {
    s.chars().count() as f32 * 6.4
}

pub fn role(p: &CheckedPart) -> Option<(Role, &'static str)> {
    Some(match (p.domain, p.kind) {
        ("electrical", "bus") | ("hydraulic", "circuit") | ("fuel", "line") | ("fuel", "tank") | ("pneumatic", "duct") => (Role::Hub, ""),
        ("electrical", "battery" | "generator" | "ext_power") => (Role::Source, "bus"),
        ("electrical", "load") => (Role::Sink, "bus"),
        ("hydraulic", "pump" | "accumulator") => (Role::Source, "circuit"),
        ("hydraulic", "consumer") => (Role::Sink, "circuit"),
        ("fuel", "feed") => (Role::Sink, "line"),
        ("pneumatic", "source") => (Role::Source, "duct"),
        ("pneumatic", "consumer") => (Role::Sink, "duct"),
        _ => return None,
    })
}

pub fn edge_ends(p: &CheckedPart) -> Option<(usize, usize)> {
    match (p.domain, p.kind) {
        ("electrical", "contactor" | "breaker") | ("hydraulic", "ptu") | ("fuel", "valve") | ("pneumatic", "valve") => {
            let e = p.parts("between");
            Some((*e.first()?, *e.get(1)?))
        }
        ("electrical", "tru" | "inverter") => Some((p.part("input")?, p.part("output")?)),
        ("fuel", "pump") => Some((p.part("tank")?, p.part("outlet")?)),
        _ => None,
    }
}

struct Node {
    hub: Option<usize>,
    w: f32,
}

/// Orders a row by where its neighbours sit, then pulls same-row partners next to each other.
fn order(row: &mut Vec<usize>, x: &[f32], adj: &[(usize, usize)], same: &[(usize, usize)]) {
    let bary: HashMap<usize, f32> = row
        .iter()
        .filter_map(|&n| {
            let xs: Vec<f32> = adj.iter().filter_map(|&(a, b)| if a == n { Some(x[b]) } else if b == n { Some(x[a]) } else { None }).collect();
            (!xs.is_empty()).then(|| (n, xs.iter().sum::<f32>() / xs.len() as f32))
        })
        .collect();
    row.sort_by(|a, b| bary.get(a).unwrap_or(&x[*a]).total_cmp(bary.get(b).unwrap_or(&x[*b])));
    for &(a, b) in same {
        let (Some(ia), Some(ib)) = (row.iter().position(|&n| n == a), row.iter().position(|&n| n == b)) else { continue };
        if ia.abs_diff(ib) > 1 {
            let moved = row.remove(ib);
            let ia = row.iter().position(|&n| n == a).unwrap();
            row.insert(if ib > ia { ia + 1 } else { ia }, moved);
        }
    }
}

pub fn build(c: &Checked, domain: &str, pinned: &HashMap<String, (f32, f32)>) -> Diagram {
    let mut d = Diagram::default();
    let parts: Vec<usize> = (0..c.parts.len()).filter(|&i| c.parts[i].domain == domain).collect();
    let hubs: Vec<usize> = parts.iter().copied().filter(|&i| role(&c.parts[i]).is_some_and(|r| r.0 == Role::Hub)).collect();
    let ends: Vec<(usize, usize, usize)> = parts.iter().filter_map(|&i| edge_ends(&c.parts[i]).map(|(a, b)| (i, a, b))).filter(|(_, a, b)| hubs.contains(a) && hubs.contains(b) && a != b).collect();
    let mut attached: HashMap<(usize, bool), Vec<usize>> = HashMap::new();
    for &i in &parts {
        let Some((r, field)) = role(&c.parts[i]) else { continue };
        if r == Role::Hub {
            continue;
        }
        if let Some(h) = c.parts[i].part(field).filter(|h| hubs.contains(h)) {
            attached.entry((h, r == Role::Source)).or_default().push(i);
        }
    }

    let mut level: HashMap<usize, usize> = HashMap::new();
    let mut q = VecDeque::new();
    for &h in &hubs {
        if attached.contains_key(&(h, true)) || c.parts[h].kind == "tank" {
            level.insert(h, 0);
            q.push_back(h);
        }
    }
    while let Some(h) = q.pop_front() {
        let l = level[&h];
        for &(_, a, b) in &ends {
            for (x, y) in [(a, b), (b, a)] {
                if x == h && !level.contains_key(&y) {
                    level.insert(y, l + 1);
                    q.push_back(y);
                }
            }
        }
    }
    let bfs = level.clone();
    let forward: Vec<(usize, usize)> = ends
        .iter()
        .filter_map(|&(i, a, b)| {
            let conv = matches!((c.parts[i].domain, c.parts[i].kind), ("electrical", "tru" | "inverter") | ("fuel", "pump"));
            match (bfs.get(&a), bfs.get(&b)) {
                _ if conv => Some((a, b)),
                (Some(x), Some(y)) if x < y => Some((a, b)),
                (Some(x), Some(y)) if y < x => Some((b, a)),
                _ => None,
            }
        })
        .collect();
    for _ in 0..hubs.len() {
        let mut changed = false;
        for &(a, b) in &forward {
            if let (Some(&la), Some(&lb)) = (level.get(&a), level.get(&b)) {
                if lb < la + 1 {
                    level.insert(b, la + 1);
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
    let spare = level.values().copied().max().map_or(0, |m| m + 1);
    for &h in &hubs {
        level.entry(h).or_insert(spare);
    }
    let levels = level.values().copied().max().unwrap_or(0) + 1;

    let child_w = |i: usize| {
        let w = text_w(&c.parts[i].id);
        match role(&c.parts[i]).map(|r| r.0) {
            Some(Role::Source) => (w + 12.0).max(40.0),
            _ => (w + 16.0).max(46.0),
        }
    };
    let side_w = |h: usize, below: bool| attached.get(&(h, below)).map_or(0.0, |l| l.iter().map(|&i| child_w(i) + 12.0).sum::<f32>());
    let degree = |h: usize| ends.iter().filter(|(_, a, b)| *a == h || *b == h).count() as f32;
    let width = |h: usize| (text_w(&c.parts[h].id) + 110.0).max(side_w(h, true).max(side_w(h, false)) + 24.0 + degree(h) * 30.0).max(160.0);
    let (gap, dy, hub_h) = (60.0f32, 190.0f32, 28.0f32);

    let mut nodes: Vec<Node> = hubs.iter().map(|&h| Node { hub: Some(h), w: width(h) }).collect();
    let node_of: HashMap<usize, usize> = hubs.iter().enumerate().map(|(n, &h)| (h, n)).collect();
    let mut rows: Vec<Vec<usize>> = vec![Vec::new(); levels];
    for (n, &h) in hubs.iter().enumerate() {
        rows[level[&h]].push(n);
    }
    let mut adj: Vec<(usize, usize)> = Vec::new();
    let mut same: Vec<(usize, usize)> = Vec::new();
    let mut chains: Vec<Vec<usize>> = Vec::new();
    for &(_, a, b) in &ends {
        let (na, nb) = (node_of[&a], node_of[&b]);
        let (lo, hi) = if level[&a] <= level[&b] { (na, nb) } else { (nb, na) };
        let (llo, lhi) = (level[&hubs[lo]], level[&hubs[hi]]);
        let mut chain = vec![lo];
        if llo == lhi {
            same.push((lo, hi));
        } else {
            for l in llo + 1..lhi {
                nodes.push(Node { hub: None, w: 16.0 });
                let n = nodes.len() - 1;
                rows[l].push(n);
                chain.push(n);
            }
            chain.push(hi);
            for w in chain.windows(2) {
                adj.push((w[0], w[1]));
            }
        }
        chains.push(chain);
    }
    let mut x = vec![0.0f32; nodes.len()];
    let place = |row: &[usize], x: &mut Vec<f32>| {
        let mut cur = 40.0f32;
        for &n in row {
            x[n] = cur + nodes[n].w / 2.0;
            cur += nodes[n].w + if nodes[n].hub.is_some() { gap } else { 14.0 };
        }
    };
    for row in &rows {
        place(row, &mut x);
    }
    for _ in 0..6 {
        for r in 1..levels {
            let a: Vec<(usize, usize)> = adj.iter().copied().filter(|(u, v)| rows[r - 1].contains(u) || rows[r - 1].contains(v)).collect();
            order(&mut rows[r], &x, &a, &same);
            place(&rows[r].clone(), &mut x);
        }
        for r in (0..levels.saturating_sub(1)).rev() {
            let a: Vec<(usize, usize)> = adj.iter().copied().filter(|(u, v)| rows[r + 1].contains(u) || rows[r + 1].contains(v)).collect();
            order(&mut rows[r], &x, &a, &same);
            place(&rows[r].clone(), &mut x);
        }
    }
    let row_y = |l: usize| 110.0 + (levels - 1 - l) as f32 * dy;
    let mut node_level = vec![0usize; nodes.len()];
    for (l, row) in rows.iter().enumerate() {
        for &n in row {
            node_level[n] = l;
            if let Some(h) = nodes[n].hub {
                let pos = pinned.get(&c.parts[h].id).copied().unwrap_or((x[n], row_y(l)));
                x[n] = pos.0;
                d.items.push(Item { part: h, role: Role::Hub, pos, size: (nodes[n].w, hub_h) });
            }
        }
    }

    let hub: HashMap<usize, Item> = d.items.iter().map(|i| (i.part, i.clone())).collect();
    let item = |n: usize| &hub[&nodes[n].hub.unwrap()];
    let mut used: HashMap<(usize, bool), Vec<f32>> = HashMap::new();
    let free = |h: usize, top: bool, want: f32, used: &HashMap<(usize, bool), Vec<f32>>| {
        let it = &hub[&h];
        let (l, r) = (it.pos.0 - it.size.0 / 2.0 + 14.0, it.pos.0 + it.size.0 / 2.0 - 14.0);
        let mut v = want.clamp(l, r);
        let taken = used.get(&(h, top)).cloned().unwrap_or_default();
        let mut k = 0u32;
        while taken.iter().any(|t| (t - v).abs() < 40.0) && k < 16 {
            k += 1;
            let step = 40.0 * k.div_ceil(2) as f32 * if k % 2 == 1 { 1.0 } else { -1.0 };
            v = (want + step).clamp(l, r);
        }
        v
    };
    let mut arcs: HashMap<usize, usize> = HashMap::new();
    let mut lanes: HashMap<usize, usize> = HashMap::new();
    let mut pairs: HashMap<(usize, usize), usize> = HashMap::new();
    for (k, &(part, _, _)) in ends.iter().enumerate() {
        let chain = &chains[k];
        let path = if chain.len() == 1 {
            let (_, a, b) = ends[k];
            let (ia, ib) = (&hub[&a], &hub[&b]);
            let (l, r) = if ia.pos.0 < ib.pos.0 { (ia, ib) } else { (ib, ia) };
            let lv = node_level[node_of[&l.part]];
            let blocked = rows[lv].iter().any(|&n| nodes[n].hub.is_some() && x[n] > l.pos.0 + 1.0 && x[n] < r.pos.0 - 1.0);
            if !blocked && (l.pos.1 - r.pos.1).abs() < 1.0 {
                vec![(l.pos.0 + l.size.0 / 2.0, l.pos.1), (r.pos.0 - r.size.0 / 2.0, r.pos.1)]
            } else {
                let n = arcs.entry(lv).or_default();
                let y = l.top().min(r.top()) - 92.0 - *n as f32 * 12.0;
                *n += 1;
                let xa = free(l.part, true, l.pos.0 + l.size.0 / 2.0, &used);
                let xb = free(r.part, true, r.pos.0 - r.size.0 / 2.0, &used);
                used.entry((l.part, true)).or_default().push(xa);
                used.entry((r.part, true)).or_default().push(xb);
                vec![(xa, l.top()), (xa, y), (xb, y), (xb, r.top())]
            }
        } else {
            let (lo, up) = (item(chain[0]), item(*chain.last().unwrap()));
            let next_x = x[chain[1]];
            let prev_x = x[chain[chain.len() - 2]];
            let want_lo = if chain.len() == 2 {
                let (ul, ur) = (up.pos.0 - up.size.0 / 2.0 + 14.0, up.pos.0 + up.size.0 / 2.0 - 14.0);
                let (ll, lr) = (lo.pos.0 - lo.size.0 / 2.0 + 14.0, lo.pos.0 + lo.size.0 / 2.0 - 14.0);
                if ul.max(ll) <= ur.min(lr) { (ul.max(ll) + ur.min(lr)) / 2.0 } else { next_x }
            } else {
                next_x
            };
            let xa = free(lo.part, true, want_lo, &used);
            used.entry((lo.part, true)).or_default().push(xa);
            let xb = free(up.part, false, if chain.len() == 2 { xa } else { prev_x }, &used);
            used.entry((up.part, false)).or_default().push(xb);
            let mut xs: Vec<f32> = vec![xa];
            xs.extend(chain[1..chain.len() - 1].iter().map(|&n| x[n]));
            xs.push(xb);
            let mut pts = vec![(xa, lo.top())];
            let mut cur = xa;
            let l0 = node_level[chain[0]];
            for (s, &nx) in xs.iter().enumerate().skip(1) {
                let l = l0 + s - 1;
                if (nx - cur).abs() >= 1.0 {
                    let lane = lanes.entry(l).or_default();
                    let mid = row_y(l) - 135.0 + (*lane % 7) as f32 * 9.0 - 27.0;
                    *lane += 1;
                    pts.push((cur, mid));
                    pts.push((nx, mid));
                }
                cur = nx;
            }
            pts.push((xb, up.bottom()));
            pts
        };
        let seg = path.windows(2).max_by(|a, b| {
            let la = (a[0].0 - a[1].0).abs() + (a[0].1 - a[1].1).abs();
            let lb = (b[0].0 - b[1].0).abs() + (b[0].1 - b[1].1).abs();
            la.total_cmp(&lb)
        });
        let (mut mark, vertical) = match seg {
            Some(s) => (((s[0].0 + s[1].0) / 2.0, (s[0].1 + s[1].1) / 2.0), (s[0].0 - s[1].0).abs() < 1.0),
            None => ((0.0, 0.0), false),
        };
        let (_, ea, eb) = ends[k];
        let n = pairs.entry((ea.min(eb), ea.max(eb))).or_insert(0usize);
        let step = 30.0 * n.div_ceil(2) as f32 * if *n % 2 == 1 { 1.0 } else { -1.0 };
        *n += 1;
        if vertical {
            mark.1 += step;
        } else {
            mark.0 += step * 2.0;
        }
        d.edges.push(Edge { part, path, mark, vertical });
    }

    let mut keys: Vec<_> = attached.keys().copied().collect();
    keys.sort();
    for (h, below) in keys {
        let Some(hi) = hub.get(&h) else { continue };
        let list = &attached[&(h, below)];
        let taken = used.get(&(h, !below)).cloned().unwrap_or_default();
        let total: f32 = list.iter().map(|&i| child_w(i) + 12.0).sum::<f32>() - 12.0;
        let mut cx = hi.pos.0 - total / 2.0;
        for &i in list {
            let w = child_w(i);
            while taken.iter().any(|t| *t > cx - 8.0 && *t < cx + w + 8.0) {
                cx += 10.0;
            }
            let r = if below { Role::Source } else { Role::Sink };
            let size = if below { (w, 40.0) } else { (w, 22.0) };
            let y = if below { hi.bottom() + 52.0 } else { hi.top() - 52.0 };
            let pos = pinned.get(&c.parts[i].id).copied().unwrap_or((cx + w / 2.0, y));
            let attach = if below { hi.bottom() } else { hi.top() };
            let from = if below { pos.1 - size.1 / 2.0 } else { pos.1 + size.1 / 2.0 };
            d.links.push((i, [(pos.0, from), (pos.0, attach)]));
            d.items.push(Item { part: i, role: r, pos, size });
            cx += w + 12.0;
        }
    }
    d
}

/// Every part of a domain without a diagram, as a list.
pub fn listed(c: &Checked, domain: &str) -> Vec<usize> {
    (0..c.parts.len()).filter(|&i| c.parts[i].domain == domain && role(&c.parts[i]).is_none() && edge_ends(&c.parts[i]).is_none()).collect()
}

/// Whether a part is supplied, failed, and its value text.
pub fn state(p: &CheckedPart, get: impl Fn(&str) -> Option<f64>) -> (bool, bool, String) {
    let g = |n: &str| get(&format!("{}.{n}", p.id));
    let failed = g("failed").is_some_and(|v| v != 0.0);
    let on = |n: &str| g(n).is_some_and(|v| v != 0.0);
    let psi = |n: &str| g(n).unwrap_or(0.0) / 6894.757;
    let (live, text) = match (p.domain, p.kind) {
        ("electrical", "bus") => (on("powered"), format!("{:.1} V", g("voltage").unwrap_or(0.0))),
        ("electrical", "battery") => (true, format!("{:.1} V {:+.0} A", g("voltage").unwrap_or(0.0), g("current").unwrap_or(0.0))),
        ("electrical", "generator") => (on("ready"), format!("{:.1} kW", g("load").unwrap_or(0.0) / 1e3)),
        ("electrical", "ext_power") => (on("available"), String::new()),
        ("electrical", "load") => (on("powered"), if on("shed") { "shed".into() } else { format!("{:.1} kW", g("power").unwrap_or(0.0) / 1e3) }),
        ("electrical", "contactor" | "breaker") => (on("closed"), if on("tripped") { "tripped".into() } else { String::new() }),
        ("electrical", "tru" | "inverter") => (on("active"), format!("{:.0} A", g("current").unwrap_or(0.0))),
        ("hydraulic", "circuit") => (on("available"), format!("{:.0} psi", psi("pressure"))),
        ("hydraulic", "pump") => (on("running"), String::new()),
        ("hydraulic", "consumer") => (on("powered"), String::new()),
        ("hydraulic", "accumulator") => (psi("pressure") > 500.0, format!("{:.0} psi", psi("pressure"))),
        ("hydraulic", "ptu") => (on("running"), String::new()),
        ("fuel", "tank") => (g("quantity").unwrap_or(0.0) > 0.0, format!("{:.0} kg", g("quantity").unwrap_or(0.0))),
        ("fuel", "line") => (g("pressure").unwrap_or(0.0) > 100.0, format!("{:.1} psi", psi("pressure"))),
        ("fuel", "pump") => (on("running"), format!("{:.2} kg/s", g("flow").unwrap_or(0.0))),
        ("fuel", "valve") | ("pneumatic", "valve") => (on("open"), String::new()),
        ("fuel", "feed") => (!on("starved") && g("flow").unwrap_or(0.0) > 0.0, format!("{:.2} kg/s", g("flow").unwrap_or(0.0))),
        ("pneumatic", "duct") => (g("pressure").unwrap_or(0.0) > 110_000.0, format!("{:.0} psi {:.0} °C", psi("pressure"), g("temperature").unwrap_or(273.15) - 273.15)),
        ("pneumatic", "source") => (on("available"), String::new()),
        ("pneumatic", "consumer") => (on("supplied"), String::new()),
        _ => (false, String::new()),
    };
    (live, failed, text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn twin_jet_hubs_do_not_overlap() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/twin-jet");
        let (a, _) = easyacft_parts::Aircraft::load(&dir).map_err(|r| r.render()).unwrap();
        let d = build(&a.checked, "electrical", &HashMap::new());
        let hubs: Vec<&Item> = d.items.iter().filter(|i| i.role == Role::Hub).collect();
        assert!(hubs.len() >= 10);
        for (k, x) in hubs.iter().enumerate() {
            for y in &hubs[k + 1..] {
                let apart = (x.pos.0 - y.pos.0).abs() >= (x.size.0 + y.size.0) / 2.0 || (x.pos.1 - y.pos.1).abs() >= x.size.1;
                assert!(apart, "{} overlaps {}", a.checked.parts[x.part].id, a.checked.parts[y.part].id);
            }
        }
        assert_eq!(d.edges.len(), a.checked.parts.iter().filter(|p| p.domain == "electrical" && edge_ends(p).is_some()).count());
    }
}
