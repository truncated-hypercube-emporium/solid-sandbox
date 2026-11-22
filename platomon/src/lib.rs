//! platomon — Platonic-solid ASCII system monitor.
//!
//! The binary lives in `main.rs`; everything else is a library so the
//! renderer can be benchmarked and unit-tested.

pub mod ascii;
pub mod aspect;
pub mod clock;
pub mod color;
#[cfg(feature = "monitor")]
pub mod collector;
#[cfg(feature = "monitor")]
pub mod config;
#[cfg(feature = "monitor")]
pub mod metrics;
pub mod overlay;
pub mod render;
pub mod scene;
#[cfg(feature = "monitor")]
pub mod ui;
