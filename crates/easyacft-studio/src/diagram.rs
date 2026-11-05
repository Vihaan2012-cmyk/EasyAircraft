//! Automatic system diagrams: hubs in levels from their sources, attached parts around them.

use easyacft_lang::{Checked, CheckedPart};
use std::collections::{HashMap, HashSet, VecDeque};

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
    /// The hub an attached part hangs from.
    pub under: Option<usize>,
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

/// One breaker-fed user in a bus's panel: its breakers or SSPCs, its own bus and the loads on it.
#[derive(Clone, Debug)]
pub struct Row {
    pub pos: (f32, f32),
    pub w: f32,
    pub feeds: Vec<usize>,
    pub bus: usize,
    pub users: Vec<usize>,
    /// A tie or second feed to another bus, instead of users.
    pub to: Option<usize>,
}

/// A main bus's block of rows, below the network.
#[derive(Clone, Debug)]
pub struct Panel {
    pub hub: usize,
    pub pos: (f32, f32),
    pub size: (f32, f32),
}

pub const ROW_H: f32 = 22.0;

#[derive(Clone, Debug, Default)]
pub struct Diagram {
    pub rows: Vec<Row>,
    pub panels: Vec<Panel>,
    pub items: Vec<Item>,
    pub edges: Vec<Edge>,
    pub links: Vec<(usize, [(f32, f32); 2])>,
    /// Per drawn hub: breaker-fed sub-buses and loads folded into it.
    pub folded: HashMap<usize, (usize, usize)>,
    /// Frames around a family of computers drawn as one box: left, top, width, height.
    pub groups: Vec<(f32, f32, f32, f32)>,
    /// Hubs that carry the structure (not a load's own feeder bus or a pass-through fuel line): labelled at any zoom.
    pub major: HashSet<usize>,
}

/// Most users a hub shows before they fold into its count.
pub const MAX_SINKS: usize = 8;

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
        ("computer", "air_data" | "inertial") if p.part("computer").is_some() => (Role::Sink, "computer"),
        ("computer", "unit" | "air_data" | "inertial") => (Role::Hub, ""),
        _ => return None,
    })
}

pub fn edge_ends(p: &CheckedPart) -> Option<(usize, usize)> {
    match (p.domain, p.kind) {
        ("electrical", "contactor" | "breaker" | "sspc" | "diode") | ("hydraulic", "ptu") | ("fuel", "valve") | ("pneumatic", "valve") => {
            let e = p.parts("between");
            Some((*e.first()?, *e.get(1)?))
        }
        ("electrical", "tru" | "inverter") => Some((p.part("input")?, p.part("output")?)),
        ("fuel", "pump") => Some((p.part("tank")?, p.part("outlet")?)),
        ("data", "link") => Some((p.part("from")?, *p.parts("to").first()?)),
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

/// As `role`, with custom parts placed by their port in `domain`.
pub fn role_in(domain: &str, p: &CheckedPart) -> Option<(Role, &'static str)> {
    if p.domain != "custom" {
        return role(p);
    }
    let port = easyacft_parts::custom::ports_of(p.kind).into_iter().find(|x| x.domain == domain && !x.link)?;
    Some((if port.source { Role::Source } else { Role::Sink }, port.name))
}

/// As `edge_ends`, with custom links in `domain`.
pub fn edge_in(domain: &str, p: &CheckedPart) -> Option<(usize, usize)> {
    if p.domain != "custom" {
        return edge_ends(p);
    }
    let link = easyacft_parts::custom::ports_of(p.kind).into_iter().find(|x| x.domain == domain && x.link)?;
    let e = p.parts(link.name);
    Some((*e.first()?, *e.get(1)?))
}

/// A computer's family: its id without a short trailing number or letter, `prim_2` and `fadec_1a` as `prim` and `fadec`.
fn stem(id: &str) -> &str {
    match id.rsplit_once('_') {
        Some((head, last)) if last.len() <= 2 && (last.len() == 1 || last.chars().any(|c| c.is_ascii_digit())) => head,
        _ => id.trim_end_matches(|c: char| c.is_ascii_digit()),
    }
}

/// Computer levels, top to bottom: each switched network (a mesh of one family's switches, at least three links
/// each) with the first network over its modules over the last; under it every other linked computer by how its
/// data flows, families together; computers with no links at the bottom, level 0. Also the bottom switch rows,
/// whose own links run below them.
fn network_levels(c: &Checked, hubs: &[usize], ends: &[(usize, usize, usize)]) -> (HashMap<usize, usize>, HashSet<usize>, HashSet<usize>) {
    let same = |a: usize, b: usize| stem(&c.parts[a].id) == stem(&c.parts[b].id);
    let mut core: HashSet<usize> = hubs.iter().copied().collect();
    loop {
        let weak: Vec<usize> = core.iter().copied().filter(|&h| ends.iter().filter(|(_, a, b)| ((*a == h && core.contains(b)) || (*b == h && core.contains(a))) && same(*a, *b)).count() < 3).collect();
        if weak.is_empty() {
            break;
        }
        for h in weak {
            core.remove(&h);
        }
    }
    let near = |x: usize, f: &dyn Fn(usize) -> bool| -> Vec<usize> { ends.iter().filter_map(|&(_, a, b)| if a == x && f(b) { Some(b) } else if b == x && f(a) { Some(a) } else { None }).collect() };
    let mut comp: HashMap<usize, usize> = HashMap::new();
    let mut n = 0;
    for &h in hubs.iter().filter(|h| core.contains(h)) {
        if comp.contains_key(&h) {
            continue;
        }
        let mut q = VecDeque::from([h]);
        comp.insert(h, n);
        while let Some(x) = q.pop_front() {
            for y in near(x, &|y| core.contains(&y) && same(x, y)) {
                if comp.insert(y, n).is_none() {
                    q.push_back(y);
                }
            }
        }
        n += 1;
    }
    let mut dist: HashMap<usize, usize> = core.iter().map(|&h| (h, 0)).collect();
    let mut q: VecDeque<usize> = hubs.iter().copied().filter(|h| core.contains(h)).collect();
    while let Some(x) = q.pop_front() {
        for y in near(x, &|_| true) {
            if !dist.contains_key(&y) {
                dist.insert(y, dist[&x] + 1);
                q.push_back(y);
            }
        }
    }
    // Data flow among the rest, by family: sources on top, readers below, cycles broken greedily.
    let linked: Vec<usize> = hubs.iter().copied().filter(|h| !dist.contains_key(h) && ends.iter().any(|e| e.1 == *h || e.2 == *h)).collect();
    let fam = |h: usize| stem(&c.parts[h].id).to_owned();
    let mut fams: Vec<String> = Vec::new();
    for &h in &linked {
        if !fams.contains(&fam(h)) {
            fams.push(fam(h));
        }
    }
    let fi = |h: usize| fams.iter().position(|f| *f == fam(h));
    let mut arcs: HashSet<(usize, usize)> = HashSet::new();
    for &(_, a, b) in ends {
        if let (Some(x), Some(y)) = (fi(a), fi(b)) {
            if x != y && linked.contains(&a) && linked.contains(&b) {
                arcs.insert((x, y));
            }
        }
    }
    let mut left: Vec<usize> = (0..fams.len()).collect();
    let (mut head, mut tail): (Vec<usize>, Vec<usize>) = (Vec::new(), Vec::new());
    while !left.is_empty() {
        let deg = |f: usize, left: &[usize], out: bool| arcs.iter().filter(|&&(x, y)| if out { x == f && left.contains(&y) } else { y == f && left.contains(&x) }).count();
        if let Some(k) = left.iter().position(|&f| deg(f, &left, true) == 0) {
            tail.insert(0, left.remove(k));
        } else if let Some(k) = left.iter().position(|&f| deg(f, &left, false) == 0) {
            head.push(left.remove(k));
        } else {
            let k = (0..left.len()).max_by_key(|&k| deg(left[k], &left, true) as i64 - deg(left[k], &left, false) as i64).unwrap();
            head.push(left.remove(k));
        }
    }
    head.extend(tail);
    let rank: HashMap<usize, usize> = head.iter().enumerate().map(|(r, &f)| (f, r)).collect();
    let mut depth = vec![0usize; fams.len()];
    for &f in &head {
        for &(x, y) in &arcs {
            let (x, y) = if rank[&x] < rank[&y] { (x, y) } else { (y, x) };
            if y == f {
                depth[f] = depth[f].max(depth[x] + 1);
            }
        }
    }
    // Readers that pass nothing on, such as displays, sit together on the bottom row.
    let last = depth.iter().copied().max().unwrap_or(0);
    for f in 0..fams.len() {
        if !arcs.iter().any(|&(x, _)| x == f) {
            depth[f] = last;
        }
    }
    let flow = last + 1;
    let deepest = dist.values().copied().max().unwrap_or(0);
    let top = flow + 1;
    let mut below = HashSet::new();
    let levels = hubs
        .iter()
        .map(|&h| {
            let l = if let Some(f) = fi(h).filter(|_| linked.contains(&h)) {
                flow - depth[f]
            } else {
                match (comp.get(&h), dist.get(&h)) {
                    (Some(0), _) if n > 1 => top + deepest + 1,
                    (Some(_), _) if n > 1 => {
                        below.insert(h);
                        top
                    }
                    (_, Some(&d)) if n > 1 => top + d,
                    (_, Some(&d)) => top + deepest - d,
                    (_, None) => 0,
                }
            };
            (h, l)
        })
        .collect();
    (levels, below, linked.into_iter().collect())
}

/// `fold` folds feeder buses and long user lists into their parent; off, every breaker and user is drawn.
pub fn build(c: &Checked, domain: &str, pinned: &HashMap<String, (f32, f32)>, fold: bool, aspect: f32) -> Diagram {
    let mut d = Diagram::default();
    let parts: Vec<usize> = (0..c.parts.len()).filter(|&i| c.parts[i].domain == domain || (domain == "computer" && c.parts[i].domain == "data") || (c.parts[i].domain == "custom" && easyacft_parts::custom::ports_of(c.parts[i].kind).iter().any(|x| x.domain == domain))).collect();
    let mut hubs: Vec<usize> = parts.iter().copied().filter(|&i| role_in(domain, &c.parts[i]).is_some_and(|r| r.0 == Role::Hub)).collect();
    // A computer's link from or to a part it hosts, such as ADR 1 in ADIRS 1, is drawn from the host.
    let host = |p: usize| match role_in(domain, &c.parts[p]) {
        Some((Role::Sink, f)) if domain == "computer" => c.parts[p].part(f).unwrap_or(p),
        _ => p,
    };
    let mut ends: Vec<(usize, usize, usize)> = parts.iter().filter_map(|&i| edge_in(domain, &c.parts[i]).map(|(a, b)| (i, host(a), host(b)))).filter(|(_, a, b)| hubs.contains(a) && hubs.contains(b) && a != b).collect();
    if domain == "computer" {
        let mut seen = HashSet::new();
        ends.retain(|(_, a, b)| seen.insert((*a.min(b), *a.max(b))));
    }
    // A bus fed by one breaker and feeding only users is a distribution feeder: fold it into its parent.
    let sources_of = |h: usize| parts.iter().any(|&i| role_in(domain, &c.parts[i]).is_some_and(|(r, f)| r == Role::Source && c.parts[i].part(f) == Some(h)));
    // Also a load's own bus behind SSPCs and diodes: its neighbour that is not itself such a feeder is its parent.
    let protective = |e: usize| matches!(c.parts[e].kind, "breaker" | "sspc" | "diode");
    let feeders: HashSet<usize> = hubs
        .iter()
        .copied()
        .filter(|&h| {
            let touching: Vec<&(usize, usize, usize)> = ends.iter().filter(|(_, a, b)| *a == h || *b == h).collect();
            touching.iter().any(|(e, _, _)| c.parts[*e].kind != "diode") && touching.iter().all(|(e, _, _)| protective(*e)) && !sources_of(h)
        })
        .collect();
    let attached_to = |h: usize| parts.iter().any(|&i| role_in(domain, &c.parts[i]).is_some_and(|(r, f)| r != Role::Hub && c.parts[i].part(f) == Some(h)));
    d.major = hubs.iter().copied().filter(|h| !feeders.contains(h) && (domain != "fuel" || c.parts[*h].kind == "tank" || attached_to(*h))).collect();
    let folding = fold;
    let candidate: HashSet<usize> = if folding { feeders.clone() } else { HashSet::new() };
    let mut feeder_of: HashMap<usize, usize> = HashMap::new();
    for &h in &candidate {
        let parent = ends.iter().filter(|(_, a, b)| *a == h || *b == h).map(|&(_, a, b)| if a == h { b } else { a }).find(|n| !candidate.contains(n));
        if let Some(p) = parent {
            feeder_of.insert(h, p);
        }
    }
    hubs.retain(|h| !feeder_of.contains_key(h));
    ends.retain(|(_, a, b)| !feeder_of.contains_key(a) && !feeder_of.contains_key(b));
    for (&f, &parent) in &feeder_of {
        let top = feeder_of.get(&parent).copied().unwrap_or(parent);
        let users = parts.iter().filter(|&&i| role_in(domain, &c.parts[i]).is_some_and(|(r, fl)| r == Role::Sink && c.parts[i].part(fl) == Some(f))).count();
        let e = d.folded.entry(top).or_default();
        e.0 += 1;
        e.1 += users;
    }
    // A fuel line that only joins two pipes, pumps or valves is drawn as one run from end to end.
    if domain == "fuel" {
        loop {
            let through = |h: usize| ends.iter().filter(|(_, a, b)| *a == h || *b == h).count() == 2;
            let Some(h) = hubs.iter().copied().find(|&h| c.parts[h].kind == "line" && through(h) && !attached_to(h)) else { break };
            let two: Vec<usize> = ends.iter().enumerate().filter(|(_, (_, a, b))| *a == h || *b == h).map(|(k, _)| k).collect();
            let other = ends[two[1]];
            let far = if other.1 == h { other.2 } else { other.1 };
            hubs.retain(|&q| q != h);
            ends.remove(two[1]);
            let kept = &mut ends[two[0]];
            if kept.1 == h {
                kept.1 = far;
            } else {
                kept.2 = far;
            }
            if kept.1 == kept.2 {
                ends.remove(two[0]);
            }
        }
    }
    let mut attached: HashMap<(usize, bool), Vec<usize>> = HashMap::new();
    // A computer linked to only one other, such as a function a CPIOM hosts, hangs off it.
    if domain == "computer" {
        let degree = |h: usize, ends: &[(usize, usize, usize)]| ends.iter().filter(|(_, a, b)| *a == h || *b == h).count();
        let leaves: Vec<(usize, usize)> = hubs
            .iter()
            .filter_map(|&h| {
                let mut e = ends.iter().filter(|(_, a, b)| *a == h || *b == h);
                let (_, a, b) = *e.next()?;
                let other = if a == h { b } else { a };
                (e.next().is_none() && degree(other, &ends) > 1).then_some((h, other))
            })
            .collect();
        for &(h, parent) in &leaves {
            attached.entry((parent, false)).or_default().push(h);
            hubs.retain(|&q| q != h);
            ends.retain(|(_, a, b)| *a != h && *b != h);
        }
    }
    for &i in &parts {
        let Some((r, field)) = role_in(domain, &c.parts[i]) else { continue };
        if r == Role::Hub {
            continue;
        }
        if let Some(h) = c.parts[i].part(field).filter(|h| hubs.contains(h)) {
            attached.entry((h, r == Role::Source)).or_default().push(i);
        }
    }
    for ((h, src), list) in attached.iter_mut() {
        if folding && !*src && list.len() > MAX_SINKS {
            d.folded.entry(*h).or_default().1 += list.len();
            list.clear();
        }
    }

    let (mut level, below_rows, flowing) = if domain == "computer" { network_levels(c, &hubs, &ends) } else { (HashMap::new(), HashSet::new(), HashSet::new()) };
    // A family of computers in the data flow, such as PRIM 1 to 3, is one framed box, one link per pair of families.
    let mut members: HashMap<usize, Vec<usize>> = HashMap::new();
    for &h in &hubs {
        if flowing.contains(&h) {
            let r = *hubs.iter().find(|&&g| flowing.contains(&g) && stem(&c.parts[g].id) == stem(&c.parts[h].id) && level.get(&g) == level.get(&h)).unwrap();
            members.entry(r).or_default().push(h);
        }
    }
    let rep_of: HashMap<usize, usize> = members.iter().flat_map(|(&r, m)| m.iter().map(move |&h| (h, r))).collect();
    if !rep_of.is_empty() {
        hubs.retain(|h| rep_of.get(h).is_none_or(|r| r == h));
        for e in ends.iter_mut() {
            e.1 = rep_of.get(&e.1).copied().unwrap_or(e.1);
            e.2 = rep_of.get(&e.2).copied().unwrap_or(e.2);
        }
        ends.retain(|(_, a, b)| a != b);
        let mut seen = HashSet::new();
        ends.retain(|(_, a, b)| seen.insert((*a.min(b), *a.max(b))));
    }
    let mut q = VecDeque::new();
    for &h in &hubs {
        if domain != "computer" && (attached.contains_key(&(h, true)) || c.parts[h].kind == "tank") {
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
        match role_in(domain, &c.parts[i]).map(|r| r.0) {
            Some(Role::Source) => (w + 14.0).max(56.0),
            _ => (w + 16.0).max(56.0),
        }
    };
    // A computer's many hosted parts stack in a grid of up to three a row.
    let grid = |n: usize| if domain == "computer" && n > 3 { let rows = n.div_ceil(3); (n.div_ceil(rows), rows) } else { (n, 1) };
    let side_w = |h: usize, below: bool| {
        attached.get(&(h, below)).map_or(0.0, |l| {
            let (cols, rows) = grid(l.len());
            if rows > 1 { cols as f32 * (l.iter().map(|&i| child_w(i)).fold(0.0, f32::max) + 12.0) } else { l.iter().map(|&i| child_w(i) + 12.0).sum::<f32>() }
        })
    };
    let stacked = |h: usize| attached.get(&(h, false)).map_or(1, |l| grid(l.len()).1);
    let degree = |h: usize| ends.iter().filter(|(_, a, b)| *a == h || *b == h).count() as f32;
    let net = domain == "computer";
    // Computers carry no power or flow readings, so their boxes and gaps only need room for the name.
    let width = |h: usize| if net { (text_w(&c.parts[h].id) * 1.15 + 24.0).max(side_w(h, true).max(side_w(h, false)) + 12.0).max(70.0) } else { (text_w(&c.parts[h].id) + 76.0).max(side_w(h, true).max(side_w(h, false)) + 24.0 + degree(h) * 26.0).max(110.0) };
    let (gap, dy, hub_h) = if net { (20.0f32, 160.0f32, 34.0f32) } else { (44.0f32, 190.0f32, 40.0f32) };

    let family_w = |h: usize| match members.get(&h) {
        Some(m) if m.len() > 1 => m.iter().map(|&x| width(x)).sum::<f32>() + 12.0 * (m.len() - 1) as f32 + 20.0,
        _ => width(h),
    };
    let mut nodes: Vec<Node> = hubs.iter().map(|&h| Node { hub: Some(h), w: family_w(h) }).collect();
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
    let span = |row: &[usize]| row.iter().map(|&n| nodes[n].w + if nodes[n].hub.is_some() { gap } else { 14.0 }).sum::<f32>();
    let widest = rows.iter().map(|r| span(r)).fold(0.0, f32::max);
    // Wrap rows at the width that gives the whole diagram the panel's own shape.
    let total: f32 = rows.iter().map(|r| span(r)).sum();
    let (b, k) = (aspect * dy * levels as f32, aspect * dy * total);
    let page = (b + (b * b + 4.0 * k).sqrt()) / 2.0;
    let max_w = if domain == "computer" { rows.iter().skip(1).map(|r| span(r)).fold(1500.0, f32::max) } else { widest.min(page.max(1500.0)) };
    let mut all_lines: Vec<Vec<usize>> = Vec::new();
    let mut node_y = vec![0.0f32; nodes.len()];
    let mut node_line = vec![0usize; nodes.len()];
    let mut y = 110.0f32;
    let mut line_no = 0;
    for l in (0..levels).rev() {
        // A row too long for the page wraps in contiguous runs, so siblings stay together under their parent.
        let mut lines: Vec<Vec<usize>> = vec![Vec::new()];
        let mut acc = 0.0f32;
        for &n in &rows[l] {
            let w = nodes[n].w + if nodes[n].hub.is_some() { gap } else { 14.0 };
            // A computer network's levels stay whole; only the unlinked computers wrap.
            if acc + w > max_w && lines.last().is_some_and(|l| !l.is_empty()) && (domain != "computer" || l == 0) {
                lines.push(Vec::new());
                acc = 0.0;
            }
            lines.last_mut().unwrap().push(n);
            acc += w;
        }
        for line in lines {
            place(&line, &mut x);
            // Room for the arcs between neighbours on one line: above it, or below the bottom line of a network.
            let arcs = same.iter().filter(|(a, b)| line.contains(a) || line.contains(b)).count() as f32 * 12.0;
            let down = line.iter().any(|n| nodes[*n].hub.is_some_and(|h| below_rows.contains(&h)));
            if !down {
                y += arcs;
            }
            y += line.iter().filter_map(|n| nodes[*n].hub).map(|h| stacked(h) - 1).max().unwrap_or(0) as f32 * 40.0;
            for &n in &line {
                node_y[n] = y;
                node_line[n] = line_no;
            }
            y += dy + if down { arcs } else { 0.0 };
            line_no += 1;
            all_lines.push(line);
        }
    }
    // A network's narrower lines sit over what they connect to, in their order and without overlapping.
    if domain == "computer" {
        let full = all_lines.iter().map(|l| span(l)).fold(0.0, f32::max);
        for _ in 0..4 {
            for line in all_lines.iter().filter(|l| span(l) < full - 1.0) {
                let want: Vec<f32> = line
                    .iter()
                    .map(|&n| {
                        let xs: Vec<f32> = adj.iter().filter_map(|&(a, b)| if a == n { Some(x[b]) } else if b == n { Some(x[a]) } else { None }).collect();
                        if xs.is_empty() { x[n] } else { xs.iter().sum::<f32>() / xs.len() as f32 }
                    })
                    .collect();
                let sp = |i: usize| (nodes[line[i - 1]].w + nodes[line[i]].w) / 2.0 + if nodes[line[i - 1]].hub.is_some() { gap } else { 14.0 };
                for (i, &n) in line.iter().enumerate() {
                    x[n] = if i == 0 { want[0].max(40.0 + nodes[n].w / 2.0) } else { want[i].max(x[line[i - 1]] + sp(i)) };
                }
                for i in (1..line.len()).rev() {
                    let (a, b) = (line[i - 1], line[i]);
                    x[a] = x[a].min(x[b] - sp(i)).max(want[i - 1].min(x[a]));
                }
                for i in 1..line.len() {
                    x[line[i]] = x[line[i]].max(x[line[i - 1]] + sp(i));
                }
            }
        }
    }
    let mut frames: HashMap<usize, Item> = HashMap::new();
    for row in &rows {
        for &n in row {
            if let Some(h) = nodes[n].hub {
                let pos = pinned.get(&c.parts[h].id).copied().unwrap_or((x[n], node_y[n]));
                x[n] = pos.0;
                node_y[n] = pos.1;
                let frame = Item { part: h, role: Role::Hub, pos, size: (nodes[n].w, hub_h), under: None };
                match members.get(&h) {
                    Some(m) if m.len() > 1 => {
                        let mut cx = pos.0 - nodes[n].w / 2.0 + 10.0;
                        for &x in m {
                            let w = width(x);
                            d.items.push(Item { part: x, role: Role::Hub, pos: (cx + w / 2.0, pos.1), size: (w, hub_h), under: None });
                            cx += w + 12.0;
                        }
                        d.groups.push((pos.0 - nodes[n].w / 2.0, pos.1 - hub_h / 2.0 - 7.0, nodes[n].w, hub_h + 14.0));
                    }
                    _ => d.items.push(frame.clone()),
                }
                frames.insert(h, frame);
            }
        }
    }

    let hub: HashMap<usize, Item> = frames;
    let placed: HashMap<usize, Item> = d.items.iter().map(|i| (i.part, i.clone())).collect();
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
    let arcs_below = |h: &usize| below_rows.contains(h);
    let mut arcs: HashMap<i32, usize> = HashMap::new();
    let mut lanes: HashMap<i32, usize> = HashMap::new();
    let mut pairs: HashMap<(usize, usize), usize> = HashMap::new();
    for (k, &(part, _, _)) in ends.iter().enumerate() {
        let chain = &chains[k];
        let (_, a0, b0) = ends[k];
        let level_pair = chain.len() == 1 && (hub[&a0].pos.1 - hub[&b0].pos.1).abs() >= 1.0;
        let chain = &if level_pair {
            let (na, nb) = (node_of[&a0], node_of[&b0]);
            if node_y[na] > node_y[nb] { vec![na, nb] } else { vec![nb, na] }
        } else {
            chain.clone()
        };
        let path = if chain.len() == 1 {
            let (_, a, b) = ends[k];
            let (ia, ib) = (&hub[&a], &hub[&b]);
            let (l, r) = if ia.pos.0 < ib.pos.0 { (ia, ib) } else { (ib, ia) };
            let lv = l.pos.1 as i32;
            let blocked = hub.values().any(|h| (h.pos.1 - l.pos.1).abs() < 1.0 && h.pos.0 > l.pos.0 + 1.0 && h.pos.0 < r.pos.0 - 1.0);
            if !blocked {
                vec![(l.pos.0 + l.size.0 / 2.0, l.pos.1), (r.pos.0 - r.size.0 / 2.0, r.pos.1)]
            } else {
                let n = arcs.entry(lv).or_default();
                let down = arcs_below(&l.part);
                let y = if down { l.bottom().max(r.bottom()) + 40.0 + *n as f32 * 12.0 } else { l.top().min(r.top()) - 92.0 - *n as f32 * 12.0 };
                *n += 1;
                let xa = free(l.part, !down, l.pos.0 + l.size.0 / 2.0, &used);
                let xb = free(r.part, !down, r.pos.0 - r.size.0 / 2.0, &used);
                used.entry((l.part, !down)).or_default().push(xa);
                used.entry((r.part, !down)).or_default().push(xb);
                let (ya, yb) = if down { (l.bottom(), r.bottom()) } else { (l.top(), r.top()) };
                vec![(xa, ya), (xa, y), (xb, y), (xb, yb)]
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
            for (s, &nx) in xs.iter().enumerate().skip(1) {
                let below = node_y[chain[s - 1]];
                if (nx - cur).abs() >= 1.0 {
                    let lane = lanes.entry(below as i32).or_default();
                    let mid = below - 135.0 + (*lane % 7) as f32 * 9.0 - 27.0;
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
        let Some(hi) = placed.get(&h) else { continue };
        let list = &attached[&(h, below)];
        let taken = used.get(&(h, !below)).cloned().unwrap_or_default();
        let (cols, rows) = grid(list.len());
        let cell = list.iter().map(|&i| child_w(i)).fold(0.0, f32::max);
        let total: f32 = if rows > 1 { cols as f32 * (cell + 12.0) - 12.0 } else { list.iter().map(|&i| child_w(i) + 12.0).sum::<f32>() - 12.0 };
        let mut cx = hi.pos.0 - total / 2.0;
        for (k, &i) in list.iter().enumerate() {
            let w = if rows > 1 { cell } else { child_w(i) };
            if rows > 1 {
                cx = hi.pos.0 - total / 2.0 + (k % cols) as f32 * (cell + 12.0);
            } else {
                while taken.iter().any(|t| *t > cx - 8.0 && *t < cx + w + 8.0) {
                    cx += 10.0;
                }
            }
            let r = if below { Role::Source } else { Role::Sink };
            let size = (w, 34.0);
            let lift = (k / cols.max(1)) as f32 * 40.0;
            let y = if below { hi.bottom() + 52.0 + lift } else { hi.top() - 52.0 - lift };
            let pos = pinned.get(&c.parts[i].id).copied().unwrap_or((cx + w / 2.0, y));
            let attach = if below { hi.bottom() } else { hi.top() };
            let from = if below { pos.1 - size.1 / 2.0 } else { pos.1 + size.1 / 2.0 };
            d.links.push((i, [(pos.0, from), (pos.0, attach)]));
            d.items.push(Item { part: i, role: r, pos, size, under: Some(h) });
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
    if p.domain == "custom" {
        if let Some(pt) = easyacft_parts::custom::ports_of(p.kind).first() {
            let on = g(&format!("{}_g", pt.name)).unwrap_or(0.0) > 0.0;
            return (on && !failed, failed, format!("{:.3}", g(&format!("{}_q", pt.name)).unwrap_or(0.0)));
        }
    }
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
        let d = build(&a.checked, "electrical", &HashMap::new(), true, 1.2);
        let hubs: Vec<&Item> = d.items.iter().filter(|i| i.role == Role::Hub).collect();
        assert!(hubs.len() >= 10);
        for (k, x) in hubs.iter().enumerate() {
            for y in &hubs[k + 1..] {
                let apart = (x.pos.0 - y.pos.0).abs() >= (x.size.0 + y.size.0) / 2.0 || (x.pos.1 - y.pos.1).abs() >= x.size.1;
                assert!(apart, "{} overlaps {}", a.checked.parts[x.part].id, a.checked.parts[y.part].id);
            }
        }
        let folded: usize = d.folded.values().map(|f| f.0).sum();
        assert_eq!(d.edges.len() + folded, a.checked.parts.iter().filter(|p| p.domain == "electrical" && edge_ends(p).is_some()).count());
    }
}
