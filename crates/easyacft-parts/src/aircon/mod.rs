//! Air conditioning and pressurisation: packs, mixers, recirculation, trim air, zones (well mixed or resolved on a
//! grid), the system controller, avionics and cargo ventilation, the pressure vessel, its outflow and safety
//! valves and the cabin pressure controller. Stepped together in one domain so each part sees the others' state.

pub mod cfd;
pub mod controller;
pub mod mixer;
pub mod pack;
pub mod pressurization;
pub mod quality;
pub mod thermo;
pub mod trim;
pub mod ventilation;
pub mod zone;

use crate::{Domain, Expr};
use easyacft_lang::schema::PartSpec;
use easyacft_lang::{Checked, CheckedPart};
use std::collections::HashMap;
use thermo::{humidity_ratio, Stream};

pub static SPECS: &[PartSpec] = &[
    pack::SPEC,
    mixer::MIXER,
    mixer::FAN,
    trim::HOT_AIR,
    trim::TRIM_VALVE,
    zone::SPEC,
    controller::SPEC,
    ventilation::AVIONICS,
    ventilation::CARGO,
    quality::EXTRACT_FAN,
    pressurization::FUSELAGE_SPEC,
    pressurization::VALVE_SPEC,
    pressurization::SAFETY_SPEC,
    pressurization::CONTROLLER_SPEC,
];

pub(crate) fn eval(e: &mut Option<Expr>, slots: &[f64], dt: f64, or: f64) -> f64 {
    e.as_mut().map_or(or, |x| x.eval(slots, dt))
}

pub(crate) fn yes(e: &mut Option<Expr>, slots: &[f64], dt: f64, or: bool) -> bool {
    e.as_mut().map_or(or, |x| x.yes(slots, dt))
}

/// Slot of output `out` on the part a field refers to.
pub(crate) fn slot_of(c: &Checked, p: &CheckedPart, field: &str, out: &str) -> Option<usize> {
    p.part(field).and_then(|i| c.slots.index.get(&format!("{}.{out}", c.parts[i].id)).copied())
}

/// Outside relative humidity assumed for air the parts take in when nothing else says.
const AMBIENT_RH: f64 = 0.5;
/// How often a healthy state is kept to fall back on, in steps.
const CHECKPOINT_EVERY: u32 = 25;

pub struct AirCon {
    packs: Vec<pack::Pack>,
    mixers: Vec<mixer::Mixer>,
    fans: Vec<mixer::Fan>,
    hot_air: Vec<trim::HotAir>,
    trims: Vec<trim::TrimValve>,
    zones: Vec<zone::Zone>,
    controllers: Vec<controller::Controller>,
    avionics: Vec<ventilation::Avionics>,
    cargo: Vec<ventilation::Cargo>,
    extracts: Vec<quality::ExtractFan>,
    fuselages: Vec<pressurization::Fuselage>,
    valves: Vec<pressurization::OutflowValve>,
    safety: Vec<pressurization::SafetyValve>,
    cpcs: Vec<pressurization::Controller>,
    /// Which mixer each pack feeds.
    pack_mixer: Vec<Option<usize>>,
    /// Which controller drives each outflow valve.
    valve_cpc: Vec<Option<usize>>,
    /// Output slot ranges this domain writes, for the blowup check.
    outputs: Vec<(usize, usize)>,
    grid_turn: usize,
    checkpoint: Vec<f64>,
    since_checkpoint: u32,
    pub recoveries: u32,
}

fn index_of(c: &Checked, domain: &str, kind: &str) -> HashMap<usize, usize> {
    c.parts.iter().enumerate().filter(|(_, p)| p.domain == domain && p.kind == kind).enumerate().map(|(k, (i, _))| (i, k)).collect()
}

impl AirCon {
    pub fn new(c: &Checked) -> Option<Self> {
        let ours = |p: &CheckedPart| p.domain == "aircon" || p.domain == "pressurization";
        if !c.parts.iter().any(ours) {
            return None;
        }
        let ix = |kind: &str, domain: &str| index_of(c, domain, kind);
        let (packs_ix, mixers_ix, fans_ix, hot_ix, trims_ix, zones_ix) = (ix("pack", "aircon"), ix("mixer", "aircon"), ix("recirc_fan", "aircon"), ix("hot_air", "aircon"), ix("trim_valve", "aircon"), ix("zone", "aircon"));
        let (fus_ix, valves_ix) = (ix("fuselage", "pressurization"), ix("outflow_valve", "pressurization"));
        let map = |m: &HashMap<usize, usize>, refs: Vec<usize>| refs.into_iter().filter_map(|r| m.get(&r).copied()).collect::<Vec<_>>();
        let one = |m: &HashMap<usize, usize>, r: Option<usize>| r.and_then(|r| m.get(&r).copied());
        let mut a = AirCon {
            packs: vec![],
            mixers: vec![],
            fans: vec![],
            hot_air: vec![],
            trims: vec![],
            zones: vec![],
            controllers: vec![],
            avionics: vec![],
            cargo: vec![],
            extracts: vec![],
            fuselages: vec![],
            valves: vec![],
            safety: vec![],
            cpcs: vec![],
            pack_mixer: vec![],
            valve_cpc: vec![],
            outputs: vec![],
            grid_turn: 0,
            checkpoint: vec![],
            since_checkpoint: 0,
            recoveries: 0,
        };
        for p in c.parts.iter().filter(|p| ours(p)) {
            let n = crate::catalog::all().iter().find(|s| s.domain == p.domain && s.kind == p.kind).map_or(0, |s| s.outputs.len());
            a.outputs.push((p.slot, n));
            match (p.domain, p.kind) {
                ("pressurization", "fuselage") => {
                    let mixers = map(&mixers_ix, p.parts("mixers"));
                    a.fuselages.extend(pressurization::Fuselage::new(c, p, mixers));
                }
                ("aircon", "pack") => a.packs.extend(pack::Pack::new(c, p)),
                ("aircon", "mixer") => {
                    let Some(f) = one(&fus_ix, p.part("fuselage")) else { continue };
                    a.mixers.push(mixer::Mixer::new(c, p, map(&packs_ix, p.parts("packs")), map(&fans_ix, p.parts("recirculation")), f));
                }
                ("aircon", "extract_fan") => a.extracts.push(quality::ExtractFan::new(p, map(&zones_ix, p.parts("zones")))),
                ("aircon", "recirc_fan") => a.fans.push(mixer::Fan::new(p, map(&zones_ix, p.parts("zones")))),
                ("aircon", "hot_air") => {
                    let Some(f) = one(&fus_ix, p.part("fuselage")) else { continue };
                    a.hot_air.extend(trim::HotAir::new(c, p, f));
                }
                ("aircon", "trim_valve") => {
                    let Some(h) = one(&hot_ix, p.part("hot_air")) else { continue };
                    a.trims.push(trim::TrimValve::new(p, h));
                }
                ("aircon", "zone") => {
                    let (Some(m), Some(f)) = (one(&mixers_ix, p.part("mixer")), one(&fus_ix, p.part("fuselage"))) else { continue };
                    a.zones.extend(zone::Zone::new(c, p, m, f, one(&trims_ix, p.part("trim_valve"))));
                }
                ("aircon", "controller") => a.controllers.push(controller::Controller::new(p, map(&packs_ix, p.parts("packs")), map(&zones_ix, p.parts("zones")), map(&hot_ix, p.parts("hot_air")))),
                ("aircon", "avionics_ventilation") => {
                    let Some(f) = one(&fus_ix, p.part("fuselage")) else { continue };
                    a.avionics.extend(ventilation::Avionics::new(c, p, f));
                }
                ("aircon", "cargo") => {
                    let Some(f) = one(&fus_ix, p.part("fuselage")) else { continue };
                    a.cargo.extend(ventilation::Cargo::new(c, p, f, one(&hot_ix, p.part("hot_air"))));
                }
                ("pressurization", "outflow_valve") => {
                    let Some(f) = one(&fus_ix, p.part("fuselage")) else { continue };
                    a.valves.push(pressurization::OutflowValve::new(p, f));
                }
                ("pressurization", "safety_valve") => {
                    let Some(f) = one(&fus_ix, p.part("fuselage")) else { continue };
                    a.safety.push(pressurization::SafetyValve::new(p, f));
                }
                ("pressurization", "controller") => {
                    let Some(f) = one(&fus_ix, p.part("fuselage")) else { continue };
                    a.cpcs.extend(pressurization::Controller::new(c, p, f, map(&valves_ix, p.parts("valves"))));
                }
                _ => {}
            }
        }
        a.pack_mixer = (0..a.packs.len()).map(|k| a.mixers.iter().position(|m| m.packs.contains(&k))).collect();
        a.valve_cpc = (0..a.valves.len()).map(|k| a.cpcs.iter().position(|c| c.valves.contains(&k))).collect();
        let mut turn = 0;
        for z in a.zones.iter_mut().filter(|z| z.has_grid()) {
            z.grid_turn = turn;
            turn += 1;
        }
        Some(a)
    }

    fn grids(&self) -> usize {
        self.zones.iter().filter(|z| z.has_grid()).count()
    }

    /// Grid temperatures across the middle of a zone, for diagrams.
    pub fn zone_section(&self, zone: usize) -> Option<Vec<Vec<Option<f64>>>> {
        self.zones.get(zone).and_then(|z| z.section())
    }

    fn finite(&self, slots: &[f64]) -> bool {
        self.outputs.iter().all(|&(base, n)| slots[base..base + n].iter().all(|v| v.is_finite()))
    }

    fn run(&mut self, slots: &mut [f64], dt: f64) {
        // Cabin pressure controllers command their valves from the vessel as it stands.
        for cpc in &mut self.cpcs {
            let f = &self.fuselages[cpc.fuselage];
            let cabin = (f.p, thermo::isa_altitude(f.p), f.vs);
            cpc.step(slots, dt, cabin);
        }
        let mut valve_out = vec![0.0; self.fuselages.len()];
        for (k, v) in self.valves.iter_mut().enumerate() {
            if let Some(ci) = self.valve_cpc[k] {
                let cpc = &self.cpcs[ci];
                v.target = cpc.command;
                v.channel = if cpc.manual_now { 3 } else { cpc.channel.max(1) };
                v.manual_rate = cpc.manual_switch;
            }
            let f = &self.fuselages[v.fuselage];
            let outside = f.ambient(slots);
            valve_out[v.fuselage] += v.step(slots, dt, (f.p, f.t), outside);
        }
        for sv in &mut self.safety {
            let f = &self.fuselages[sv.fuselage];
            let outside = f.ambient(slots);
            valve_out[sv.fuselage] += sv.step(slots, dt, (f.p, f.t), outside);
        }
        // Trim air: manifold, then the valves drawing on it.
        for h in &mut self.hot_air {
            let f = &self.fuselages[h.fuselage];
            let (pa, ta) = f.ambient(slots);
            h.step(slots, dt, f.p, humidity_ratio(AMBIENT_RH, ta, pa));
            h.drawn = 0.0;
        }
        for (k, tv) in self.trims.iter_mut().enumerate() {
            let zone = self.zones.iter().find(|z| z.trim == Some(k));
            let duct_p = zone.map_or(thermo::P_SL, |z| self.mixers[z.mixer].p);
            if let Some(z) = zone {
                tv.command = z.trim_command;
            }
            let h = &self.hot_air[tv.hot_air];
            tv.step(slots, dt, (h.p, h.t, h.w), duct_p);
        }
        for tv in &self.trims {
            self.hot_air[tv.hot_air].drawn += tv.out.flow;
        }
        // Packs deliver into their mixer's pressure.
        for (k, p) in self.packs.iter_mut().enumerate() {
            if let Some(m) = self.pack_mixer[k] {
                p.downstream = self.mixers[m].p;
            }
            p.step(slots, dt);
        }
        // Recirculation fans draw the zones' exhaust air.
        for fan in &mut self.fans {
            let from: Vec<&zone::Zone> = if fan.zones.is_empty() { self.zones.iter().collect() } else { fan.zones.iter().filter_map(|&z| self.zones.get(z)).collect() };
            let mut streams: Vec<Stream> = from.iter().map(|z| z.exhaust).collect();
            if streams.iter().all(|s| s.flow <= 0.0) {
                streams = from.iter().map(|z| Stream { flow: 1.0, ..z.exhaust }).collect();
            }
            let intake = thermo::mix(&streams);
            fan.step(slots, dt, intake);
        }
        let zone_co2 = |zs: &[zone::Zone]| {
            let w: f64 = zs.iter().map(|z| z.exhaust.flow).sum();
            if w > 1e-9 { zs.iter().map(|z| z.exhaust.flow * z.exhaust_co2).sum::<f64>() / w } else { zs.iter().map(|z| z.exhaust_co2).sum::<f64>() / zs.len().max(1) as f64 }
        };
        let recirc_co2 = zone_co2(&self.zones);
        let zone_ozone = {
            let w: f64 = self.zones.iter().map(|z| z.exhaust.flow).sum();
            if w > 1e-9 { self.zones.iter().map(|z| z.exhaust.flow * z.ozone.ppb).sum::<f64>() / w } else { 30.0 }
        };
        for m in &mut self.mixers {
            let (mut num, mut den) = (0.0, 0.0);
            for &k in &m.packs {
                num += self.packs[k].out.flow * self.packs[k].ozone_out;
                den += self.packs[k].out.flow;
            }
            let recirc: f64 = m.fans.iter().map(|&k| self.fans[k].out.flow).sum();
            num += recirc * zone_ozone;
            den += recirc;
            m.ozone = if den > 1e-9 { num / den } else { m.ozone };
            let f = &self.fuselages[m.fuselage];
            let (pa, ta) = f.ambient(slots);
            let packs: Vec<Stream> = m.packs.iter().map(|&k| self.packs[k].out).collect();
            let recirc: Vec<Stream> = m.fans.iter().map(|&k| self.fans[k].out).collect();
            m.step(slots, dt, f.p, &packs, &recirc, recirc_co2, humidity_ratio(AMBIENT_RH, ta, pa));
        }
        // System controllers set pack demand from the zones.
        for ctl in &mut self.controllers {
            let demands: Vec<f64> = ctl.zones.iter().filter_map(|&z| self.zones.get(z)).map(|z| z.duct_demand).collect();
            let operating = ctl.packs.iter().filter(|&&k| self.packs.get(k).is_some_and(|p| p.out.flow > 0.02)).count();
            ctl.step(slots, dt, &demands, operating);
        }
        for z in &mut self.zones {
            z.extract = 0.0;
        }
        for fan in &mut self.extracts {
            let each = fan.step(slots, dt);
            for &zi in &fan.zones {
                if let Some(z) = self.zones.get_mut(zi) {
                    z.extract += each;
                }
            }
        }
        // Zones take their share of their mixer's air plus their trim air.
        let mut share_sum = vec![0.0; self.mixers.len()];
        for z in &self.zones {
            share_sum[z.mixer] += z.share;
        }
        let grids = self.grids().max(1);
        let turn = self.grid_turn % grids;
        for z in &mut self.zones {
            let m = &self.mixers[z.mixer];
            let share = if share_sum[z.mixer] > 0.0 { z.share / share_sum[z.mixer] } else { 0.0 };
            let supply = m.out.with_flow(m.out.flow * share);
            let trim = z.trim.map_or(Stream::none(m.t, m.p), |t| self.trims[t].out);
            let cabin_p = self.fuselages[z.fuselage].p;
            let now = z.has_grid() && z.grid_turn == turn;
            z.step(slots, dt, supply, m.co2, m.ozone, trim, cabin_p, now);
        }
        self.grid_turn = self.grid_turn.wrapping_add(1);
        // The vessel: fresh air in, valves and leaks out; zones set its temperature.
        for (k, f) in self.fuselages.iter_mut().enumerate() {
            f.prime(slots);
            let (mut fresh, mut h) = (0.0, 0.0);
            for &mi in &f.mixers {
                let m = &self.mixers[mi];
                let fl = m.fresh;
                fresh += fl;
                h += fl * m.t;
            }
            let t_mix = if fresh > 1e-9 { h / fresh } else { f.t };
            let zs: Vec<&zone::Zone> = self.zones.iter().filter(|z| z.fuselage == k).collect();
            let zone_t = (!zs.is_empty()).then(|| zs.iter().map(|z| z.t).sum::<f64>() / zs.len() as f64);
            f.step(slots, dt, (fresh, t_mix), zone_t, valve_out[k]);
        }
        for av in &mut self.avionics {
            let t = self.fuselages[av.fuselage].t;
            av.step(slots, dt, t);
        }
        for cg in &mut self.cargo {
            let f = &self.fuselages[cg.fuselage];
            let hot = cg.hot_air.map(|h| (self.hot_air[h].p, self.hot_air[h].t));
            cg.step(slots, dt, (f.p, f.t), hot);
        }
        for cg in &self.cargo {
            if let Some(h) = cg.hot_air {
                self.hot_air[h].drawn += cg.hot_flow;
            }
        }
    }

    fn scalar_state(&self) -> Vec<f64> {
        let mut v = Vec::new();
        self.packs.iter().for_each(|x| x.state(&mut v));
        self.mixers.iter().for_each(|x| x.state(&mut v));
        self.fans.iter().for_each(|x| x.state(&mut v));
        self.hot_air.iter().for_each(|x| x.state(&mut v));
        self.trims.iter().for_each(|x| x.state(&mut v));
        self.controllers.iter().for_each(|x| x.state(&mut v));
        self.avionics.iter().for_each(|x| x.state(&mut v));
        self.cargo.iter().for_each(|x| x.state(&mut v));
        self.extracts.iter().for_each(|x| x.state(&mut v));
        self.fuselages.iter().for_each(|x| x.state(&mut v));
        self.valves.iter().for_each(|x| x.state(&mut v));
        self.safety.iter().for_each(|x| x.state(&mut v));
        self.cpcs.iter().for_each(|x| x.state(&mut v));
        self.zones.iter().for_each(|x| x.scalar_state(&mut v));
        v
    }

    fn restore_scalar(&mut self, it: &mut impl Iterator<Item = f64>) {
        self.packs.iter_mut().for_each(|x| x.restore(it));
        self.mixers.iter_mut().for_each(|x| x.restore(it));
        self.fans.iter_mut().for_each(|x| x.restore(it));
        self.hot_air.iter_mut().for_each(|x| x.restore(it));
        self.trims.iter_mut().for_each(|x| x.restore(it));
        self.controllers.iter_mut().for_each(|x| x.restore(it));
        self.avionics.iter_mut().for_each(|x| x.restore(it));
        self.cargo.iter_mut().for_each(|x| x.restore(it));
        self.extracts.iter_mut().for_each(|x| x.restore(it));
        self.fuselages.iter_mut().for_each(|x| x.restore(it));
        self.valves.iter_mut().for_each(|x| x.restore(it));
        self.safety.iter_mut().for_each(|x| x.restore(it));
        self.cpcs.iter_mut().for_each(|x| x.restore(it));
        self.zones.iter_mut().for_each(|x| x.restore_scalar(it));
    }
}

impl Domain for AirCon {
    fn name(&self) -> &'static str {
        "aircon"
    }

    fn step(&mut self, slots: &mut [f64], dt: f64) {
        if self.checkpoint.is_empty() {
            self.checkpoint = self.scalar_state();
        }
        self.run(slots, dt);
        if self.finite(slots) {
            self.since_checkpoint += 1;
            if self.since_checkpoint >= CHECKPOINT_EVERY {
                self.checkpoint = self.scalar_state();
                self.since_checkpoint = 0;
            }
            return;
        }
        // Something went non-finite: fall back to the last healthy state and settle the grids.
        self.recoveries += 1;
        let saved = std::mem::take(&mut self.checkpoint);
        self.restore_scalar(&mut saved.iter().copied());
        self.checkpoint = saved;
        for z in &mut self.zones {
            z.settle();
        }
        for &(base, n) in &self.outputs {
            for v in &mut slots[base..base + n] {
                if !v.is_finite() {
                    *v = 0.0;
                }
            }
        }
    }

    fn state(&self) -> Vec<f64> {
        let mut v = self.scalar_state();
        self.zones.iter().for_each(|z| z.grid_state(&mut v));
        v.push(self.grid_turn as f64);
        v
    }

    fn restore(&mut self, s: &[f64]) {
        let mut it = s.iter().copied();
        self.restore_scalar(&mut it);
        self.zones.iter_mut().for_each(|z| z.grid_restore(&mut it));
        self.grid_turn = it.next().unwrap_or(0.0) as usize;
    }
}
