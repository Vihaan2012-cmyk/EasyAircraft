//! Live link for EasyAcft Studio: values out, settings in, over a local TCP port.

use easyacft_parts::Aircraft;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};

pub const PORT: u16 = 47800;

pub struct Server {
    listener: Option<TcpListener>,
    clients: Vec<(BufReader<TcpStream>, bool)>,
    since: f64,
}

impl Server {
    /// Listens on 127.0.0.1, `EASYACFT_PORT` or 47800; quietly off if the port is taken.
    pub fn start() -> Self {
        let port = std::env::var("EASYACFT_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(PORT);
        let listener = TcpListener::bind(("127.0.0.1", port)).ok().filter(|l| l.set_nonblocking(true).is_ok());
        Server { listener, clients: Vec::new(), since: 0.0 }
    }

    pub fn serve(&mut self, a: &mut Aircraft, dt: f64) {
        let Some(l) = &self.listener else { return };
        while let Ok((stream, _)) = l.accept() {
            if stream.set_nonblocking(true).is_ok() {
                self.clients.push((BufReader::new(stream), false));
            }
        }
        self.since += dt;
        let send_values = self.since >= 0.1;
        if send_values {
            self.since = 0.0;
        }
        let values = send_values.then(|| format!("V {}\n", a.slots.iter().map(|v| format!("{v}")).collect::<Vec<_>>().join(" ")));
        self.clients.retain_mut(|(r, greeted)| {
            if !*greeted {
                let names: Vec<&str> = a.names().collect();
                let msg = format!("N {}\n{}\n", names.len(), names.join("\n"));
                if r.get_mut().write_all(msg.as_bytes()).is_err() {
                    return false;
                }
                *greeted = true;
            }
            let mut line = String::new();
            loop {
                line.clear();
                match r.read_line(&mut line) {
                    Ok(0) => return false,
                    Ok(_) => {
                        let mut it = line.split_whitespace();
                        if let (Some("S"), Some(name), Some(v)) = (it.next(), it.next(), it.next().and_then(|v| v.parse::<f64>().ok())) {
                            a.set(name, v);
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(_) => return false,
                }
            }
            match &values {
                Some(v) => r.get_mut().write_all(v.as_bytes()).is_ok(),
                None => true,
            }
        });
    }
}
