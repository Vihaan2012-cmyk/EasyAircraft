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
  easyacft import <file.acf | MSFS aircraft folder> <new folder>");
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
        Some("import") if args.len() == 3 => {
            let out = PathBuf::from(&args[2]);
            if out.join("aircraft.toml").exists() {
                eprintln!("`{}` already has an aircraft.toml", out.display());
                return ExitCode::FAILURE;
            }
            match easyacft_import::import(&PathBuf::from(&args[1]), &out) {
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
