//! The static half of the widget graph: data model, JSON manifest, primitive
//! registry, checker (rules C0 to C7) and reachability queries.
//!
//! No runtime, no signals, no painter.

mod check;
mod flow;
mod model;
mod query;
mod registry;

pub use check::check;
pub use model::{
    Body, Dir, Endpoint, EndpointError, Kind, Label, LabelError, Manifest, Node, Port, PortKind,
    PortKindTag, PortPath, PortShapeError, Schema, Widget, Wire,
};
pub use query::{labels_of, reach_from, reach_of, sources_of};
pub use registry::{PrimitiveSpec, spec};
