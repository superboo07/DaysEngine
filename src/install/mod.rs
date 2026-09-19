//! Reading the player's installed game.
//!
//! Nothing here is bundled: every byte the engine runs on is found in the
//! user's own install at runtime. This is the layer that finds it — the packs
//! ([`vfs`]), the engine's `.INI` dialect ([`ini`]) that names everything
//! inside them, the player's settings file ([`config`]) and the save data
//! ([`save`]) that sits beside the executable rather than in a pack, the
//! affection counters the branch system keeps ([`feeling`]) and the player's
//! place in the branch graph ([`progress`]).
//!
//! Two files here are the exception, and they are ours rather than theirs:
//! [`engine`] reads `DaysEngine.ini`, which holds the choices the original
//! never had to make, and [`binding`] is the part of it that says which key or
//! which controller button does what.
//!
//! Every read and write of a file in the install goes through [`storage`]
//! rather than through `std::fs` directly. On a desktop the two are the same
//! thing; on Android the player grants a folder and not a path, and that is
//! the seam where the difference lives.

pub mod binaries;
pub mod binding;
pub mod clock;
pub mod config;
pub mod dialog;
pub mod engine;
pub mod feeling;
pub mod ini;
pub mod progress;
/// The install as an Android Storage Access Framework tree.
#[cfg(target_os = "android")]
pub mod saf;
pub mod save;
pub mod storage;
pub mod vfs;
