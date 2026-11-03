//! EasyAcft core: units, flow networks, scheduling, snapshots.

pub mod network;
pub mod schedule;
pub mod snapshot;
pub mod units;

pub use network::{LinkId, Network, NodeId, STIFF};
pub use schedule::{Rate, Stepped};
pub use snapshot::Snapshot;
pub use units::{parse_quantity, Dim, Quantity};
