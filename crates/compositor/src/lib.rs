//! Spike (#RUST-23): a minimal Wayland compositor on Smithay.
//!
//! The model has one idea: a [`DeskId`] names a seat together with the outputs
//! it works on. The person's desk and an agent space are the same thing; a
//! space is a desk with one virtual output that is never shown, its own seat
//! and its own XWayland.
//!
//! Two backends: `Headless` renders in memory with no GPU, `Nested` shows the
//! outputs in a window on the host desktop.
//!
//! Linux only. On other systems the crate is empty.
#![cfg(target_os = "linux")]

mod api;
mod focus;
mod handlers;
mod input;
mod nested;
mod render;
mod state;

pub use api::{
    Backend, Config, DeskHandle, DeskId, DeskKind, Error, Frame, Handle, OutputConfig, OutputInfo,
    OutputKind, Rect, SeatInfo, SeatSnapshot, SpaceConfig, Stats, WindowInfo, start,
};
pub use input::keys;
pub use state::run;
