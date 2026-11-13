//! Tier 3 of the dev rig (`docs/spec/08-dev-rig.md`): run host-built
//! binaries in a QEMU/KVM guest that has a real DRM device with two virtual
//! monitors, a real logind session and scriptable monitor hot-plug.
//!
//! Nothing is compiled in the guest. The host's `target/` is mounted
//! read-only at `/mnt/target`.
//!
//! ```no_run
//! use vm::{Family, Rig, STAND_IN_SESSION};
//!
//! let rig = Rig::new(Family::Apt)?;
//! rig.up(&vm::target_dir()?)?;
//! rig.session(STAND_IN_SESSION)?;
//! rig.unplug(1)?;
//! rig.plug(1, 1280, 800)?;
//! rig.screenshot(0, "head0.png".as_ref())?;
//! # Ok::<(), vm::Error>(())
//! ```

mod error;
pub mod qemu;
pub mod qmp;
pub mod recipe;
mod rig;

pub use error::{Error, Result};
pub use recipe::{Family, SESSION_UNIT, STAND_IN_SESSION, TARGET_MOUNT};
pub use rig::{DEFAULT_SIZE, HEADS, Rig, build, target_dir};
