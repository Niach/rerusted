//! `teamlog::TeamLog` on top of p2panda's auth, encryption and spaces crates.
//!
//! Spike for #RUST-30. Everything p2panda stays inside this crate: the
//! public API is [`Replica`], [`Error`] and the trait from `teamlog`.
//!
//! How the trait maps onto p2panda 0.8:
//!
//! | ours | theirs |
//! |---|---|
//! | team | a space, and the group it creates |
//! | `Role::Admin` / `Role::Member` | `Access::manage()` / `Access::write()` |
//! | `Entry.bytes` | a `p2panda_core` header and, as its body, the spaces message |
//! | `announce` | publishing a long-term key bundle |
//! | team key and rotation | the "data scheme" group secrets of `p2panda-encryption` |
//! | concurrent admin operations | their `StrongRemove` resolver |

mod forge;
mod replica;
mod tables;

pub use replica::{Error, Replica};
