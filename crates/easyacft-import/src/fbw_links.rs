//! Who reads whom among FlyByWire's A380 computers: the bus inputs `FlyByWireInterface.cpp` wires into each PRIM,
//! SEC, FCDC, FCU and FADEC, and the computers each display's code reads, as data links.

use std::collections::{BTreeSet, HashSet};
use std::fmt::Write;
use std::path::Path;

/// A source array of `FlyByWireInterface.cpp` by its index.
fn source(array: &str, i: usize) -> Option<String> {
    Some(match array {
        "adr" | "ir" => format!("adirs_{}", i + 1),
        "ra" => format!("ra_sys_{}", ["a", "b", "c"].get(i)?),
        "lgciu" => format!("lgciu_{}", i + 1),
        "fcu" => format!("fcu_{}", i + 1),
        "prims" => format!("prim_{}", i + 1),
        "secs" => format!("sec_{}", i + 1),
        _ => return None,
    })
}

/// The receivers an update function feeds, by its index.
fn receivers(func: &str, k: usize) -> Vec<String> {
    match func {
        "updatePrim" => vec![format!("prim_{}", k + 1)],
        "updateSec" => vec![format!("sec_{}", k + 1)],
        "updateFcdc" => vec![format!("fcdc_{}", k + 1)],
        "updateFcu" => vec![format!("fcu_{}", k + 1)],
        "updateFadec" => vec![format!("fadec_{}a", k + 1), format!("fadec_{}b", k + 1)],
        _ => Vec::new(),
    }
}

const COUNTS: [(&str, usize); 5] = [("updatePrim", 3), ("updateSec", 3), ("updateFcdc", 2), ("updateFcu", 2), ("updateFadec", 4)];

/// `xBusOutputs[n]` or `xBusOutputs[i]` on the right of an assignment.
fn rhs(line: &str) -> Option<(String, String)> {
    let r = line.split_once(" = ")?.1.trim().trim_end_matches(';');
    let (name, idx) = r.split_once("BusOutputs[")?;
    Some((name.to_owned(), idx.trim_end_matches(']').to_owned()))
}

/// Runs one update function for receiver `k`, following `if (idx == n)` chains and `for` loops.
fn run(body: &[&str], idx: &str, k: usize, out: &mut BTreeSet<(String, String)>, to: &[String]) {
    // Each open block: whether it runs, whether its chain already ran, and a loop's count.
    let mut stack: Vec<(bool, bool, Option<usize>)> = Vec::new();
    let mut alias: Vec<(String, String)> = Vec::new();
    let test = |c: &str| -> Option<bool> {
        let c = c.replace(' ', "");
        let n = c.strip_prefix(&format!("({idx}=="))?.split(')').next()?.parse::<usize>().ok()?;
        Some(n == k)
    };
    for raw in body {
        let l = raw.trim().replace("{}", "");
        let active = stack.iter().all(|s| s.0);
        if let Some(rest) = l.strip_prefix("} else if ") {
            let top = stack.last_mut().unwrap();
            let v = !top.1 && test(rest).unwrap_or(true);
            top.0 = v;
            top.1 |= v;
            continue;
        }
        if l.starts_with("} else") {
            let top = stack.last_mut().unwrap();
            top.0 = !top.1;
            top.1 = true;
            continue;
        }
        if let Some(rest) = l.strip_prefix("if ") {
            if l.ends_with('{') {
                let v = test(rest).unwrap_or(true);
                stack.push((v, v, None));
            }
            continue;
        }
        if let Some(rest) = l.strip_prefix("for (int i = 0; i < ") {
            let n = rest.split(';').next().and_then(|n| n.trim().parse().ok());
            stack.push((true, true, n));
            continue;
        }
        if l == "}" {
            stack.pop();
            continue;
        }
        if l.ends_with('{') {
            stack.push((true, true, None));
            continue;
        }
        if !active {
            continue;
        }
        let looped = stack.iter().rev().find_map(|s| s.2);
        if let Some((name, i)) = rhs(&l) {
            let ids: Vec<usize> = match i.parse::<usize>() {
                Ok(n) => vec![n],
                Err(_) if i == "i" => (0..looped.unwrap_or(0)).collect(),
                Err(_) => Vec::new(),
            };
            let lhs = l.split(" = ").next().unwrap_or("");
            if lhs.contains("bus_inputs.") || lhs.contains("busInputs.") || lhs.contains(".in.") {
                for n in ids {
                    if let Some(s) = source(&name, n) {
                        for r in to {
                            out.insert((s.clone(), r.clone()));
                        }
                    }
                }
            } else if let (false, [n]) = (lhs.contains('.'), ids.as_slice()) {
                if let Some(s) = source(&name, *n) {
                    alias.push((lhs.trim().to_owned(), s));
                }
            }
        } else if let Some((lhs, r)) = l.split_once(" = ") {
            let r = r.trim().trim_end_matches(';');
            if lhs.contains("bus_inputs.") {
                if let Some((_, s)) = alias.iter().rev().find(|(a, _)| a == r) {
                    for t in to {
                        out.insert((s.clone(), t.clone()));
                    }
                }
            }
        }
    }
}

/// Display code folders and the display units that run them.
const DISPLAYS: [(&str, &[&str]); 4] = [("PFD", &["capt_pfd_du", "fo_pfd_du"]), ("ND", &["capt_nd_du", "fo_nd_du"]), ("EWD", &["capt_ewd_du", "fo_ewd_du"]), ("HUD", &["hud"])];

/// An `L:A32NX_` variable prefix a display reads, and the computers that publish it.
const PUBLISHERS: [(&str, &[&str]); 12] = [
    ("ADIRS_ADR_", &["adirs_1", "adirs_2", "adirs_3"]),
    ("ADIRS_IR_", &["adirs_1", "adirs_2", "adirs_3"]),
    ("TCAS_", &["tcas"]),
    ("RA_", &["ra_sys_a", "ra_sys_b", "ra_sys_c"]),
    ("LGCIU_", &["lgciu_1", "lgciu_2"]),
    ("FCU_", &["fcu_1", "fcu_2"]),
    ("FMGC_", &["fms_1", "fms_2", "fms_3"]),
    ("FM_", &["fms_1", "fms_2", "fms_3"]),
    ("WXR_", &["wxr"]),
    ("KCCU_", &["kccu_capt", "kccu_fo"]),
    ("EGPWS_", &["egpwc"]),
    ("FCDC_", &["fcdc_1", "fcdc_2"]),
];

fn display_reads(tree: &Path, out: &mut BTreeSet<(String, String)>) {
    let root = tree.join("fbw-a380x/src/systems/instruments/src");
    for (dir, units) in DISPLAYS {
        let mut text = String::new();
        let mut todo = vec![root.join(dir)];
        while let Some(d) = todo.pop() {
            let Ok(rd) = std::fs::read_dir(&d) else { continue };
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    todo.push(p);
                } else if p.extension().is_some_and(|x| x == "ts" || x == "tsx") {
                    text += &std::fs::read_to_string(&p).unwrap_or_default();
                }
            }
        }
        for (prefix, from) in PUBLISHERS {
            if text.contains(&format!("L:A32NX_{prefix}")) {
                for s in from {
                    for u in units {
                        out.insert((s.to_string(), u.to_string()));
                    }
                }
            }
        }
    }
}

/// The links as `data.link` parts between computers that exist in `has`.
pub fn links(tree: &Path, has: &HashSet<String>) -> String {
    let mut pairs = BTreeSet::new();
    let cpp = std::fs::read_to_string(tree.join("fbw-a380x/src/wasm/fbw_a380/src/FlyByWireInterface.cpp")).unwrap_or_default().replace("\r\n", "\n");
    let lines: Vec<&str> = cpp.lines().collect();
    for (func, count) in COUNTS {
        let head = format!("bool FlyByWireInterface::{func}(");
        let Some(start) = lines.iter().position(|l| l.starts_with(&head)) else { continue };
        let end = lines[start + 1..].iter().position(|l| l.starts_with("bool FlyByWireInterface::") || l.starts_with("void FlyByWireInterface::")).map_or(lines.len(), |e| start + 1 + e);
        let idx = lines[start].split("int ").nth(1).and_then(|s| s.split(')').next()).unwrap_or("").trim().to_owned();
        for k in 0..count {
            run(&lines[start + 1..end], &idx, k, &mut pairs, &receivers(func, k));
        }
    }
    display_reads(tree, &mut pairs);
    // Each FCU reads its own side's EFIS control panel.
    if cpp.contains("getFcuEfisPanelInputs(fcuIndex)") {
        pairs.insert(("capt_efis_ctl_panel".into(), "fcu_1".into()));
        pairs.insert(("fo_efis_ctl_panel".into(), "fcu_2".into()));
    }
    // The EGPWC reads ADIRS 1's position and LGCIU 1's gear.
    let rust = std::fs::read_to_string(tree.join("fbw-a380x/src/wasm/systems/a380_systems/src/lib.rs")).unwrap_or_default();
    if rust.contains("self.egpwc.update(&self.adirs, self.lgcius.lgciu1())") {
        pairs.insert(("adirs_1".into(), "egpwc".into()));
        pairs.insert(("lgciu_1".into(), "egpwc".into()));
    }
    // Radio altimeter n's antenna feeds are RA SYS A, B or C's.
    for n in 1..=3 {
        for part in ["coupling", "interrupt"] {
            pairs.insert((format!("ra_ant_{part}_{n}"), format!("ra_sys_{}", ["a", "b", "c"][n - 1])));
        }
    }
    let mut f = String::new();
    let mut n = 0;
    for (a, b) in pairs.iter().filter(|(a, b)| a != b && has.contains(a) && has.contains(b)) {
        writeln!(f, "[[data.link]]\nid = \"{a}_to_{b}\"\nfrom = \"{a}\"\nto = [\"{b}\"]\nvalue = \"1\"\n").unwrap();
        n += 1;
    }
    if n > 0 {
        f.insert_str(0, &format!("# {n} links between computers: the bus inputs FlyByWire's FlyByWireInterface.cpp wires into each PRIM, SEC, FCDC, FCU and FADEC, and the computers each display's code reads.\n\n"));
    }
    f
}
