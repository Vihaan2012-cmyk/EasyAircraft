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
    eprintln!("usage:\n  easyacft check <aircraft dir>\n  easyacft sim <aircraft dir> [--for 10s] [--every 1s] [--set name=value]... [--print name,name]\n  easyacft names <aircraft dir>\n  easyacft kinds\n  easyacft explain <code>");
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
