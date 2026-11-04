//! Custom parts inside the networks: a port is a Norton source on one node, flow = g * (e - v);
//! a link is a conductance between two nodes. Code sets `<name>_e` and `<name>_g`, the network writes `<name>_v` and `<name>_q`.

use crate::custom;
use easyacft_core::network::{LinkId, Network, NodeId};
use easyacft_lang::Checked;

struct Port {
    node: NodeId,
    e: usize,
    g: usize,
    v: usize,
    q: usize,
}

struct Link {
    link: LinkId,
    g: usize,
    q: usize,
}

#[derive(Default)]
pub struct Ports {
    ports: Vec<Port>,
    links: Vec<Link>,
}

impl Ports {
    /// Ports and links of `domain`; `node_of` maps a part index to its network node.
    pub fn new(c: &Checked, domain: &str, net: &mut Network, node_of: impl Fn(usize) -> Option<NodeId>) -> Self {
        let mut out = Ports::default();
        let slot = |id: &str, n: &str, k: &str| c.slots.index.get(&format!("{id}.{n}_{k}")).copied();
        for p in c.parts.iter().filter(|p| p.domain == "custom") {
            for port in custom::ports_of(p.kind).into_iter().filter(|x| x.domain == domain) {
                let n = port.name;
                if port.link {
                    let ends = p.parts(n);
                    let (Some(a), Some(b)) = (ends.first().and_then(|&x| node_of(x)), ends.get(1).and_then(|&x| node_of(x))) else { continue };
                    let (Some(g), Some(q)) = (slot(&p.id, n, "g"), slot(&p.id, n, "q")) else { continue };
                    let link = net.add_link(a, b, 1e12, 0.0);
                    net.set_closed(link, false);
                    out.links.push(Link { link, g, q });
                } else {
                    let Some(node) = p.part(n).and_then(&node_of) else { continue };
                    let (Some(e), Some(g), Some(v), Some(q)) = (slot(&p.id, n, "e"), slot(&p.id, n, "g"), slot(&p.id, n, "v"), slot(&p.id, n, "q")) else { continue };
                    out.ports.push(Port { node, e, g, v, q });
                }
            }
        }
        out
    }

    /// Call between `begin_step` and the solve.
    pub fn stamp(&self, net: &mut Network, slots: &[f64]) {
        for p in &self.ports {
            let g = slots[p.g].max(0.0);
            if g > 0.0 {
                net.stamp(p.node, g, g * slots[p.e]);
            }
        }
        for l in &self.links {
            let g = slots[l.g];
            net.set_closed(l.link, g > 0.0);
            if g > 0.0 {
                net.set_resistance(l.link, 1.0 / g);
            }
        }
    }

    /// Call after the solve.
    pub fn read(&self, net: &Network, slots: &mut [f64]) {
        for p in &self.ports {
            let v = net.potential(p.node);
            slots[p.v] = v;
            slots[p.q] = slots[p.g].max(0.0) * (slots[p.e] - v);
        }
        for l in &self.links {
            slots[l.q] = if slots[l.g] > 0.0 { net.flow(l.link) } else { 0.0 };
        }
    }
}
