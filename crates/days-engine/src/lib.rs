//! The DaysEngine runtime.
//!
//! FILMEngine plays a script as a **timeline**, not a program: every statement
//! owns a `[start, end)` window in frames and the engine walks one clock across
//! the whole file. So the runtime is a scheduler, not an interpreter — see
//! [`stage::Stage`].

#![forbid(unsafe_code)]

pub mod compose;
pub mod ini;
pub mod mixer;
pub mod stage;
pub mod text;

pub use ini::Ini;
pub use mixer::Mixer;
pub use stage::{Stage, Visual};
