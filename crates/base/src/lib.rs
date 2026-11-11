//! Shared types for every crate: typed ids, names, [`Fault`] and [`Secret`].
//!
//! Types only. No I/O.

mod fault;
mod id;
mod name;
mod secret;

pub use fault::{Class, Code, CodeError, Fault, Path, Pointer, PointerError, Remedy, Span};
pub use id::{DeviceId, FaultId, SessionId, TeamId, UserId, WidgetId};
pub use name::{ChildName, NameError, PortName};
pub use secret::Secret;
