//! Electrical domain: one phasor network for the whole aircraft. DC buses carry a voltage, AC buses a
//! complex voltage per phase at their island's frequency, solved together each step as a sparse nodal
//! admittance system. Generators are synchronous machines with their exciters, regulators, governors and
//! control units; batteries are equivalent circuits; converters couple the AC and DC sides; switchgear,
//! feeders and transformers join the buses; loads draw by type and phase; shedding keeps each island inside
//! its sources. A step whose solution is not finite is thrown away and the domain rolls back.

mod battery;
mod bus;
pub mod complex;
mod convert;
mod feeder;
mod gcu;
mod grid;
mod ground;
mod load;
mod machine;
mod sparse;
mod switchgear;

use crate::{Domain, Expr};
use battery::Battery;
use bus::{Bus, BusKind, Islands};
use complex::{balanced, C, ZERO, ZERO3};
use convert::{ConvKind, Converter, Transformer};
use easyacft_core::Dim;
use easyacft_lang::schema::{Kind, PartSpec};
use easyacft_lang::{Checked, CheckedPart, Type};
use feeder::Feeder;
use grid::{Grid, Shunt};
use ground::ExtPower;
use load::Load;
use machine::Generator;
use std::collections::{HashMap, VecDeque};
use switchgear::{Breaker, Contactor, Diode, Path, Sspc};

pub(crate) const B: Type = Type::Bool;
pub(crate) const V: Type = Type::Num(Dim::VOLTAGE);
pub(crate) const A: Type = Type::Num(Dim::CURRENT);
pub(crate) const W: Type = Type::Num(Dim::POWER);
pub(crate) const HZ: Type = Type::Num(Dim::FREQUENCY);
pub(crate) const N: Type = Type::Num(Dim::NONE);
pub(crate) const T: Type = Type::Num(Dim::TEMPERATURE);
pub(crate) const Q: Type = Type::Num(Dim::CHARGE);
pub(crate) const SEC: Type = Type::Num(Dim::TIME);
pub(crate) const TQ: Type = Type::Num(Dim::TORQUE);
pub(crate) const E: Type = Type::Num(Dim::ENERGY);
pub(crate) const WHEN: Kind = Kind::Expr(Type::Bool);

pub static SPECS: &[PartSpec] = &[
    bus::SPEC,
    battery::SPEC,
    machine::SPEC,
    ground::SPEC,
    switchgear::SPECS[0],
    switchgear::SPECS[1],
    switchgear::SPECS[2],
    switchgear::SPECS[3],
    load::SPEC,
    convert::SPECS[0],
    convert::SPECS[1],
    convert::SPECS[2],
    convert::SPECS[3],
    feeder::SPEC,
];

const UNSHED_MARGIN: f64 = 0.9;
/// Most times a step re-solves for diodes switching on its own voltages.
const DIODE_PASSES: usize = 8;
/// Steps between rollback checkpoints.
const CHECKPOINT_EVERY: u32 = 50;

pub(crate) fn eval(e: &mut Option<Expr>, slots: &[f64], dt: f64, or: f64) -> f64 {
    e.as_mut().map_or(or, |x| x.eval(slots, dt))
}

pub(crate) fn yes(e: &mut Option<Expr>, slots: &[f64], dt: f64, or: bool) -> bool {
    e.as_mut().map_or(or, |x| x.yes(slots, dt))
}

#[inline]
pub(crate) fn put(slots: &mut [f64], at: usize, v: f64) {
    slots[at] = if v.is_finite() { v } else { 0.0 };
}

#[inline]
pub(crate) fn flag(slots: &mut [f64], at: usize, v: bool) {
    slots[at] = v as u8 as f64;
}

/// An angle wrapped into (-pi, pi].
pub(crate) fn wrap(a: f64) -> f64 {
    let t = std::f64::consts::TAU;
    let x = a.rem_euclid(t);
    if x > std::f64::consts::PI {
        x - t
    } else {
        x
    }
}

/// A custom part's port on a bus: a Norton source, flow = g (e - v).
struct CustomPort {
    sh: Shunt,
    ac: bool,
    n: usize,
    bus: usize,
    /// Slots of its e, g, v and q values.
    slot: [usize; 4],
}

struct CustomLink {
    path: Path,
    g: usize,
    q: usize,
}

/// Which link a generator's differential zone ends on.
#[derive(Clone, Copy)]
enum LinkRef {
    Contactor(usize),
    Breaker(usize),
    Sspc(usize),
    Feeder(usize),
}

pub struct Electrical {
    grid: Grid,
    buses: Vec<Bus>,
    isl: Islands,
    gens: Vec<Generator>,
    exts: Vec<ExtPower>,
    bats: Vec<Battery>,
    convs: Vec<Converter>,
    charger_bat: Vec<Option<usize>>,
    loads: Vec<Load>,
    contactors: Vec<Contactor>,
    breakers: Vec<Breaker>,
    diodes: Vec<Diode>,
    sspcs: Vec<Sspc>,
    feeders: Vec<Feeder>,
    transformers: Vec<Transformer>,
    ports: Vec<CustomPort>,
    custom_links: Vec<CustomLink>,
    zones: Vec<(usize, LinkRef)>,
    shed_order: Vec<usize>,
    key: Vec<bool>,
    scratch_key: Vec<bool>,
    pairs: Vec<(usize, usize)>,
    checkpoint: Vec<f64>,
    since: u32,
    pub rollbacks: u64,
}

/// AC unless declared otherwise when an AC source, inverter output, transformer or TRU input can reach the bus.
fn bus_kinds(c: &Checked, parts: &[(usize, &CheckedPart)], bus_of: &HashMap<usize, usize>) -> Vec<BusKind> {
    let n = bus_of.len();
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut seed = vec![false; n];
    let b = |i: Option<usize>| i.and_then(|x| bus_of.get(&x).copied());
    for &(_, p) in parts {
        match p.kind {
            "contactor" | "breaker" | "diode" | "sspc" | "feeder" | "transformer" => {
                let e = p.parts("between");
                if let (Some(x), Some(y)) = (b(e.first().copied()), b(e.get(1).copied())) {
                    adj[x].push(y);
                    adj[y].push(x);
                    if p.kind == "transformer" {
                        seed[x] = true;
                        seed[y] = true;
                    }
                }
            }
            "generator" | "ext_power" if p.num("frequency") > 0.0 => {
                if let Some(x) = b(p.part("bus")) {
                    seed[x] = true;
                }
            }
            "inverter" => {
                if let Some(x) = b(p.part("output")) {
                    seed[x] = true;
                }
            }
            "tru" => {
                if let Some(x) = b(p.part("input")) {
                    seed[x] = true;
                }
            }
            _ => {}
        }
    }
    let mut declared = vec![None; n];
    for (&pi, &bi) in bus_of {
        declared[bi] = bus::declared_kind(&c.parts[pi]);
    }
    let mut ac = vec![false; n];
    let mut q: VecDeque<usize> = (0..n).filter(|&i| seed[i] && declared[i] != Some(BusKind::Dc)).collect();
    for &i in &q {
        ac[i] = true;
    }
    while let Some(i) = q.pop_front() {
        for &j in &adj[i] {
            if !ac[j] && declared[j] != Some(BusKind::Dc) {
                ac[j] = true;
                q.push_back(j);
            }
        }
    }
    (0..n).map(|i| declared[i].unwrap_or(if ac[i] { BusKind::Ac3 } else { BusKind::Dc })).collect()
}

impl Electrical {
    pub fn new(c: &Checked) -> Option<Self> {
        let parts: Vec<(usize, &CheckedPart)> = c.parts.iter().enumerate().filter(|(_, p)| p.domain == "electrical").collect();
        if parts.is_empty() {
            return None;
        }
        let mut bus_of: HashMap<usize, usize> = HashMap::new();
        for &(i, p) in &parts {
            if p.kind == "bus" {
                let k = bus_of.len();
                bus_of.insert(i, k);
            }
        }
        let kinds = bus_kinds(c, &parts, &bus_of);
        let mut grid = Grid::default();
        let mut order: Vec<(usize, usize)> = bus_of.iter().map(|(&pi, &bi)| (bi, pi)).collect();
        order.sort_unstable();
        let buses: Vec<Bus> = order.iter().map(|&(bi, pi)| Bus::new(&c.parts[pi], kinds[bi], &mut grid)).collect();
        let bus_index = |i: usize| bus_of.get(&i).copied();
        let bus_ref = |p: &CheckedPart, k: &str| p.part(k).and_then(bus_index);
        let mut e = Electrical {
            grid: Grid::default(),
            buses: Vec::new(),
            isl: Islands::default(),
            gens: vec![],
            exts: vec![],
            bats: vec![],
            convs: vec![],
            charger_bat: vec![],
            loads: vec![],
            contactors: vec![],
            breakers: vec![],
            diodes: vec![],
            sspcs: vec![],
            feeders: vec![],
            transformers: vec![],
            ports: vec![],
            custom_links: vec![],
            zones: vec![],
            shed_order: vec![],
            key: vec![],
            scratch_key: vec![],
            pairs: vec![],
            checkpoint: vec![],
            since: 0,
            rollbacks: 0,
        };
        let mut bat_of: HashMap<usize, usize> = HashMap::new();
        let mut link_of: HashMap<usize, LinkRef> = HashMap::new();
        let mut gen_zone: Vec<(usize, usize)> = vec![];
        let mut charger_ref: Vec<Option<usize>> = vec![];
        let mut spread = vec![[0usize; 3]; buses.len()];
        for &(pi, p) in &parts {
            match p.kind {
                "battery" => {
                    let Some(b) = bus_ref(p, "bus") else { continue };
                    if buses[b].kind.ac() {
                        continue;
                    }
                    bat_of.insert(pi, e.bats.len());
                    e.bats.push(Battery::new(p, b, &mut grid, &buses));
                }
                "generator" => {
                    let Some(b) = bus_ref(p, "bus") else { continue };
                    if let Some(z) = p.part("zone") {
                        gen_zone.push((e.gens.len(), z));
                    }
                    e.gens.push(Generator::new(p, b, &mut grid, &buses));
                }
                "ext_power" => {
                    let Some(b) = bus_ref(p, "bus") else { continue };
                    e.exts.push(ExtPower::new(p, b, &mut grid, &buses));
                }
                "contactor" => {
                    let Some(path) = Path::from_part(&mut grid, &buses, &bus_index, p) else { continue };
                    let coil = bus_ref(p, "coil").map(|cb| (cb, Shunt::new(&mut grid, buses[cb].term)));
                    link_of.insert(pi, LinkRef::Contactor(e.contactors.len()));
                    e.contactors.push(Contactor::new(p, path, coil));
                }
                "breaker" => {
                    let Some(path) = Path::from_part(&mut grid, &buses, &bus_index, p) else { continue };
                    link_of.insert(pi, LinkRef::Breaker(e.breakers.len()));
                    e.breakers.push(Breaker::new(p, path));
                }
                "diode" => {
                    let Some(path) = Path::from_part(&mut grid, &buses, &bus_index, p) else { continue };
                    if path.ac {
                        continue;
                    }
                    let d = Diode::new(p, path, &mut grid, &buses);
                    e.diodes.push(d);
                }
                "sspc" => {
                    let Some(path) = Path::from_part(&mut grid, &buses, &bus_index, p) else { continue };
                    link_of.insert(pi, LinkRef::Sspc(e.sspcs.len()));
                    e.sspcs.push(Sspc::new(p, path));
                }
                "feeder" => {
                    let Some(path) = Path::from_part(&mut grid, &buses, &bus_index, p) else { continue };
                    link_of.insert(pi, LinkRef::Feeder(e.feeders.len()));
                    let f = Feeder::new(p, path, &mut grid, &buses);
                    e.feeders.push(f);
                }
                "transformer" => {
                    let ends = p.parts("between");
                    let (Some(a), Some(b)) = (ends.first().copied().and_then(bus_index), ends.get(1).copied().and_then(bus_index)) else { continue };
                    if let Some(t) = Transformer::new(p, &mut grid, &buses, a, b) {
                        e.transformers.push(t);
                    }
                }
                "load" => {
                    let Some(b) = bus_ref(p, "bus") else { continue };
                    e.loads.push(Load::new(p, b, &mut grid, &buses, &mut spread[b]));
                }
                "tru" | "inverter" | "charger" => {
                    let (Some(ib), Some(ob)) = (bus_ref(p, "input"), bus_ref(p, "output")) else { continue };
                    let kind = match p.kind {
                        "tru" => ConvKind::Tru,
                        "inverter" => ConvKind::Inverter,
                        _ => ConvKind::Charger,
                    };
                    let wrong = match kind {
                        ConvKind::Tru => !buses[ib].kind.ac() || buses[ob].kind.ac(),
                        ConvKind::Inverter => buses[ib].kind.ac() || !buses[ob].kind.ac(),
                        ConvKind::Charger => buses[ob].kind.ac(),
                    };
                    if wrong {
                        continue;
                    }
                    charger_ref.push(if kind == ConvKind::Charger { p.part("battery") } else { None });
                    e.convs.push(Converter::new(p, kind, &mut grid, &buses, ib, ob));
                }
                _ => {}
            }
        }
        e.charger_bat = charger_ref.into_iter().map(|r| r.and_then(|x| bat_of.get(&x).copied())).collect();
        for (gi, zp) in gen_zone {
            if let Some(&l) = link_of.get(&zp) {
                let path = match l {
                    LinkRef::Contactor(i) => &e.contactors[i].path,
                    LinkRef::Breaker(i) => &e.breakers[i].path,
                    LinkRef::Sspc(i) => &e.sspcs[i].path,
                    LinkRef::Feeder(i) => &e.feeders[i].path,
                };
                let forward = path.a == e.gens[gi].bus;
                e.gens[gi].zone = Some((0, forward));
                e.zones.push((gi, l));
            }
        }
        let slot = |id: &str, n: &str, k: &str| c.slots.index.get(&format!("{id}.{n}_{k}")).copied();
        for p in c.parts.iter().filter(|p| p.domain == "custom") {
            for port in crate::custom::ports_of(p.kind).into_iter().filter(|x| x.domain == "electrical") {
                let n = port.name;
                if port.link {
                    let ends = p.parts(n);
                    let (Some(a), Some(b)) = (ends.first().copied().and_then(bus_index), ends.get(1).copied().and_then(bus_index)) else { continue };
                    let (Some(g), Some(q)) = (slot(&p.id, n, "g"), slot(&p.id, n, "q")) else { continue };
                    let Some(path) = Path::new(&mut grid, &buses, a, b, "a") else { continue };
                    e.custom_links.push(CustomLink { path, g, q });
                } else {
                    let Some(b) = p.part(n).and_then(bus_index) else { continue };
                    let (Some(se), Some(sg), Some(sv), Some(sq)) = (slot(&p.id, n, "e"), slot(&p.id, n, "g"), slot(&p.id, n, "v"), slot(&p.id, n, "q")) else { continue };
                    let bus = &buses[b];
                    e.ports.push(CustomPort { sh: Shunt::new(&mut grid, bus.term), ac: bus.kind.ac(), n: bus.phases(), bus: b, slot: [se, sg, sv, sq] });
                }
            }
        }
        grid.finish();
        e.grid = grid;
        e.buses = buses;
        e.shed_order = (0..e.loads.len()).filter(|&k| e.loads[k].priority > 0.0).collect();
        e.shed_order.sort_by(|&a, &b| e.loads[b].priority.total_cmp(&e.loads[a].priority));
        Some(e)
    }

    /// Factors and solves, then lets each TRU follow the AC it rectifies this step.
    fn solve(&mut self) -> bool {
        if !self.grid.solve() {
            return false;
        }
        let mut moved = false;
        for cv in &mut self.convs {
            moved |= cv.refresh(&mut self.grid, &self.buses);
        }
        !moved || self.grid.resolve()
    }

    /// Network size, for profiling: unknowns and matrix entries.
    pub fn size(&self) -> (usize, usize) {
        (self.grid.size(), self.grid.sp.nonzeros())
    }

    fn update_links(&mut self, slots: &mut [f64], dt: f64) {
        for x in &mut self.contactors {
            x.update(slots, dt, &self.buses, &self.grid, &self.isl);
        }
        for x in &mut self.breakers {
            x.update(slots, dt);
        }
        for x in &mut self.diodes {
            x.update(&self.grid, slots, dt);
        }
        for x in &mut self.sspcs {
            x.update(&self.grid, slots, dt);
        }
        for x in &mut self.feeders {
            x.update(slots, dt);
        }
        for x in &mut self.transformers {
            x.update(slots);
        }
        for l in &mut self.custom_links {
            let g = slots[l.g];
            if g > 0.0 && g.is_finite() {
                l.path.set(g);
            } else {
                l.path.open();
            }
        }
    }

    fn paths(&self) -> impl Iterator<Item = &Path> {
        self.contactors
            .iter()
            .map(|x| &x.path)
            .chain(self.breakers.iter().map(|x| &x.path))
            .chain(self.diodes.iter().map(|x| &x.path))
            .chain(self.sspcs.iter().map(|x| &x.path))
            .chain(self.feeders.iter().map(|x| &x.path))
            .chain(self.transformers.iter().map(|x| &x.path))
            .chain(self.custom_links.iter().map(|x| &x.path))
    }

    /// Rebuilds the islands when a link opened or closed.
    fn islands(&mut self) {
        let mut now = std::mem::take(&mut self.scratch_key);
        now.clear();
        now.extend(self.paths().map(|p| p.conducts()));
        if now != self.key || self.isl.of.len() != self.buses.len() {
            let mut pairs = std::mem::take(&mut self.pairs);
            pairs.clear();
            pairs.extend(self.paths().filter(|p| p.conducts()).map(|p| (p.a, p.b)));
            self.isl.rebuild(&self.buses, &pairs);
            for (i, b) in self.buses.iter_mut().enumerate() {
                b.island = self.isl.of[i];
            }
            self.pairs = pairs;
            std::mem::swap(&mut self.key, &mut now);
        }
        self.scratch_key = now;
    }

    fn sources(&mut self, slots: &mut [f64], dt: f64) {
        for x in &mut self.gens {
            x.drive(slots, dt);
        }
        for x in &mut self.exts {
            x.update(slots, dt);
        }
        self.isl.begin();
        for x in &self.gens {
            x.offer(&mut self.isl, &self.buses);
        }
        for x in &self.exts {
            x.offer(&mut self.isl, &self.buses);
        }
        for (k, cv) in self.convs.iter_mut().enumerate() {
            let bt = self.charger_bat.get(k).copied().flatten().map(|b| (self.bats[b].temp, self.bats[b].temp > self.bats[b].chem.overheat));
            cv.update(slots, dt, &self.buses, bt);
        }
        for cv in &self.convs {
            let o = self.buses[cv.output].island;
            let cap = cv.capacity();
            if cap > 0.0 {
                self.isl.capacity[o] += cap;
                self.isl.has_capacity[o] = true;
                self.isl.source_va[o] += cap;
                if cv.kind == ConvKind::Inverter {
                    self.isl.offer_reference(o, cv.rating, cv.frequency, cv.theta);
                }
            }
        }
        for b in &mut self.buses {
            b.freq = self.isl.reactance_freq(b.island);
        }
    }

    fn shed(&mut self, slots: &[f64], dt: f64) {
        for l in &mut self.loads {
            l.demand(slots, dt, &self.buses);
            if l.wanted && !l.shed {
                self.isl.demand[self.buses[l.bus].island] += l.va();
            }
            self.isl.nonlinear[self.buses[l.bus].island] += l.harmonics * l.drawn;
        }
        for cv in &self.convs {
            if cv.active {
                self.isl.demand[self.buses[cv.input].island] += cv.input_w;
            }
        }
        for &k in &self.shed_order {
            let isl = self.buses[self.loads[k].bus].island;
            if !self.isl.has_capacity[isl] || !self.loads[k].wanted {
                continue;
            }
            let (cap, dem) = (self.isl.capacity[isl], self.isl.demand[isl]);
            let l = &mut self.loads[k];
            let va = l.va();
            if !l.shed && dem > cap {
                l.shed = true;
                self.isl.demand[isl] -= va;
            } else if l.shed && dem + va <= cap * UNSHED_MARGIN {
                l.shed = false;
                self.isl.demand[isl] += va;
            }
        }
    }

    fn stamp(&mut self, slots: &[f64], dt: f64, first: bool) {
        let g = &mut self.grid;
        g.begin();
        for b in &self.buses {
            b.stamp(g);
        }
        for x in &self.contactors {
            x.stamp(g, &self.buses);
        }
        for x in &self.breakers {
            x.path.stamp(g);
        }
        for x in &self.diodes {
            x.stamp(g);
        }
        for x in &self.sspcs {
            x.path.stamp(g);
        }
        for x in &mut self.feeders {
            let f = self.buses[x.path.a].freq;
            x.stamp(g, f);
        }
        for x in &self.transformers {
            x.stamp(g, self.buses[x.path.a].freq);
        }
        for l in &self.custom_links {
            l.path.stamp(g);
        }
        for x in &mut self.gens {
            x.stamp(g, &self.buses, &self.isl);
        }
        for x in &mut self.exts {
            x.stamp(g, &self.buses, &self.isl);
        }
        for x in &mut self.bats {
            let v = self.buses[x.bus].v[0].re;
            x.stamp(g, slots, dt, v);
        }
        for x in &mut self.convs {
            x.stamp(g, &self.buses, &self.isl);
        }
        for x in &mut self.loads {
            x.stamp(g, &self.buses, dt, first);
        }
        for p in &self.ports {
            let gp = slots[p.slot[1]].max(0.0);
            if gp > 0.0 && gp.is_finite() {
                let ep = slots[p.slot[0]];
                p.sh.each(g, C::real(gp));
                let set = if p.ac { balanced(C::real(ep)) } else { [C::real(ep), ZERO, ZERO] };
                let mut inj = ZERO3;
                for k in 0..p.n {
                    inj[k] = set[k].scale(gp);
                }
                p.sh.inject(g, &inj);
            }
        }
    }

    fn post(&mut self, slots: &mut [f64], dt: f64) {
        let g = &self.grid;
        for b in &mut self.buses {
            let (f, thd) = (self.isl.freq.get(b.island).copied().unwrap_or(0.0), self.isl.thd(b.island));
            b.post(g, slots, dt, f, thd);
        }
        for x in &mut self.contactors {
            x.post(g, slots);
        }
        for x in &mut self.breakers {
            x.post(g, slots, dt);
        }
        for x in &mut self.diodes {
            x.post(g, slots);
        }
        for x in &mut self.sspcs {
            x.post(g, slots, dt);
        }
        for x in &mut self.feeders {
            x.post(g, slots, dt);
        }
        for x in &mut self.transformers {
            let f = self.buses[x.path.a].freq;
            x.post(g, slots, dt, f);
        }
        for l in &mut self.custom_links {
            l.path.read(g);
            slots[l.q] = if l.path.conducts() { if l.path.ac { l.path.i[0].abs() } else { l.path.i[0].re } } else { 0.0 };
        }
        for &(gi, l) in &self.zones {
            let path = match l {
                LinkRef::Contactor(i) => &self.contactors[i].path,
                LinkRef::Breaker(i) => &self.breakers[i].path,
                LinkRef::Sspc(i) => &self.sspcs[i].path,
                LinkRef::Feeder(i) => &self.feeders[i].path,
            };
            let gen = &mut self.gens[gi];
            let forward = gen.zone.is_some_and(|z| z.1);
            let mut i = ZERO3;
            for k in 0..path.phases() {
                i[k] = if forward { path.i[k] } else { -path.i[k] };
            }
            gen.zone_i = i;
        }
        for x in &mut self.gens {
            x.post(g, slots, dt, &self.isl, &self.buses);
        }
        for x in &mut self.exts {
            x.post(g, slots);
        }
        for x in &mut self.bats {
            x.post(g, slots, dt);
        }
        for x in &mut self.convs {
            x.post(g, slots, &self.buses, &mut self.isl);
        }
        for x in &mut self.loads {
            x.post(g, slots, dt, &self.buses);
        }
        for p in &self.ports {
            let v = g.v(&p.sh.t);
            let vm = if p.ac { complex::mean_abs(&v, p.n) } else { v[0].re };
            let _ = p.bus;
            slots[p.slot[2]] = vm;
            slots[p.slot[3]] = slots[p.slot[1]].max(0.0) * (slots[p.slot[0]] - vm);
        }
    }
}

impl Domain for Electrical {
    fn name(&self) -> &'static str {
        "electrical"
    }

    fn step(&mut self, slots: &mut [f64], dt: f64) {
        if self.since == 0 || self.checkpoint.is_empty() {
            self.checkpoint = self.state();
        }
        self.since = (self.since + 1) % CHECKPOINT_EVERY;
        for b in &mut self.buses {
            b.pre(slots, dt);
        }
        self.update_links(slots, dt);
        self.islands();
        self.sources(slots, dt);
        self.shed(slots, dt);
        self.stamp(slots, dt, true);
        let mut ok = self.solve();
        // Diodes switch on this step's own voltages, not the last step's, or a network of them chatters step to step.
        for _ in 0..DIODE_PASSES {
            if !ok {
                break;
            }
            let mut switched = false;
            for x in &mut self.diodes {
                switched |= x.settle(&self.grid);
            }
            if !switched {
                break;
            }
            self.stamp(slots, dt, false);
            ok = self.solve();
        }
        if ok {
            // A power controller in current limit runs linear: solve again with it holding its limit.
            let mut limited = false;
            for s in &mut self.sspcs {
                limited |= s.limit(&self.grid);
            }
            if limited {
                self.stamp(slots, dt, false);
                ok = self.solve();
            }
        }
        if !ok {
            // Throw the step away: back to the last checkpoint, which was finite.
            let cp = std::mem::take(&mut self.checkpoint);
            self.restore(&cp);
            self.checkpoint = cp;
            self.rollbacks += 1;
            return;
        }
        self.post(slots, dt);
    }

    fn state(&self) -> Vec<f64> {
        let mut v = self.grid.state();
        for x in &self.gens {
            x.state(&mut v);
        }
        for x in &self.exts {
            x.state(&mut v);
        }
        for x in &self.bats {
            x.state(&mut v);
        }
        for x in &self.convs {
            x.state(&mut v);
        }
        for x in &self.loads {
            x.state(&mut v);
        }
        for x in &self.contactors {
            x.state(&mut v);
        }
        for x in &self.breakers {
            x.state(&mut v);
        }
        for x in &self.diodes {
            x.state(&mut v);
        }
        for x in &self.sspcs {
            x.state(&mut v);
        }
        for x in &self.feeders {
            x.state(&mut v);
        }
        for x in &self.transformers {
            x.state(&mut v);
        }
        v
    }

    fn restore(&mut self, s: &[f64]) {
        let n = 2 * self.grid.size();
        if s.len() < n {
            return;
        }
        self.grid.restore(&s[..n]);
        let mut it = s[n..].iter().copied();
        for x in &mut self.gens {
            x.restore(&mut it);
        }
        for x in &mut self.exts {
            x.restore(&mut it);
        }
        for x in &mut self.bats {
            x.restore(&mut it);
        }
        for x in &mut self.convs {
            x.restore(&mut it);
        }
        for x in &mut self.loads {
            x.restore(&mut it);
        }
        for x in &mut self.contactors {
            x.restore(&mut it);
        }
        for x in &mut self.breakers {
            x.restore(&mut it);
        }
        for x in &mut self.diodes {
            x.restore(&mut it);
        }
        for x in &mut self.sspcs {
            x.restore(&mut it);
        }
        for x in &mut self.feeders {
            x.restore(&mut it);
        }
        for x in &mut self.transformers {
            x.restore(&mut it);
        }
        // Bus voltages come from the restored solution so loads linearise where the network was.
        let g = &self.grid;
        for b in &mut self.buses {
            b.v = g.v(&b.term);
        }
    }
}
