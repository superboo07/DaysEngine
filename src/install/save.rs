//! Finding the player's save data in their install.
//!
//! Save data is the one thing that does *not* live in the packs: it sits in
//! plain files next to the executable, at paths `Ini/FILMENGINE.INI` names.
//! So these take a game directory rather than a [`Vfs`](crate::install::vfs::Vfs).
//!
//! The format itself is [`days_save`]; this module is only where to find it.

use std::path::{Path, PathBuf};

pub use days_save::{FlagStore, Mark, Slot, Value};

use crate::install::ini::Ini;

/// Substitutes a slot number into one of the INI's `printf` patterns.
///
/// The engine passes these straight to `swprintf_s`, so the pattern is
/// whatever the install's INI says: the shipped one is `Save/SaveFile00%d.DAT`,
/// which is why slot 0 is `SaveFile000.DAT` and slot 14 is `SaveFile0014.DAT`.
/// Width and zero-padding are honoured because `swprintf_s` honours them; a
/// pattern with no conversion comes back unchanged, which is what the C
/// function would do with it.
pub fn format_slot(pattern: &str, slot: u32) -> String {
    let mut out = String::with_capacity(pattern.len() + 4);
    let mut rest = pattern;
    while let Some(at) = rest.find('%') {
        out.push_str(&rest[..at]);
        let spec = &rest[at + 1..];
        if let Some(after) = spec.strip_prefix('%') {
            out.push('%');
            rest = after;
            continue;
        }
        let zero = spec.starts_with('0');
        let digits: String = spec.chars().take_while(char::is_ascii_digit).collect();
        let after = &spec[digits.len()..];
        if !after.starts_with('d') {
            // Not a conversion this understands; leave it be rather than
            // silently eat the rest of the path.
            out.push('%');
            rest = spec;
            continue;
        }
        let width: usize = digits.parse().unwrap_or(0);
        let body = slot.to_string();
        if zero {
            for _ in body.len()..width {
                out.push('0');
            }
        } else {
            for _ in body.len()..width {
                out.push(' ');
            }
        }
        out.push_str(&body);
        // Only the first conversion takes the slot number, as the one
        // argument the engine passes does.
        out.push_str(&after[1..]);
        return out;
    }
    out.push_str(rest);
    out
}

/// Where `FILMENGINE.INI` says a save slot lives.
pub fn slot_path(game: &Path, film: &Ini, slot: u32) -> PathBuf {
    let pattern = film.get("SaveFileName").unwrap_or("Save/SaveFile00%d.DAT");
    game.join(format_slot(pattern, slot).replace('\\', "/"))
}

/// The two global-store keys a slot's display line lives under.
///
/// The slot file holds no description of itself: the save/load screen reads
/// these out of `GlobalFlag.DAT`. The first is written as a timestamp with the
/// chapter label appended, the second is the player's own comment.
pub fn slot_keys(film: &Ini, slot: u32) -> (String, String) {
    let pattern = film.get("SaveConfig").unwrap_or("FILMEngine/SaveFile00%d");
    let head = format_slot(pattern, slot);
    let sub = format!("{head}_Sub");
    (head, sub)
}

/// Reads and decodes one save slot.
///
/// An empty slot is not an error — it is a slot the player has not written —
/// so a missing file reads as `None`. A corrupt one is logged and also reads
/// as `None`, following the rule that a broken asset costs a feature and never
/// the session.
pub fn load_slot(game: &Path, film: &Ini, slot: u32) -> Option<Slot> {
    let path = slot_path(game, film, slot);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return None,
        Err(err) => {
            log::warn!("reading {}: {err}", path.display());
            return None;
        }
    };
    match Slot::parse(&bytes) {
        Ok(slot) => Some(slot),
        Err(err) => {
            log::warn!("decoding {}: {err}", path.display());
            None
        }
    }
}

/// Writes one save slot.
///
/// Written to a neighbouring temporary file and renamed over the old one, so a
/// crash midway through cannot leave the player with a half-written slot where
/// a good one used to be. The game does not do this; losing a save to a power
/// cut is not a behaviour worth reproducing.
pub fn write_slot(game: &Path, film: &Ini, slot: u32, data: &Slot) -> std::io::Result<()> {
    replace(&slot_path(game, film, slot), &data.to_bytes())
}

/// Writes the global flag store back.
pub fn write_flags(game: &Path, film: &Ini, flags: &FlagStore) -> std::io::Result<()> {
    replace(&flag_path(game, film), &flags.to_bytes())
}

fn replace(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let temp = path.with_extension("tmp");
    std::fs::write(&temp, bytes)?;
    std::fs::rename(&temp, path)?;
    log::info!("wrote {} ({} bytes)", path.display(), bytes.len());
    Ok(())
}

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
    fn puts_the_slot_number_through_the_inis_own_pattern() {
        assert_eq!(
            format_slot("Save/SaveFile00%d.DAT", 0),
            "Save/SaveFile000.DAT"
        );
        assert_eq!(
            format_slot("Save/SaveFile00%d.DAT", 14),
            "Save/SaveFile0014.DAT"
        );
        assert_eq!(
            format_slot("FILMEngine/SaveFile00%d", 7),
            "FILMEngine/SaveFile007"
        );
        // Width and padding, because swprintf_s honours them.
        assert_eq!(format_slot("Save%03d.DAT", 7), "Save007.DAT");
        // Nothing to substitute, and an escaped percent.
        assert_eq!(format_slot("Save/Fixed.DAT", 3), "Save/Fixed.DAT");
        assert_eq!(format_slot("100%% %d", 2), "100% 2");
    }

    #[test]
    fn names_both_halves_of_a_slots_display_line() {
        let film = Ini::parse("[SaveConfig]=\"FILMEngine/SaveFile00%d\"");
        assert_eq!(
            slot_keys(&film, 12),
            (
                "FILMEngine/SaveFile0012".into(),
                "FILMEngine/SaveFile0012_Sub".into()
            )
        );
    }

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
