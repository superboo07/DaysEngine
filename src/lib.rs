//! The DaysEngine runtime.
//!
//! FILMEngine plays a script as a **timeline**, not a program: every statement
//! owns a `[start, end)` window in frames and the engine walks one clock across
//! the whole file. So the runtime is a scheduler, not an interpreter — see
//! [`stage::Stage`].

// Every other module is held to the same rule the workspace always had. The
// exception is `media`, whose every `unsafe` is a call into the system libav —
// Rust requires the keyword on all FFI, so it cannot be avoided there, and the
// allow is deliberately narrow and greppable.
#![deny(unsafe_code)]

pub mod compose;
pub mod config;
pub mod ending;
pub mod ini;
pub mod lipsync;
#[allow(unsafe_code)]
pub mod media;
pub mod menu;
pub mod mixer;
pub mod options;
pub mod replay;
pub mod save;
pub mod screen;
pub mod stage;
pub mod text;
pub mod vfs;

pub use config::Config;
pub use days_save::FlagStore;
pub use ending::{title_backdrop, Backdrop, EndingList};
pub use ini::Ini;
pub use menu::{Action, Menu, Mode, SaveState, SystemSe};
pub use mixer::Mixer;
pub use options::Tab;
pub use replay::{Scene, Scenes};
pub use screen::{Resolution, Screen, WidgetState};
pub use stage::{Stage, Visual};
pub use vfs::Vfs;
