//! FlyByWire's A380 avionics network (`avionics_data_communication_network.rs`): two AFDX networks of switches, the
//! IOMs and CPIOMs on them, and the functions each CPIOM hosts, as computers joined by data links.

use super::plugin::Unit;
use std::fmt::Write;
use std::path::Path;

/// FlyByWire's switch numbers 1..7, 9 and 11..17, 19 by their index in its adjacency tables.
const SWITCH_IDS: [u32; 16] = [1, 2, 3, 4, 5, 6, 7, 9, 11, 12, 13, 14, 15, 16, 17, 19];

fn bus(t: &str) -> &'static str {
    let t = t.trim();
    if t.starts_with("DirectCurrentEssential") || t.contains("108PH") {
        "dc_ess"
    } else if t.starts_with("DirectCurrent(1)") {
        "dc1"
    } else {
        "dc2"
    }
}

fn switch_id(n: u32) -> String {
    if n >= 11 { format!("afdx_b_sw{}", n - 10) } else { format!("afdx_a_sw{n}") }
}

/// The text between `start` and the first `end` after it.
fn block<'a>(text: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let i = text.find(start)? + start.len();
    let j = text[i..].find(end)? + i;
    Some(&text[i..j])
}

/// `(name, switch, bus)` rows such as `("A1", 7, ElectricalBusType::DirectCurrent(1)),`.
fn modules(b: &str) -> Vec<(String, u32, &'static str)> {
    let mut out = Vec::new();
    for line in b.lines() {
        let l = line.trim();
        let Some(rest) = l.strip_prefix("(\"") else { continue };
        let Some((name, rest)) = rest.split_once("\",") else { continue };
        let Some((sw, rest)) = rest.trim().split_once(',') else { continue };
        let Ok(sw) = sw.trim().parse() else { continue };
        let Some(t) = rest.split("ElectricalBusType::").nth(1) else { continue };
        out.push((name.to_owned(), sw, bus(t)));
    }
    out
}

/// The network as computers and links, or nothing without FlyByWire's source.
pub fn network(tree: &Path, units: &[Unit], head: &str) -> Option<String> {
    let text = std::fs::read_to_string(tree.join("fbw-a380x/src/wasm/systems/a380_systems/src/avionics_data_communication_network.rs")).ok()?.replace("\r\n", "\n");
    let mut edges: Vec<(u32, u32)> = Vec::new();
    for b in [block(&text, "let first_network = FxHashMap::from_iter([", "]);")?, block(&text, "let second_network = FxHashMap::from_iter([", "]);")?] {
        for line in b.lines() {
            let l = line.trim().trim_end_matches(',');
            let Some((a, rest)) = l.strip_prefix('(').and_then(|r| r.split_once(", vec![")) else { continue };
            let Ok(a) = a.trim().parse::<usize>() else { continue };
            for n in rest.trim_end_matches("])").split(',').filter_map(|n| n.trim().parse::<usize>().ok()) {
                if a < n && a < 16 && n < 16 {
                    edges.push((SWITCH_IDS[a], SWITCH_IDS[n]));
                }
            }
        }
    }
    let sw = block(&text, "let afdx_switches = [", "]\n        .map")?;
    let mut switches: Vec<(u32, Vec<&'static str>)> = Vec::new();
    for part in sw.split("\n            (").skip(1) {
        let id: String = part.trim_start().chars().take_while(|c| c.is_ascii_digit()).collect();
        let Ok(id) = id.parse() else { continue };
        let buses: Vec<&'static str> = part.split("ElectricalBusType::").skip(1).map(bus).collect();
        switches.push((id, buses));
    }
    let ioms = modules(block(&text, "let io_modules = FxHashMap::from_iter(", ".map(")?);
    let cpioms = modules(block(&text, "let cpio_modules = FxHashMap::from_iter(", ".map(")?);
    if switches.is_empty() || (ioms.is_empty() && cpioms.is_empty()) {
        return None;
    }
    let mut f = head.to_owned();
    writeln!(f, "# The avionics network from FlyByWire's AFDX model: {} switches on networks A and B, {} IOMs and {} CPIOMs, each on a switch of each network, and the functions each CPIOM hosts.\n", switches.len(), ioms.len(), cpioms.len()).unwrap();
    for (id, buses) in &switches {
        let power = buses.iter().map(|b| format!("\"{b}\"")).collect::<Vec<_>>().join(", ");
        writeln!(f, "[[computer.unit]]\nid = \"{}\"\npower = [{power}]\nboot_time = \"2 s\"\n", switch_id(*id)).unwrap();
    }
    let link = |f: &mut String, id: &str, from: &str, to: &str| {
        writeln!(f, "[[data.link]]\nid = \"{id}\"\nfrom = \"{from}\"\nto = [\"{to}\"]\nvalue = \"1\"\n").unwrap();
    };
    for (a, b) in &edges {
        link(&mut f, &format!("{}_to_{}", switch_id(*a), switch_id(*b)), &switch_id(*a), &switch_id(*b));
    }
    for (kind, list) in [("iom", &ioms), ("cpiom", &cpioms)] {
        for (name, s, b) in list {
            let id = format!("{kind}_{}", name.to_ascii_lowercase());
            writeln!(f, "[[computer.unit]]\nid = \"{id}\"\npower = [\"{b}\"]\nboot_time = \"10 s\"\n").unwrap();
            for net in [*s, *s + 10] {
                link(&mut f, &format!("{id}_on_{}", switch_id(net)), &id, &switch_id(net));
            }
        }
    }
    // Hosted functions: a CPIOM's own breaker-fed partitions, such as CPIOM B1's AGS, CPCS, TCS and VCS.
    for (name, _, _) in &cpioms {
        let host = format!("cpiom_{}", name.to_ascii_lowercase());
        for (lid, _, _, _, _) in units.iter().filter(|u| u.0.starts_with(&format!("{host}_"))) {
            link(&mut f, &format!("{host}_hosts_{}", &lid[host.len() + 1..]), &host, lid);
        }
    }
    // FlyByWire's LGCIUs each read the proximity sensors of their own number.
    for (lid, _, _, _, _) in units.iter().filter(|u| u.0.starts_with("prox_")) {
        let lgciu = format!("lgciu_{}", lid.rsplit('_').next().unwrap_or(""));
        if units.iter().any(|u| u.0 == lgciu) {
            link(&mut f, &format!("{lid}_to_{lgciu}"), lid, &lgciu);
        }
    }
    Some(f)
}
