//! EasyAcft core: units, the flow-network solver, fixed-rate scheduling and snapshots.

pub mod network;
pub mod schedule;
pub mod snapshot;
pub mod units;

pub use network::{LinkId, Network, NodeId, STIFF};
pub use schedule::{Rate, Stepped};
pub use snapshot::Snapshot;
pub use units::{parse_quantity, Dim, Quantity};
