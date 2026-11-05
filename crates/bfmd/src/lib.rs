//! Daemon internals, exposed as a library so the D-Bus surface can be tested
//! in-process against a private bus rather than by spawning a binary.
//!
//! The public contract lives in `bfm-proto`; nothing here is part of it.

pub mod iface;
pub mod store;
pub mod timer;
