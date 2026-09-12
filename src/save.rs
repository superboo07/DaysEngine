//! Finding the player's save data in their install.
//!
//! Save data is the one thing that does *not* live in the packs: it sits in
//! plain files next to the executable, at paths `Ini/FILMENGINE.INI` names.
//! So these take a game directory rather than a [`Vfs`](crate::vfs::Vfs).
//!
//! The format itself is [`days_save`]; this module is only where to find it.

use std::path::{Path, PathBuf};

pub use days_save::{FlagStore, Value};

use crate::ini::Ini;

/// Where `FILMENGINE.INI` says the global flag store lives.
///
/// The shipped INI says `Save/GlobalFlag.DAT`, but the key is what the engine
/// reads, so a modified install keeps working. Backslashes are the separator
/// on the platform the game was written for and mean nothing here, so they are
/// mapped across.
pub fn flag_path(game: &Path, film: &Ini) -> PathBuf {
    let relative = film.get("FlagFileName").unwrap_or("Save/GlobalFlag.DAT");
    game.join(relative.replace('\\', "/"))
}

/// Reads and decodes the global flag store.
///
/// A player who has never finished a chapter has no flag file at all, and that
/// is not an error: it reads as an empty store, which is exactly a fresh
/// install. Anything else — an unreadable or corrupt file — is logged and also
/// treated as a fresh install, following the rule that a missing or broken
/// asset costs the player a feature and never the session.
pub fn load_flags(game: &Path, film: &Ini) -> FlagStore {
    let path = flag_path(game, film);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            log::info!("no save data at {}: starting fresh", path.display());
            return FlagStore::default();
        }
        Err(err) => {
            log::warn!("reading {}: {err}", path.display());
            return FlagStore::default();
        }
    };
    match FlagStore::parse(&bytes) {
        Ok(flags) => {
            log::info!("{}: {} flags", path.display(), flags.len());
            flags
        }
        Err(err) => {
            log::warn!("decoding {}: {err}", path.display());
            FlagStore::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn takes_the_path_from_the_ini() {
        let film = Ini::parse("[FlagFileName]=\"Save\\GlobalFlag.DAT\"");
        assert_eq!(
            flag_path(Path::new("/game"), &film),
            Path::new("/game/Save/GlobalFlag.DAT")
        );
    }

    #[test]
    fn falls_back_to_the_shipped_path() {
        let film = Ini::parse("");
        assert_eq!(
            flag_path(Path::new("/game"), &film),
            Path::new("/game/Save/GlobalFlag.DAT")
        );
    }

    #[test]
    fn a_missing_file_is_a_fresh_install() {
        let film = Ini::parse("");
        assert!(load_flags(Path::new("/nonexistent-game-dir"), &film).is_empty());
    }
}
