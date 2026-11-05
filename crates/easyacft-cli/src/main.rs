use easyacft_parts::Aircraft;
use std::path::PathBuf;
use std::process::ExitCode;

const CODES: &[(&str, &str)] = &[
    ("E0001", "A file could not be read. Check the path in `include`."),
    ("E0002", "A file is not valid TOML."),
    ("E0003", "`include` must be a list of paths, relative to the file that has it."),
    ("E0004", "Parts are written as [[domain.kind]] tables, for example [[electrical.bus]]."),
    ("E0005", "A binding is \"dataref\" = \"part.value\" (EasyAcft to X-Plane) or { to = \"part.value\" } (X-Plane to EasyAcft)."),
    ("E0006", "Every part needs an `id`."),
    ("E0007", "A part with `count` needs `{n}` in its id so each copy has its own name."),
    ("E0008", "A setting must be text, a number, true/false or a list."),
    ("E0009", "Two parts have the same id."),
    ("E0010", "A [variant] file must name the aircraft it changes with `of`."),
    ("E0011", "A variant removes a part the base aircraft does not have."),
    ("E0012", "A variant changes a part into a different kind; remove it and add a new one instead."),
    ("E0013", "A [kinds.<name>] declaration is wrong: settings and values are name = type, where a type is bool, number, text, a unit such as \"L/min\", \"expr <type>\" or \"part <domain.kind>\"."),
    ("E0101", "The part kind does not exist. `easyacft kinds` lists them."),
    ("E0102", "The part has no setting with this name."),
    ("E0103", "A setting has the wrong type or unit."),
    ("E0104", "A rate must be a time, for example \"2 ms\"."),
    ("E0105", "A required setting is missing."),
    ("E0106", "A setting names a part that does not exist or is the wrong kind."),
    ("E0201", "An expression has an error."),
];

fn usage() -> ExitCode {
    eprintln!("usage:\n  easyacft check <aircraft dir>\n  easyacft sim <aircraft dir> [--for 10s] [--every 1s] [--set name=value]... [--print name,name]\n  easyacft names <aircraft dir>\n  easyacft kinds\n  easyacft explain <code>
  easyacft fit <aircraft dir> <engine id> <points.toml> [--tune name,name]
  easyacft import <file.acf | MSFS aircraft folder> <new folder> [--code <plugin source>] [--catalogue <exported json>]
  easyacft export <aircraft dir> <new folder> [--name crate-name]
  easyacft test <aircraft dir> [scenario.toml]...   (default: every file in <dir>/tests)
  easyacft profile <aircraft dir> [--for 60s] [--scenario file.toml] [--budget 1ms]");
    ExitCode::from(2)
}

fn secs(s: &str) -> Option<f64> {
    easyacft_core::parse_quantity(s).ok().filter(|q| q.dim == easyacft_core::Dim::TIME).map(|q| q.si)
}

fn load(dir: &str) -> Option<Aircraft> {
    match Aircraft::load(&PathBuf::from(dir)) {
        Ok((a, r)) => {
            if !r.diagnostics.is_empty() {
                eprint!("{}", r.render());
            }
            Some(a)
        }
        Err(r) => {
            eprint!("{}", r.render());
            eprintln!("{} error{}", r.errors(), if r.errors() == 1 { "" } else { "s" });
            None
        }
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("check") if args.len() == 2 => match load(&args[1]) {
            Some(a) => {
                println!("{}: {} parts, {} values, no errors", if a.name.is_empty() { &args[1] } else { &a.name }, a.checked.parts.len(), a.checked.slots.names.len());
                ExitCode::SUCCESS
            }
            None => ExitCode::FAILURE,
        },
        Some("names") if args.len() == 2 => match load(&args[1]) {
            Some(a) => {
                for (n, t) in &a.checked.slots.names {
                    println!("{n}  ({})", t.name());
                }
                ExitCode::SUCCESS
            }
            None => ExitCode::FAILURE,
        },
        Some("kinds") if args.len() == 2 => {
            let Some(s) = easyacft_parts::catalog::all().iter().find(|s| format!("{}.{}", s.domain, s.kind) == args[1]) else {
                eprintln!("no part kind `{}`; `easyacft kinds` lists them", args[1]);
                return ExitCode::FAILURE;
            };
            println!("[[{}.{}]]  {}
", s.domain, s.kind, s.help);
            for f in s.fields {
                let d = f.default.map(|d| format!("= {d:?}")).unwrap_or_else(|| "(required)".into());
                println!("  {:<22} {:<28} {}", f.key, d, f.help);
            }
            println!("
values: {}", s.outputs.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(", "));
            ExitCode::SUCCESS
        }
        Some("kinds") => {
            for s in easyacft_parts::catalog::all() {
                println!("{}.{}  {}", s.domain, s.kind, s.help);
            }
            ExitCode::SUCCESS
        }
        Some("explain") if args.len() == 2 => match CODES.iter().find(|(c, _)| c.eq_ignore_ascii_case(&args[1])) {
            Some((c, t)) => {
                println!("{c}: {t}");
                ExitCode::SUCCESS
            }
            None => {
                eprintln!("no such code `{}`", args[1]);
                ExitCode::FAILURE
            }
        },
        Some("fit") if args.len() >= 4 => fit(&args[1..]),
        Some("import") if args.len() >= 3 && args.len() % 2 == 1 && args[3..].chunks(2).all(|c| c[0] == "--code" || c[0] == "--catalogue") => {
            let out = PathBuf::from(&args[2]);
            let flag = |n: &str| args[3..].chunks(2).find(|c| c[0] == n).map(|c| PathBuf::from(&c[1]));
            let (code, catalogue) = (flag("--code"), flag("--catalogue"));
            if out.join("aircraft.toml").exists() {
                eprintln!("`{}` already has an aircraft.toml", out.display());
                return ExitCode::FAILURE;
            }
            match easyacft_import::import_with(&PathBuf::from(&args[1]), &out, code.as_deref(), catalogue.as_deref()) {
                Ok(report) => {
                    print!("{report}");
                    match load(&args[2]) {
                        Some(a) => {
                            println!("
{}: {} parts, no errors. Written to {}", a.name, a.checked.parts.len(), out.display());
                            ExitCode::SUCCESS
                        }
                        None => ExitCode::FAILURE,
                    }
                }
                Err(e) => {
                    eprintln!("{e}");
                    ExitCode::FAILURE
                }
            }
        }
        Some("export") if args.len() >= 3 => {
            let name = args.iter().position(|a| a == "--name").and_then(|i| args.get(i + 1)).map(String::as_str);
            match easyacft_export::export(&PathBuf::from(&args[1]), &PathBuf::from(&args[2]), name) {
                Ok(s) => {
                    println!("wrote crate `{}` to {}: {} parts, {} values, {} expressions as Rust, {} scenario tests", s.crate_name, s.out.display(), s.parts, s.values, s.expressions, s.tests);
                    println!("build it with `cargo test` in that folder; it does not need EasyAcft");
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("{e}");
                    ExitCode::FAILURE
                }
            }
        }
        Some("test") if args.len() >= 2 => {
            let mut files: Vec<PathBuf> = args[2..].iter().map(PathBuf::from).collect();
            if files.is_empty() {
                let dir = PathBuf::from(&args[1]).join("tests");
                files = std::fs::read_dir(&dir).map(|r| r.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "toml")).collect()).unwrap_or_default();
                files.sort();
            }
            if files.is_empty() {
                eprintln!("no scenarios: put .toml files in {}/tests or name them", args[1]);
                return ExitCode::FAILURE;
            }
            let mut failed = 0;
            for f in &files {
                let Some(mut a) = load(&args[1]) else { return ExitCode::FAILURE };
                let text = match std::fs::read_to_string(f) {
                    Ok(t) => t,
                    Err(e) => {
                        eprintln!("{}: {e}", f.display());
                        return ExitCode::FAILURE;
                    }
                };
                match easyacft_parts::scenario::run(&mut a, &text, 1.0 / 60.0) {
                    Ok(out) => {
                        for o in out {
                            println!("{} {} at {:.1} s: {}", if o.passed { "ok  " } else { "FAIL" }, f.display(), o.at, o.that);
                            failed += !o.passed as usize;
                        }
                    }
                    Err(e) => {
                        eprintln!("{}: {e}", f.display());
                        failed += 1;
                    }
                }
            }
            if failed > 0 {
                eprintln!("{failed} failed");
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            }
        }
        Some("profile") if args.len() >= 2 => {
            let Some(mut a) = load(&args[1]) else { return ExitCode::FAILURE };
            let (mut total, mut budget, mut scenario) = (60.0, None, None);
            let mut it = args[2..].iter();
            while let Some(flag) = it.next() {
                match (flag.as_str(), it.next()) {
                    ("--for", Some(v)) => total = secs(v).unwrap_or(total),
                    ("--budget", Some(v)) => budget = secs(v),
                    ("--scenario", Some(v)) => scenario = Some(v.clone()),
                    _ => return usage(),
                }
            }
            if let Some(f) = scenario {
                let text = std::fs::read_to_string(&f).unwrap_or_default();
                if let Err(e) = easyacft_parts::scenario::run(&mut a, &text, 1.0 / 60.0) {
                    eprintln!("{f}: {e}");
                    return ExitCode::FAILURE;
                }
            }
            let frame = 1.0 / 60.0;
            a.start_profile();
            let mut frames: Vec<f64> = Vec::new();
            while (frames.len() as f64) * frame < total {
                let t = std::time::Instant::now();
                a.step(frame);
                frames.push(t.elapsed().as_secs_f64());
            }
            let sim_s = frames.len() as f64 * frame;
            let spent: f64 = frames.iter().sum();
            println!("{:<12} {:>9} {:>12} {:>14} {:>7}", "domain", "steps", "us per step", "ms per sim s", "share");
            for d in a.profile().iter().filter(|d| d.steps > 0) {
                println!("{:<12} {:>9} {:>12.2} {:>14.3} {:>6.1}%", d.name, d.steps, d.seconds / d.steps as f64 * 1e6, d.seconds / sim_s * 1e3, d.seconds / spent * 100.0);
            }
            frames.sort_by(f64::total_cmp);
            let mean = spent / frames.len().max(1) as f64;
            let p99 = frames.get(frames.len() * 99 / 100).copied().unwrap_or(0.0);
            let max = frames.last().copied().unwrap_or(0.0);
            println!("{} frames of {:.1} ms: mean {:.3} ms, p99 {:.3} ms, max {:.3} ms ({:.0}x real time)", frames.len(), frame * 1e3, mean * 1e3, p99 * 1e3, max * 1e3, frame / mean.max(1e-12));
            match budget {
                Some(b) if mean > b => {
                    eprintln!("mean frame {:.3} ms is over the {:.3} ms budget", mean * 1e3, b * 1e3);
                    ExitCode::FAILURE
                }
                _ => ExitCode::SUCCESS,
            }
        }
        Some("sim") if args.len() >= 2 => {
            let Some(mut a) = load(&args[1]) else { return ExitCode::FAILURE };
            let (mut total, mut every, mut print) = (10.0, 1.0, Vec::<String>::new());
            let mut it = args[2..].iter();
            while let Some(flag) = it.next() {
                let val = it.next();
                match (flag.as_str(), val) {
                    ("--for", Some(v)) => total = secs(v).unwrap_or(total),
                    ("--every", Some(v)) => every = secs(v).unwrap_or(every),
                    ("--print", Some(v)) => print = v.split(',').map(str::to_owned).collect(),
                    ("--set", Some(v)) => {
                        let Some((k, x)) = v.split_once('=') else { return usage() };
                        let x: f64 = match x {
                            "true" | "on" => 1.0,
                            "false" | "off" => 0.0,
                            other => other.parse().unwrap_or(0.0),
                        };
                        if !a.set(k, x) {
                            eprintln!("nothing called `{k}`");
                            return ExitCode::FAILURE;
                        }
                    }
                    _ => return usage(),
                }
            }
            if print.is_empty() {
                print = a.names().filter(|n| n.ends_with(".voltage") || n.ends_with(".powered")).map(str::to_owned).collect();
            }
            for p in &print {
                if a.slot(p).is_none() {
                    eprintln!("nothing called `{p}`");
                    return ExitCode::FAILURE;
                }
            }
            let dt = 1.0 / 60.0;
            let mut next = 0.0;
            while a.time <= total + 1e-9 {
                if a.time + 1e-9 >= next {
                    let cols: Vec<String> = print.iter().map(|p| format!("{p}={:.3}", a.get(p).unwrap())).collect();
                    println!("t={:.2} {}", a.time, cols.join(" "));
                    next += every;
                }
                a.step(dt);
            }
            ExitCode::SUCCESS
        }
        _ => usage(),
    }
}

fn fit(args: &[String]) -> ExitCode {
    use easyacft_parts::engine::{fit, fit_error, FitPoint, Params};
    let Some(a) = load(&args[0]) else { return ExitCode::FAILURE };
    let Some(part) = a.checked.parts.iter().find(|p| p.id == args[1] && p.kind == "gas_turbine") else {
        eprintln!("no gas turbine called `{}`", args[1]);
        return ExitCode::FAILURE;
    };
    let base = std::path::Path::new(&args[0]).to_path_buf();
    let params = Params::from_part(part, &base);
    let text = match std::fs::read_to_string(&args[2]) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("cannot read `{}`: {e}", args[2]);
            return ExitCode::FAILURE;
        }
    };
    let doc = match text.parse::<toml_edit::ImDocument<String>>() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("`{}` is not valid TOML: {}", args[2], e.message());
            return ExitCode::FAILURE;
        }
    };
    let q = |v: Option<&toml_edit::Item>, dim| v.and_then(|x| x.as_str()).and_then(|s| easyacft_core::parse_quantity(s).ok()).filter(|x| x.dim == dim).map(|x| x.si);
    let points: Vec<FitPoint> = doc
        .get("point")
        .and_then(|p| p.as_array_of_tables())
        .map(|arr| {
            arr.iter()
                .map(|t| FitPoint {
                    thrust_share: t.get("thrust").and_then(|x| x.as_float().or(x.as_integer().map(|i| i as f64))).unwrap_or(1.0),
                    fuel_flow: q(t.get("fuel_flow"), easyacft_core::Dim::MASS_FLOW),
                    egt: q(t.get("egt"), easyacft_core::Dim::TEMPERATURE),
                })
                .collect()
        })
        .unwrap_or_default();
    if points.is_empty() {
        eprintln!("`{}` has no [[point]] tables", args[2]);
        return ExitCode::FAILURE;
    }
    let tune: Vec<String> = args.iter().position(|a| a == "--tune").and_then(|i| args.get(i + 1)).map(|t| t.split(',').map(str::to_owned).collect()).unwrap_or_default();
    let show = |p: &Params, title: &str| {
        let (err, rows) = fit_error(p, &points);
        println!("{title} (error {err:.5})");
        println!("  thrust   fuel flow  target      error   EGT");
        for (pt, (wf, egt, _)) in points.iter().zip(rows) {
            let target = pt.fuel_flow.map_or("-".to_owned(), |t| format!("{t:.3}"));
            let e = pt.fuel_flow.map_or("-".to_owned(), |t| format!("{:+.1} %", (wf - t) / t * 100.0));
            println!("  {:>5.0} %  {:>7.3}    {:>8}  {:>8}  {:.0} C", pt.thrust_share * 100.0, wf, target, e, egt - 273.15);
        }
    };
    show(&params, "as written");
    if !tune.is_empty() {
        let names: Vec<&str> = tune.iter().map(String::as_str).collect();
        let tuned = fit(params.clone(), &points, &names, 10);
        show(&tuned, "tuned");
        let pairs = [
            ("fan_efficiency", tuned.eta_c[0]),
            ("lpc_efficiency", tuned.eta_c[1]),
            ("ipc_efficiency", tuned.eta_c[2]),
            ("hpc_efficiency", tuned.eta_c[3]),
            ("hpt_efficiency", tuned.eta_t[0]),
            ("ipt_efficiency", tuned.eta_t[1]),
            ("lpt_efficiency", tuned.eta_t[2]),
            ("burner_efficiency", tuned.eta_b),
            ("cooling", tuned.cooling),
        ];
        for (k, v) in pairs.iter().filter(|(k, _)| names.contains(k)) {
            println!("{k} = {v:.4}");
        }
        if names.contains(&"airflow") {
            println!("airflow = \"{:.1} kg/s\"", tuned.airflow);
        }
    }
    ExitCode::SUCCESS
}
