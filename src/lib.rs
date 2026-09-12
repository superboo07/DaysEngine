//! The DaysEngine runtime.
//!
//! FILMEngine plays a script as a **timeline**, not a program: every statement
//! owns a `[start, end)` window in frames and the engine walks one clock across
//! the whole file. So the runtime is a scheduler, not an interpreter — see
//! [`playback::stage::Stage`].
//!
//! The modules sit in four groups: [`install`] finds everything in the player's
//! own game directory, [`media`] decodes it, [`playback`] schedules a script
//! over it, and [`ui`] is the game's menus.

// Every other module is held to the same rule the workspace always had. The
// exception is `media`, whose every `unsafe` is a call into the system libav —
// Rust requires the keyword on all FFI, so it cannot be avoided there, and the
// allow is deliberately narrow and greppable.
#![deny(unsafe_code)]

pub mod install;
#[allow(unsafe_code)]
pub mod media;
pub mod playback;
pub mod ui;

pub use days_save::FlagStore;
pub use install::config::Config;
pub use install::ini::Ini;
pub use install::vfs::Vfs;
pub use playback::mixer::Mixer;
pub use playback::stage::{Stage, Visual};
pub use ui::ending::{title_backdrop, Backdrop, EndingList};
pub use ui::menu::{Action, Menu, Mode, SaveState, SystemSe};
pub use ui::options::Tab;
pub use ui::replay::{Scene, Scenes};
pub use ui::screen::{Resolution, Screen, WidgetState};
