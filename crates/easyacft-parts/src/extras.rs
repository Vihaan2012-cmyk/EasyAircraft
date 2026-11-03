//! Failure triggers, fire protection, lights, sound events and logic values.

use crate::util::{Packer, Slots};
use crate::{expr_of, Domain, Expr};
use easyacft_core::Dim;
use easyacft_lang::schema::{opt, req, Kind, PartSpec};
use easyacft_lang::{Checked, Type};

const B: Type = Type::Bool;
const N: Type = Type::Num(Dim::NONE);
const WHEN: Kind = Kind::Expr(Type::Bool);

pub static SPECS: &[PartSpec] = &[
    PartSpec {
        domain: "failure",
        kind: "trigger",
        help: "Fails a part when a condition holds, now or after a delay.",
        fields: &[
            req("part", Kind::Ref(&["*"]), "the part to fail"),
            req("when", WHEN, "for example \"delay(eng1.egt > 1300 K, 10 s)\""),
            opt("latched", Kind::Bool, "true", "stays failed after the condition clears"),
        ],
        outputs: &[("active", B)],
    },
    PartSpec {
        domain: "fire",
        kind: "zone",
        help: "A fire zone with two detection loops.",
        fields: &[
            req("fire_when", WHEN, "what sets it alight, for example a failure trigger or \"eng1.egt > 1500 K\""),
            opt("loop_a_failed", WHEN, "false", "loop A open or shorted"),
            opt("loop_b_failed", WHEN, "false", "loop B open or shorted"),
            opt("extinguish_time", Kind::Quantity(Dim::TIME), "3 s", "time agent must be present to put it out"),
        ],
        outputs: &[("burning", B), ("detected", B), ("loop_fault", B), ("agent", N)],
    },
    PartSpec {
        domain: "fire",
        kind: "bottle",
        help: "An extinguisher bottle discharged into one or more zones.",
        fields: &[
            req("zones", Kind::Refs(&["fire.zone"]), "zones it can reach"),
            req("discharge_when", WHEN, "the squib, for example \"agent1_pb.pressed\""),
            opt("duration", Kind::Quantity(Dim::TIME), "10 s", "time the agent stays effective"),
        ],
        outputs: &[("discharged", B), ("pressure", N), ("failed", B)],
    },
    PartSpec {
        domain: "light",
        kind: "unit",
        help: "A light: steady, beacon, strobe or double strobe, dimming with its bus.",
        fields: &[
            opt("bus", Kind::Ref(&["electrical.bus"]), "", "bus that powers it; none for always powered"),
            opt("on_when", WHEN, "false", "switched on"),
            opt("brightness", Kind::Expr(N), "1", "0 to 1"),
            opt("pattern", Kind::Text, "steady", "steady, beacon, strobe or double"),
        ],
        outputs: &[("lit", B), ("brightness", N), ("failed", B)],
    },
    PartSpec {
        domain: "sound",
        kind: "event",
        help: "A sound cue for FMOD or X-Plane sounds: a pulse on each rising edge and a continuous value.",
        fields: &[opt("trigger", WHEN, "false", "plays on each rising edge"), opt("value", Kind::Expr(N), "0", "continuous level, such as a pump's speed share")],
        outputs: &[("play", B), ("count", N), ("value", N)],
    },
    PartSpec {
        domain: "logic",
        kind: "value",
        help: "A named number from an expression; divide by a unit to get a plain number: \"dc1.voltage / 1 V\".",
        fields: &[req("value", Kind::Expr(N), "an expression giving a plain number")],
        outputs: &[("value", N)],
    },
];

struct Zone {
    fire: Option<Expr>,
    loops: [Option<Expr>; 2],
    time: f64,
    burning: bool,
    agent_s: f64,
    s: Slots,
}

struct Bottle {
    zones: Vec<usize>,
    when: Option<Expr>,
    duration: f64,
    left: f64,
    used: bool,
    s: Slots,
}

struct Light {
    bus: Option<(usize, f64)>,
    on: Option<Expr>,
    level: Option<Expr>,
    pattern: u8,
    phase: f64,
    s: Slots,
}

pub struct Extras {
    triggers: Vec<(usize, Option<Expr>, bool, bool, Slots)>,
    zones: Vec<Zone>,
    bottles: Vec<Bottle>,
    lights: Vec<Light>,
    sounds: Vec<(Option<Expr>, Option<Expr>, bool, f64, Slots)>,
    values: Vec<(Option<Expr>, Slots)>,
}

impl Extras {
    pub fn new(c: &Checked) -> Option<Self> {
        let mut x = Extras { triggers: vec![], zones: vec![], bottles: vec![], lights: vec![], sounds: vec![], values: vec![] };
        let zone_index: Vec<usize> = c.parts.iter().enumerate().filter(|(_, p)| p.domain == "fire" && p.kind == "zone").map(|(i, _)| i).collect();
        for p in &c.parts {
            match (p.domain, p.kind) {
                ("failure", "trigger") => {
                    let Some(t) = p.part("part").and_then(|i| c.slots.index.get(&format!("{}.failed", c.parts[i].id)).copied()) else { continue };
                    x.triggers.push((t, expr_of(p, "when"), p.flag("latched"), false, Slots::of(p, &["active"])));
                }
                ("fire", "zone") => x.zones.push(Zone {
                    fire: expr_of(p, "fire_when"),
                    loops: [expr_of(p, "loop_a_failed"), expr_of(p, "loop_b_failed")],
                    time: p.num("extinguish_time").max(0.1),
                    burning: false,
                    agent_s: 0.0,
                    s: Slots::of(p, &["burning", "detected", "loop_fault", "agent"]),
                }),
                ("fire", "bottle") => x.bottles.push(Bottle {
                    zones: p.parts("zones").iter().filter_map(|z| zone_index.iter().position(|i| i == z)).collect(),
                    when: expr_of(p, "discharge_when"),
                    duration: p.num("duration").max(0.1),
                    left: 0.0,
                    used: false,
                    s: Slots::of(p, &["discharged", "pressure", "failed"]),
                }),
                ("light", "unit") => {
                    let bus = p.part("bus").and_then(|i| {
                        let b = &c.parts[i];
                        Some((*c.slots.index.get(&format!("{}.voltage", b.id))?, b.num("nominal").max(1.0)))
                    });
                    x.lights.push(Light {
                        bus,
                        on: expr_of(p, "on_when"),
                        level: expr_of(p, "brightness"),
                        pattern: match p.text("pattern") {
                            "beacon" => 1,
                            "strobe" => 2,
                            "double" => 3,
                            _ => 0,
                        },
                        phase: 0.0,
                        s: Slots::of(p, &["lit", "brightness", "failed"]),
                    });
                }
                ("sound", "event") => x.sounds.push((expr_of(p, "trigger"), expr_of(p, "value"), false, 0.0, Slots::of(p, &["play", "count", "value"]))),
                ("logic", "value") => x.values.push((expr_of(p, "value"), Slots::of(p, &["value"]))),
                _ => {}
            }
        }
        let empty = x.triggers.is_empty() && x.zones.is_empty() && x.bottles.is_empty() && x.lights.is_empty() && x.sounds.is_empty() && x.values.is_empty();
        (!empty).then_some(x)
    }
}

impl Domain for Extras {
    fn name(&self) -> &'static str {
        "extras"
    }

    fn step(&mut self, slots: &mut [f64], dt: f64) {
        for (target, when, latched, fired, s) in &mut self.triggers {
            let now = when.as_mut().is_some_and(|e| e.yes(slots, dt));
            if now {
                *fired = true;
                slots[*target] = 1.0;
            } else if *fired && !*latched {
                *fired = false;
                slots[*target] = 0.0;
            }
            s.flag(slots, "active", *fired);
        }
        let mut agent = vec![false; self.zones.len()];
        for b in &mut self.bottles {
            if !b.used && !b.s.on(slots, "failed") && b.when.as_mut().is_some_and(|e| e.yes(slots, dt)) {
                b.used = true;
                b.left = b.duration;
            }
            if b.left > 0.0 {
                b.left = (b.left - dt).max(0.0);
                for &z in &b.zones {
                    agent[z] = true;
                }
            }
            b.s.flag(slots, "discharged", b.used);
            b.s.put(slots, "pressure", if b.used { 0.0 } else { 1.0 });
        }
        for (k, z) in self.zones.iter_mut().enumerate() {
            let source = z.fire.as_mut().is_some_and(|e| e.yes(slots, dt));
            if source && !z.burning && z.agent_s < z.time {
                z.burning = true;
            }
            z.agent_s = if agent[k] { z.agent_s + dt } else { 0.0 };
            if z.burning && z.agent_s >= z.time {
                z.burning = false;
            }
            let fa = z.loops[0].as_mut().is_some_and(|e| e.yes(slots, dt));
            let fb = z.loops[1].as_mut().is_some_and(|e| e.yes(slots, dt));
            let detected = z.burning && !(fa && fb);
            z.s.flag(slots, "burning", z.burning);
            z.s.flag(slots, "detected", detected);
            z.s.flag(slots, "loop_fault", fa || fb);
            z.s.put(slots, "agent", agent[k] as u8 as f64);
        }
        for l in &mut self.lights {
            let power = l.bus.map_or(1.0, |(v, nominal)| (slots[v] / nominal).clamp(0.0, 1.0));
            let on = !l.s.on(slots, "failed") && l.on.as_mut().is_some_and(|e| e.yes(slots, dt)) && power > 0.5;
            l.phase = (l.phase + dt) % if l.pattern == 1 { 1.2 } else { 1.0 };
            let flash = match l.pattern {
                0 => true,
                1 => l.phase < 0.15,
                2 => l.phase < 0.06,
                _ => l.phase < 0.05 || (0.15..0.2).contains(&l.phase),
            };
            let level = l.level.as_mut().map_or(1.0, |e| e.eval(slots, dt)).clamp(0.0, 1.0) * power;
            l.s.flag(slots, "lit", on && flash);
            l.s.put(slots, "brightness", if on && flash { level } else { 0.0 });
        }
        for (trig, val, last, count, s) in &mut self.sounds {
            let now = trig.as_mut().is_some_and(|e| e.yes(slots, dt));
            let play = now && !*last;
            *last = now;
            if play {
                *count += 1.0;
            }
            s.flag(slots, "play", play);
            s.put(slots, "count", *count);
            s.put(slots, "value", val.as_mut().map_or(0.0, |e| e.eval(slots, dt)));
        }
        for (e, s) in &mut self.values {
            let v = e.as_mut().map_or(0.0, |x| x.eval(slots, dt));
            s.put(slots, "value", v);
        }
    }

    fn state(&self) -> Vec<f64> {
        let mut p = Packer::default();
        self.triggers.iter().for_each(|t| p.put(t.3 as u8 as f64));
        for z in &self.zones {
            p.put(z.burning as u8 as f64);
            p.put(z.agent_s);
        }
        for b in &self.bottles {
            p.put(b.used as u8 as f64);
            p.put(b.left);
        }
        self.lights.iter().for_each(|l| p.put(l.phase));
        for s in &self.sounds {
            p.put(s.2 as u8 as f64);
            p.put(s.3);
        }
        p.0
    }

    fn restore(&mut self, s: &[f64]) {
        let mut p = Packer(s.to_vec(), 0);
        self.triggers.iter_mut().for_each(|t| t.3 = p.take() != 0.0);
        for z in &mut self.zones {
            z.burning = p.take() != 0.0;
            z.agent_s = p.take();
        }
        for b in &mut self.bottles {
            b.used = p.take() != 0.0;
            b.left = p.take();
        }
        self.lights.iter_mut().for_each(|l| l.phase = p.take());
        for s in &mut self.sounds {
            s.2 = p.take() != 0.0;
            s.3 = p.take();
        }
    }
}
