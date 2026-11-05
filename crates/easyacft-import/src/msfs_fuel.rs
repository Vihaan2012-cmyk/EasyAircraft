//! MSFS `[FUEL_SYSTEM]` version 5 graphs: tanks, lines, pumps, valves, junctions and feeds.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write;

const GAL: f64 = 3.785_411_784;
const FT: f64 = 0.3048;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Comp {
    Tank,
    Pump,
    Valve,
    Junction,
    Engine(usize),
    Apu,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Graph {
    pub tanks: Vec<(String, String, f64, f64, f64)>,
    pub nodes: Vec<String>,
    pub pumps: Vec<(String, String, String, f64)>,
    pub valves: Vec<(String, String, String, f64)>,
    pub pipes: Vec<(String, String, String)>,
    pub feeds: Vec<(String, String, Option<usize>)>,
    pub lines: usize,
    pub skipped: Vec<String>,
}

fn fields(v: &str) -> HashMap<String, String> {
    v.split('#').filter_map(|f| f.split_once(':')).map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_owned())).collect()
}

pub fn id(name: &str) -> String {
    let mut out = String::new();
    let mut prev_lower = false;
    for c in name.chars() {
        if c.is_ascii_uppercase() && prev_lower {
            out.push('_');
        }
        prev_lower = c.is_ascii_lowercase() || c.is_ascii_digit();
        out.push(if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '_' });
    }
    while out.contains("__") {
        out = out.replace("__", "_");
    }
    let out = out.trim_matches('_').to_owned();
    if out.is_empty() || out.starts_with(|c: char| c.is_ascii_digit()) {
        format!("f_{out}")
    } else {
        out
    }
}

/// Reads the graph from a `[FUEL_SYSTEM]` section; None when it is not version 5 or has no lines.
pub fn read(sys: &HashMap<String, String>, density: f64) -> Option<Graph> {
    if sys.get("version").and_then(|v| v.trim().parse::<f64>().ok()).unwrap_or(0.0) < 5.0 {
        return None;
    }
    let sorted = |prefix: &str| {
        let mut v: Vec<(u32, HashMap<String, String>)> = sys.iter().filter_map(|(k, v)| k.strip_prefix(prefix).and_then(|n| n.parse().ok()).map(|n| (n, fields(v)))).collect();
        v.sort_by_key(|x| x.0);
        v.into_iter().map(|x| x.1).collect::<Vec<_>>()
    };
    let mut kind: HashMap<String, Comp> = HashMap::new();
    let mut g = Graph::default();
    for t in sorted("tank.") {
        let Some(name) = t.get("name").cloned() else { continue };
        let cap = t.get("capacity").and_then(|c| c.parse::<f64>().ok()).unwrap_or(0.0);
        let pos: Vec<f64> = t.get("position").map(|p| p.split(',').filter_map(|x| x.trim().parse().ok()).collect()).unwrap_or_default();
        let unusable = t.get("unusablecapacity").and_then(|c| c.parse::<f64>().ok()).unwrap_or(0.0);
        kind.insert(name.clone(), Comp::Tank);
        g.tanks.push((id(&name), t.get("title").cloned().unwrap_or_else(|| name.clone()), cap * GAL * density, -pos.first().copied().unwrap_or(0.0) * FT, unusable * GAL * density));
    }
    for (k, comp) in [("pump.", Comp::Pump), ("valve.", Comp::Valve), ("junction.", Comp::Junction)] {
        for x in sorted(k) {
            if let Some(n) = x.get("name") {
                kind.insert(n.clone(), comp);
            }
        }
    }
    for e in sorted("engine.") {
        if let Some(n) = e.get("name") {
            let idx = e.get("index").and_then(|i| i.parse().ok()).unwrap_or(1);
            kind.insert(n.clone(), Comp::Engine(idx));
        }
    }
    for a in sorted("apu.") {
        kind.insert(a.get("name").cloned().unwrap_or_else(|| "APU".into()), Comp::Apu);
    }
    let lines: Vec<(String, String, String)> = sorted("line.").into_iter().filter_map(|l| Some((l.get("name")?.clone(), l.get("source")?.clone(), l.get("destination")?.clone()))).collect();
    if lines.is_empty() {
        return None;
    }
    g.lines = lines.len();
    let node_of = |name: &str| -> Option<String> {
        match kind.get(name)? {
            Comp::Tank => Some(id(name)),
            Comp::Junction => Some(id(name)),
            Comp::Pump | Comp::Valve => Some(format!("{}_out", id(name))),
            Comp::Engine(_) | Comp::Apu => Some(format!("{}_inlet", id(name))),
        }
    };
    let mut nodes: BTreeMap<String, ()> = BTreeMap::new();
    for (name, c) in &kind {
        if *c != Comp::Tank {
            nodes.insert(node_of(name).unwrap(), ());
        }
    }
    g.nodes = nodes.into_keys().collect();
    let mut valve_inputs: HashMap<String, usize> = HashMap::new();
    let opening: HashMap<String, f64> = sorted("valve.").into_iter().filter_map(|v| Some((v.get("name")?.clone(), v.get("openingtime").and_then(|t| t.parse().ok()).unwrap_or(0.0)))).collect();
    let pumps = sorted("pump.");
    for p in &pumps {
        let (Some(name), Some(tank)) = (p.get("name"), p.get("tankfuelrequired")) else { continue };
        if kind.get(tank) != Some(&Comp::Tank) {
            g.skipped.push(format!("pump {name}: its fuel comes from `{tank}`, not a tank"));
            continue;
        }
        let psi = p.get("pressure").and_then(|x| x.parse().ok()).unwrap_or(25.0);
        g.pumps.push((id(name), id(tank), format!("{}_out", id(name)), psi));
    }
    for (line, src, dst) in &lines {
        let Some(from) = node_of(src) else {
            g.skipped.push(format!("line {line}: unknown source `{src}`"));
            continue;
        };
        match kind.get(dst.as_str()) {
            Some(Comp::Valve) => {
                let n = valve_inputs.entry(dst.clone()).or_insert(0);
                *n += 1;
                let vid = if *n == 1 { id(dst) } else { format!("{}_{}", id(dst), n) };
                g.valves.push((vid, from, format!("{}_out", id(dst)), opening.get(dst).copied().unwrap_or(0.0)));
            }
            Some(Comp::Pump) => {}
            Some(_) => {
                let to = node_of(dst).unwrap();
                g.pipes.push((format!("pipe_{}", id(line)), from, to));
            }
            None => g.skipped.push(format!("line {line}: unknown destination `{dst}`")),
        }
    }
    let mut eng: Vec<(String, Comp)> = kind.iter().filter(|(_, c)| matches!(c, Comp::Engine(_) | Comp::Apu)).map(|(n, c)| (n.clone(), *c)).collect();
    eng.sort_by(|a, b| a.0.cmp(&b.0));
    for (name, c) in eng {
        let idx = if let Comp::Engine(i) = c { Some(i) } else { None };
        g.feeds.push((format!("{}_feed", id(&name)), format!("{}_inlet", id(&name)), idx));
    }
    Some(g)
}

/// The fuel file for a graph.
pub fn write(g: &Graph, head: &str, engines: usize) -> String {
    let mut f = head.to_owned();
    for (tid, title, kg, arm, unusable) in &g.tanks {
        writeln!(f, "# {title}\n[[fuel.tank]]\nid = \"{tid}\"\ncapacity = \"{:.0} kg\"\nquantity = \"{:.0} kg\"\narm = \"{arm:.2} m\"\nunusable = \"{unusable:.0} kg\"\n", kg.max(1.0), kg * 0.5).unwrap();
    }
    for n in &g.nodes {
        writeln!(f, "[[fuel.line]]\nid = \"{n}\"\n").unwrap();
    }
    for (pid, tank, out, psi) in &g.pumps {
        writeln!(f, "[[fuel.pump]]\nid = \"{pid}\"\ntank = \"{tank}\"\noutlet = \"{out}\"\npressure = \"{psi:.0} psi\"\nflow = \"2.5 kg/s\"\n").unwrap();
    }
    for (vid, a, b, t) in &g.valves {
        writeln!(f, "[[fuel.valve]]\nid = \"{vid}\"\nbetween = [\"{a}\", \"{b}\"]\nopen_time = \"{t:.1} s\"\nflow = \"10 kg/s\"\n").unwrap();
    }
    for (pid, a, b) in &g.pipes {
        writeln!(f, "[[fuel.valve]]\nid = \"{pid}\"\nbetween = [\"{a}\", \"{b}\"]\ncheck = true\nflow = \"20 kg/s\"\n").unwrap();
    }
    for (fid, line, idx) in &g.feeds {
        let demand = match idx {
            Some(i) if *i >= 1 && *i <= engines => format!("eng{i}_fuel_flow.value * 1 kg/s"),
            _ => "0 kg/s".to_owned(),
        };
        writeln!(f, "[[fuel.feed]]\nid = \"{fid}\"\nline = \"{line}\"\ndemand = \"{demand}\"\nminimum = \"0.01 psi\"\n").unwrap();
    }
    f
}
