//! Reading the player's installed game.
//!
//! Nothing here is bundled: every byte the engine runs on is found in the
//! user's own install at runtime. This is the layer that finds it — the packs
//! ([`vfs`]), the engine's `.INI` dialect ([`ini`]) that names everything
//! inside them, the player's settings file ([`config`]) and the save data
//! ([`save`]) that sits beside the executable rather than in a pack, plus the
//! affection counters the branch system keeps ([`feeling`]).

pub mod config;
pub mod feeling;
pub mod ini;
pub mod save;
pub mod vfs;
