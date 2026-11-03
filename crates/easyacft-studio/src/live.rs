//! Studio's side of the live link to the X-Plane plugin.

use easyacft_parts::Aircraft;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;

pub struct Client {
    reader: BufReader<TcpStream>,
    pub names: Vec<String>,
    pub values: Vec<f64>,
    pub connected: bool,
    want: usize,
}

impl Client {
    pub fn connect() -> Option<Self> {
        let port = std::env::var("EASYACFT_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(47800u16);
        let s = TcpStream::connect(("127.0.0.1", port)).ok()?;
        s.set_nonblocking(true).ok()?;
        Some(Client { reader: BufReader::new(s), names: Vec::new(), values: Vec::new(), connected: true, want: 0 })
    }

    pub fn matches(&self, a: &Aircraft) -> bool {
        self.connected && self.names.len() == a.checked.slots.names.len() && self.names.iter().zip(&a.checked.slots.names).all(|(x, (y, _))| x == y)
    }

    pub fn poll(&mut self) {
        let mut line = String::new();
        loop {
            line.clear();
            match self.reader.read_line(&mut line) {
                Ok(0) => {
                    self.connected = false;
                    break;
                }
                Ok(_) => {
                    let l = line.trim_end();
                    if self.want > 0 {
                        self.names.push(l.to_owned());
                        self.want -= 1;
                    } else if let Some(n) = l.strip_prefix("N ") {
                        self.want = n.parse().unwrap_or(0);
                        self.names.clear();
                    } else if let Some(v) = l.strip_prefix("V ") {
                        self.values = v.split(' ').filter_map(|x| x.parse().ok()).collect();
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(_) => {
                    self.connected = false;
                    break;
                }
            }
        }
    }

    pub fn send(&mut self, name: &str, v: f64) {
        let _ = self.reader.get_mut().write_all(format!("S {name} {v}\n").as_bytes());
    }
}
