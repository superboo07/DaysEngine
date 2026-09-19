//! The DaysEngine runtime.
//!
//! FILMEngine plays a script as a **timeline**, not a program: every statement
//! owns a `[start, end)` window in frames and the engine walks one clock across
//! the whole file. So the runtime is a scheduler, not an interpreter — see
//! [`playback::stage::Stage`].
//!
//! The modules sit in four groups: [`install`] finds everything in the player's
//! own game directory, [`media`] decodes it, [`playback`] schedules a script
//! over it, and [`ui`] is the game's menus. [`game`] is the loop that drives
//! all four, and [`inspect`] the subcommands that look at an install without
//! playing it; both are here rather than in `src/main.rs` because Android
//! loads a shared object and calls `SDL_main` instead of running a `main`.

// The rule is that `unsafe` here only ever means "FFI Rust requires the keyword
// on, with no safe binding to call instead", and that every such call is narrow
// and greppable. Two places qualify: `media`, whose every `unsafe` is a call
// into the system libav, and `install::clock::local_offset`, which reads the
// machine's UTC offset out of SDL because the safe `sdl3` crate wraps no part of
// `SDL_time.h`. `clock` carries its own allow on that one function rather than
// on the module.
#![deny(unsafe_code)]

pub mod game;
pub mod inspect;
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
