//! Scenario tests: timed inputs and expectations written as expressions.
//!
//! ```toml
//! [[step]]
//! at = "0 s"
//! set = { "bat_sw.on" = 1 }
//!
//! [[expect]]
//! at = "40 s"
//! that = "dc1.voltage > 27 V"
//!
//! [trace]
//! print = ["dc1.voltage"]
//! every = "5 s"
//! ```

use crate::{Aircraft, Expr};
use toml_edit::{DocumentMut, Item};

pub struct Outcome {
    pub at: f64,
    pub that: String,
    pub passed: bool,
}

fn time(item: Option<&Item>) -> Result<f64, String> {
    let s = item.and_then(Item::as_str).ok_or("each step and expect needs `at`, a time such as \"5 s\"")?;
    easyacft_core::parse_quantity(s).ok().filter(|q| q.dim == easyacft_core::Dim::TIME).map(|q| q.si).ok_or_else(|| format!("`{s}` is not a time"))
}

/// Runs a scenario on a fresh aircraft; every expectation is checked at its time.
pub fn run(a: &mut Aircraft, text: &str, dt: f64) -> Result<Vec<Outcome>, String> {
    let doc: DocumentMut = text.parse().map_err(|e: toml_edit::TomlError| e.to_string())?;
    let mut steps = Vec::new();
    if let Some(arr) = doc.get("step").and_then(Item::as_array_of_tables) {
        for t in arr.iter() {
            let at = time(t.get("at"))?;
            let set = t.get("set").and_then(Item::as_table_like).ok_or("a step needs `set = { \"name\" = value }`")?;
            for (k, v) in set.iter() {
                let x = v.as_float().or_else(|| v.as_integer().map(|i| i as f64)).or_else(|| v.as_bool().map(|b| b as u8 as f64)).ok_or_else(|| format!("`{k}` must be set to a number or true/false"))?;
                if a.slot(k).is_none() {
                    return Err(format!("nothing called `{k}`"));
                }
                steps.push((at, k.to_owned(), x));
            }
        }
    }
    let mut checks = Vec::new();
    if let Some(arr) = doc.get("expect").and_then(Item::as_array_of_tables) {
        for t in arr.iter() {
            let at = time(t.get("at"))?;
            let that = t.get("that").and_then(Item::as_str).ok_or("an expect needs `that`, a true/false expression")?;
            let p = easyacft_lang::expr::compile(that, &a.checked.slots).map_err(|e| format!("`{that}`: {}", e.message))?;
            if p.ty != easyacft_lang::Type::Bool {
                return Err(format!("`{that}` must be true or false"));
            }
            checks.push((at, that.to_owned(), Expr::new(&p)));
        }
    }
    let trace: Vec<String> = doc.get("trace").and_then(|t| t.get("print")).and_then(Item::as_array).map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect()).unwrap_or_default();
    let every = doc.get("trace").and_then(|t| t.get("every")).map(|e| time(Some(e))).transpose()?.unwrap_or(5.0);
    let mut next_trace = 0.0;
    steps.sort_by(|x, y| x.0.total_cmp(&y.0));
    checks.sort_by(|x, y| x.0.total_cmp(&y.0));
    let end = steps.iter().map(|s| s.0).chain(checks.iter().map(|c| c.0)).fold(0.0, f64::max);
    let start = a.time;
    let (mut si, mut out) = (0, Vec::new());
    let mut pending: std::collections::VecDeque<_> = checks.into();
    loop {
        let t = a.time - start;
        if !trace.is_empty() && t + 1e-9 >= next_trace {
            let cols: Vec<String> = trace.iter().map(|n| format!("{n}={:.3}", a.get(n).unwrap_or(f64::NAN))).collect();
            println!("t={t:.1} {}", cols.join(" "));
            next_trace += every;
        }
        while si < steps.len() && steps[si].0 <= t + 1e-9 {
            a.set(&steps[si].1, steps[si].2);
            si += 1;
        }
        while pending.front().is_some_and(|c| c.0 <= t + 1e-9) {
            let (at, that, mut e) = pending.pop_front().unwrap();
            let passed = e.yes(&a.slots, 0.0);
            out.push(Outcome { at, that, passed });
        }
        if t > end + 1e-9 || (si >= steps.len() && pending.is_empty()) {
            break;
        }
        a.step(dt);
    }
    Ok(out)
}
