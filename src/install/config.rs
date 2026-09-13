//! `Config.DAT` — the player's settings, as the Option screen reads and writes
//! them.
//!
//! # The file
//!
//! It sits next to the executable and is an ordinary engine `.INI` inside a
//! four-byte container:
//!
//! ```text
//! "DFLT"      magic
//! zlib        the INI text, deflated
//! ```
//!
//! Inflated, a shipped file looks like this — a banner line, then the same
//! `[Key]="value"` lines every other `.INI` uses:
//!
//! ```text
//! < Config.dat >
//! [Format]="22"
//! [WindowWidth]="800"
//! [MasterVolume]="-1.000000"
//! [BgmVolume]="5"
//! [TextView]="-1"
//! ```
//!
//! # Why a bool is `-1`, and how one is read back
//!
//! `Config`'s writer formats every scalar with `%d` (`FUN_0046cd30` and its
//! neighbours) and the values it is handed are Windows `VARIANT`s, so a true
//! bool arrives as `VARIANT_TRUE` and prints as `-1`.
//!
//! The getters are the other half, and they are the class's vtable at
//! `0x004d73d0`: slot `+0xc` a string, `+0x10` a bool, `+0x14` an int, `+0x18`
//! a float. Each formats `[Key]="`, finds it in the text with `wcsstr`
//! (`FUN_0046e330` — so **the first occurrence wins**), takes the characters
//! from there to the first `"` or `,` (`FUN_0046dfb0`), and converts them with
//! `VariantChangeType` into `VT_BOOL`, `VT_I4` or `VT_R4`
//! (`FUN_0046de00`/`de90`/`df20`). **So a bool is true when the number is
//! non-zero**, which is why `-1` reads as true and `1` does too.
//!
//! Two edges come out of that and are easy to get wrong. A key that is missing
//! gives the caller's default, but a key that is *present* and does not convert
//! gives **zero**, because the conversion fails and the getter returns the
//! variant it initialised rather than the default. And a comma ends a value as
//! surely as the closing quote does.
//!
//! This is why the settings do not go through [`crate::install::ini::Ini`], whose
//! `get_bool` is `value == "1"` because that is right for the packs' own INIs.
//!
//! # The reader will only take so much
//!
//! `FUN_0046c520` reads the whole file into a **1024-byte** stack buffer,
//! compares four bytes against `DFLT`, and calls
//! `FUN_0046c310(file + 4, length - 4, out, 1024)`: `inflateInit_` against zlib
//! `1.2.7` with `windowBits` 15, one `inflate` with `Z_FINISH`, `inflateEnd`.
//! The out buffer then becomes a C string. So the file has to fit in 1024
//! bytes, the inflated text has to fit with room for its terminator, the stream
//! has to finish in that single pass, and a NUL anywhere ends the text.
//! `days config --roundtrip` checks a file this engine writes against all of
//! it.
//!
//! # Keys, defaults and ranges
//!
//! The menu DLL loads exactly ten settings, in `FUN_10006ce0`, each with a
//! default it passes to the getter, and writes them back in `FUN_10006e40`:
//!
//! ```text
//! VoiceVolume  int   5     BgmVolume  int   5     SeVolume  int  5
//! TextView     bool  true  MenVoice   bool  true  Mute      bool false
//! Skip         bool  false AutoDraw   bool  true  SuperSkip bool false
//! UseSOM       bool  false
//! ```
//!
//! Volumes are `0..=10` (`FUN_10007140` clamps both ends). `MasterVolume` is a
//! float the same write-back stores and the Option screen never edits through
//! any widget this engine has recovered; the shipped value is `-1.0`.
//!
//! The remaining keys — `Format`, `WindowWidth`, `WindowHeight`, `DisplayType`,
//! `TypeMiniNote`, `WindowMode`, `UseAgate`, `Wheel` — are written back
//! untouched. The Option screen reads the display ones through the host rather
//! than from here, so this module preserves them rather than interpreting them.
//!
//! # Preserving what we do not understand
//!
//! Entries are kept in file order, and a key this engine has no name for is
//! written back exactly as it was read. Saving a settings file must never drop
//! a line the real game put there, because the real game is what reads it next.

use std::path::{Path, PathBuf};

/// Magic at the head of the container.
pub const MAGIC: [u8; 4] = *b"DFLT";

/// The banner the shipped writer puts on the first line.
const BANNER: &str = "< Config.dat >";

/// The file's name in the game directory.
///
/// Not configurable as far as anything recovered goes: no `.INI` key names it,
/// and the executable's own banner calls it `Config.dat`.
pub const FILE_NAME: &str = "Config.DAT";

/// What can go wrong reading or writing the file.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{path} is not a Config.DAT: expected magic {MAGIC:?}, found {found:?}")]
    BadMagic { path: PathBuf, found: Vec<u8> },
    #[error("could not inflate {path}: {source}")]
    Inflate {
        path: PathBuf,
        source: miniz_oxide::inflate::DecompressError,
    },
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
}

/// The three volume sliders the Sound tab offers.
///
/// The order is the DLL's own: `FUN_10006fd0` maps 0 to voice, 1 to sound
/// effects and 2 to music.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    Voice,
    Se,
    Bgm,
}

impl Channel {
    /// The `Config.DAT` key.
    pub fn key(self) -> &'static str {
        match self {
            Channel::Voice => "VoiceVolume",
            Channel::Se => "SeVolume",
            Channel::Bgm => "BgmVolume",
        }
    }

    pub const ALL: [Channel; 3] = [Channel::Voice, Channel::Se, Channel::Bgm];
}

/// A boolean setting, by the key the DLL asks for.
///
/// Every one of these is loaded in `FUN_10006ce0` with the default below and
/// written back in `FUN_10006e40`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flag {
    TextView,
    MenVoice,
    Mute,
    Skip,
    AutoDraw,
    SuperSkip,
    UseSom,
}

impl Flag {
    pub fn key(self) -> &'static str {
        match self {
            Flag::TextView => "TextView",
            Flag::MenVoice => "MenVoice",
            Flag::Mute => "Mute",
            Flag::Skip => "Skip",
            Flag::AutoDraw => "AutoDraw",
            Flag::SuperSkip => "SuperSkip",
            Flag::UseSom => "UseSOM",
        }
    }

    /// The default the DLL passes to the getter when the key is absent.
    pub fn default_value(self) -> bool {
        match self {
            Flag::TextView | Flag::MenVoice | Flag::AutoDraw => true,
            Flag::Mute | Flag::Skip | Flag::SuperSkip | Flag::UseSom => false,
        }
    }

    pub const ALL: [Flag; 7] = [
        Flag::TextView,
        Flag::MenVoice,
        Flag::Mute,
        Flag::Skip,
        Flag::AutoDraw,
        Flag::SuperSkip,
        Flag::UseSom,
    ];
}

/// The default every volume loads with, from `FUN_10006ce0`.
pub const DEFAULT_VOLUME: i32 = 5;

/// The loudest a volume goes; `FUN_10007140` clamps to `0..=MAX_VOLUME`.
pub const MAX_VOLUME: i32 = 10;

/// The `MasterVolume` the shipped file carries, used when the key is absent.
pub const DEFAULT_MASTER_VOLUME: f32 = -1.0;

/// The player's settings file.
#[derive(Debug, Clone, Default)]
pub struct Config {
    /// Every `[Key]="value"` line, in file order, so a rewrite preserves keys
    /// this engine does not interpret.
    entries: Vec<(String, String)>,
    /// Whether anything has changed since the last load or save.
    dirty: bool,
}

impl Config {
    /// Where the settings live for a game directory.
    pub fn path(game: &Path) -> PathBuf {
        game.join(FILE_NAME)
    }

    /// Reads the settings for a game directory.
    ///
    /// A missing file is not an error — it is a first run, and every setting
    /// falls back to the default the DLL would have passed the getter. A file
    /// that cannot be decoded is logged and treated the same way, following the
    /// rule that a broken asset costs a feature and never the session.
    pub fn load(game: &Path) -> Config {
        let path = Config::path(game);
        match Config::read(&path) {
            Ok(config) => {
                log::info!("{}: {} settings", path.display(), config.entries.len());
                config
            }
            Err(Error::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => {
                log::info!("no settings at {}: using defaults", path.display());
                Config::default()
            }
            Err(err) => {
                log::warn!("{err}: using defaults");
                Config::default()
            }
        }
    }

    /// Reads one settings file, reporting why it could not be read.
    pub fn read(path: &Path) -> Result<Config, Error> {
        let bytes = std::fs::read(path).map_err(|source| Error::Io {
            path: path.to_path_buf(),
            source,
        })?;
        Config::parse(&bytes).map_err(|err| match err {
            Error::BadMagic { found, .. } => Error::BadMagic {
                path: path.to_path_buf(),
                found,
            },
            Error::Inflate { source, .. } => Error::Inflate {
                path: path.to_path_buf(),
                source,
            },
            other => other,
        })
    }

    /// Decodes the container and parses the INI inside it.
    pub fn parse(bytes: &[u8]) -> Result<Config, Error> {
        let Some(body) = bytes.strip_prefix(&MAGIC) else {
            return Err(Error::BadMagic {
                path: PathBuf::new(),
                found: bytes[..bytes.len().min(4)].to_vec(),
            });
        };
        let text = miniz_oxide::inflate::decompress_to_vec_zlib(body).map_err(|source| {
            Error::Inflate {
                path: PathBuf::new(),
                source,
            }
        })?;
        Ok(Config::parse_text(&String::from_utf8_lossy(&text)))
    }

    /// Parses the inflated body.
    ///
    /// Lines that are not `[Key]="value"` are dropped, which is what handles
    /// the banner — and also the broken lines the shipped writer leaves
    /// behind. A real file in the retail install has `MenVoice]="1"` on one
    /// line and `[MenVoice]="-1"` on another; skipping the one missing its
    /// bracket leaves each key exactly once, and is what the retail reader
    /// does too, since it looks a key up as the literal `[Key]="`.
    pub fn parse_text(text: &str) -> Config {
        let mut entries = Vec::new();
        for line in text.lines() {
            let line = line.trim().trim_start_matches('\u{feff}');
            let Some(rest) = line.strip_prefix('[') else {
                continue;
            };
            let Some(close) = rest.find("]=") else {
                continue;
            };
            let key = rest[..close].trim().to_string();
            // `FUN_0046dfb0` takes the text from just past the key to the
            // first `"` **or `,`** — a comma ends a value as surely as the
            // closing quote does. The opening quote is the last character of
            // the `[Key]="` the search matched, so it is skipped, not trimmed.
            let after = rest[close + 2..].trim_start();
            let body = after.strip_prefix('"').unwrap_or(after);
            let end = body.find(['"', ',']).unwrap_or(body.len());
            entries.push((key, body[..end].to_string()));
        }
        Config {
            entries,
            dirty: false,
        }
    }

    /// True when a setting has changed since the file was read or written.
    pub fn dirty(&self) -> bool {
        self.dirty
    }

    /// Every entry, in file order.
    pub fn entries(&self) -> &[(String, String)] {
        &self.entries
    }

    /// The raw text for a key, case-insensitively.
    ///
    /// **The first occurrence wins.** Every getter on the `Config` vtable at
    /// `0x004d73d0` — `FUN_0046c9e0` for a string at slot `+0xc`, `FUN_0046ca70`
    /// for a bool at `+0x10`, `FUN_0046cae0` for an int at `+0x14`,
    /// `FUN_0046cb50` for a float at `+0x18` — formats `[Key]="` and hands it
    /// to a helper whose search is `FUN_0046e330`, and `FUN_0046e330` is
    /// `wcsstr`. Reading the last one instead would make this engine and the
    /// retail game disagree about the same file.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v.as_str())
    }

    /// Replaces a key's value, or appends it if it is not there yet.
    ///
    /// The first occurrence again, and appending only when there is none, which
    /// is what `FUN_0046cd30` does: find, then either `FUN_0046d190` over the
    /// hit or `+=` on the end.
    pub fn set(&mut self, key: &str, value: impl Into<String>) {
        let value = value.into();
        match self
            .entries
            .iter_mut()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
        {
            Some(slot) => {
                if slot.1 == value {
                    return;
                }
                slot.1 = value;
            }
            None => self.entries.push((key.to_string(), value)),
        }
        self.dirty = true;
    }

    /// A stored integer, or `None` when the key is absent or not a number.
    ///
    /// The retail getter has no third answer: see [`Config::int_or`], which is
    /// the one that matches it.
    pub fn int(&self, key: &str) -> Option<i32> {
        self.get(key)?.trim().parse().ok()
    }

    /// An integer the way `FUN_0046cae0` reads one.
    ///
    /// A key that is not there at all gives the caller's default, and a key
    /// that is there gives whatever `VariantChangeType(.., VT_I4)` makes of the
    /// text — which for text that is not a number at all is **zero, not the
    /// default**, because the conversion fails and leaves the freshly
    /// initialised variant. The two are only the same when the default is zero.
    pub fn int_or(&self, key: &str, default: i32) -> i32 {
        match self.get(key) {
            Some(text) => variant_i4(text),
            None => default,
        }
    }

    /// A stored bool, the way `FUN_0046ca70` reads one.
    ///
    /// The text goes through `VariantChangeType(.., VT_BOOL)`, so a number is
    /// true when it is non-zero — which is why `-1`, the way the writer spells
    /// true, reads as true. Text that is not a number converts to false rather
    /// than to the default; only an absent key gets the default.
    pub fn flag(&self, flag: Flag) -> bool {
        match self.get(flag.key()) {
            Some(text) => variant_bool(text),
            None => flag.default_value(),
        }
    }

    /// Sets a bool, writing it the way the shipped writer does.
    pub fn set_flag(&mut self, flag: Flag, value: bool) {
        self.set(flag.key(), if value { "-1" } else { "0" });
    }

    /// A volume, clamped into the range the DLL enforces.
    pub fn volume(&self, channel: Channel) -> i32 {
        self.int_or(channel.key(), DEFAULT_VOLUME)
            .clamp(0, MAX_VOLUME)
    }

    /// Sets a volume, clamping as `FUN_10007140` does.
    pub fn set_volume(&mut self, channel: Channel, level: i32) {
        self.set(channel.key(), level.clamp(0, MAX_VOLUME).to_string());
    }

    /// `MasterVolume`, the per-step factor the level is multiplied by.
    ///
    /// `FUN_0046cb50`, so `VT_R4` and the same rule as the other two: absent
    /// gives the default, present but unconvertible gives zero.
    pub fn master_volume(&self) -> f32 {
        match self.get("MasterVolume") {
            Some(text) => variant_r4(text),
            None => DEFAULT_MASTER_VOLUME,
        }
    }

    /// The attenuation in decibels a channel's level works out to.
    ///
    /// `FUN_10006fd0`: `(11 - level) * MasterVolume`. With the shipped
    /// `MasterVolume` of -1.0 that is -1 dB at level 10 down to -11 dB at level
    /// 0 — an attenuation ladder, which is why louder is a *smaller* number.
    /// The DLL's fourth case, index 3, is a fixed level of 2 used when the
    /// player has muted; [`Config::mute_gain_db`] is that case.
    pub fn attenuation_db(&self, channel: Channel) -> f32 {
        self.level_attenuation_db(self.volume(channel))
    }

    /// The attenuation the muted case uses, from the same function's index 3.
    pub fn mute_gain_db(&self) -> f32 {
        self.level_attenuation_db(2)
    }

    fn level_attenuation_db(&self, level: i32) -> f32 {
        (11 - level) as f32 * self.master_volume()
    }

    /// A channel's attenuation as a linear gain in `0.0..=1.0`.
    ///
    /// The original hands the decibel figure to DirectSound; this engine's
    /// mixer multiplies samples, so the conversion happens here rather than
    /// silently in a driver.
    pub fn gain(&self, channel: Channel) -> f32 {
        if self.flag(Flag::Mute) {
            return 0.0;
        }
        db_to_gain(self.attenuation_db(channel))
    }

    /// Encodes the file: the banner, every entry in order, deflated under the
    /// magic.
    pub fn encode(&self) -> Vec<u8> {
        let mut text = String::with_capacity(64 + self.entries.len() * 24);
        text.push_str(BANNER);
        text.push('\n');
        for (key, value) in &self.entries {
            text.push_str(&format!("[{key}]=\"{value}\"\n"));
        }
        let mut out = MAGIC.to_vec();
        out.extend(miniz_oxide::deflate::compress_to_vec_zlib(
            text.as_bytes(),
            6,
        ));
        out
    }

    /// Writes the settings back to the game directory.
    ///
    /// This replaces a file in the player's own install, which is what the real
    /// game does when the Option screen is closed — so it happens only on that
    /// path, and the write goes to a temporary file that is renamed into place,
    /// so an interrupted save cannot leave a half-written settings file where a
    /// readable one used to be.
    pub fn save(&mut self, game: &Path) -> Result<(), Error> {
        let path = Config::path(game);
        let temp = path.with_extension("DAT.new");
        let io = |source| Error::Io {
            path: path.clone(),
            source,
        };
        std::fs::write(&temp, self.encode()).map_err(io)?;
        std::fs::rename(&temp, &path).map_err(io)?;
        self.dirty = false;
        log::info!("wrote {}", path.display());
        Ok(())
    }
}

/// The `VariantChangeType` conversions the three getters end in.
///
/// `FUN_0046de00`, `FUN_0046de90` and `FUN_0046df20` each wrap the text in a
/// `BSTR` variant, `VariantInit` a second one, call `VariantChangeType` into
/// `VT_BOOL`, `VT_I4` or `VT_R4`, and return that variant's field **whether or
/// not the call succeeded**. A failed conversion therefore reads back as the
/// zero the variant was initialised to.
///
/// What is modelled here is the part these files exercise: a decimal number,
/// with an optional sign, converts to its value, and text that is not a number
/// converts to zero. OLE's full string grammar — locale words like `True`,
/// thousands separators, a fraction rounded into an integer — is **not
/// reproduced**, because nothing the engine or the menus write uses it.
fn variant_i4(text: &str) -> i32 {
    text.trim().parse().unwrap_or(0)
}

/// `VT_BOOL`: a non-zero number is true. See [`variant_i4`] for the limits.
fn variant_bool(text: &str) -> bool {
    variant_r4(text) != 0.0
}

/// `VT_R4`. See [`variant_i4`] for the limits.
fn variant_r4(text: &str) -> f32 {
    text.trim().parse().unwrap_or(0.0)
}

/// Decibels to a linear amplitude, clamped to unity.
///
/// The recovered figures are attenuations — zero or negative — so anything
/// positive would be a gain the original could not have asked DirectSound for.
fn db_to_gain(db: f32) -> f32 {
    if db >= 0.0 {
        return 1.0;
    }
    10f32.powf(db / 20.0).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The body of the retail install's own `Config.DAT`, including the two
    /// half-overwritten lines the shipped writer left in it.
    const SHIPPED: &str = "< Config.dat >\n\
        [Format]=\"22\"\n\
        [WindowWidth]=\"800\"\n\
        [WindowHeight]=\"600\"\n\
        [DisplayType]=\"0\"\n\
        [TypeMiniNote]=\"0\"\n\
        [WindowMode]=\"0\"\n\
        [UseAgate]=\"0\"\n\
        [MasterVolume]=\"-1.000000\"\n\
        [VoiceVolume]=\"5\"\n\
        [BgmVolume]=\"5\"\n\
        [SeVolume]=\"5\"\n\
        [TextView]=\"-1\"\n\
        MenVoice]=\"1\"\n\
        [Mute]=\"0\"\n\
        [Skip]=\"0\"\n\
        [Wheel]=\"0\"\n\
        [AutoDraw]=\"-1\"\n\
        SuperSkip]=\"0\"\n\
        [UseSOM]=\"0\"\n\
        [MenVoice]=\"-1\"\n\
        [SuperSkip]=\"0\"\n";

    #[test]
    fn the_shipped_file_parses_to_one_entry_per_key() {
        let config = Config::parse_text(SHIPPED);
        // The banner and the two lines missing their opening bracket are not
        // entries; everything else is, exactly once.
        assert_eq!(config.entries.len(), 19);
        assert_eq!(config.get("Format"), Some("22"));
        assert_eq!(config.get("MenVoice"), Some("-1"));
    }

    /// `-1` is how the shipped writer spells a true bool, so it must not read
    /// as false. This is the one that would silently invert every toggle.
    #[test]
    fn a_bool_is_true_when_it_is_non_zero() {
        let config = Config::parse_text(SHIPPED);
        assert!(config.flag(Flag::TextView), "-1 is true");
        assert!(config.flag(Flag::MenVoice));
        assert!(config.flag(Flag::AutoDraw));
        assert!(!config.flag(Flag::Mute), "0 is false");
        assert!(!config.flag(Flag::Skip));
        assert!(!config.flag(Flag::SuperSkip));
        assert!(!config.flag(Flag::UseSom));

        // And "1", which the same install also contains, is true too.
        let one = Config::parse_text("[Skip]=\"1\"");
        assert!(one.flag(Flag::Skip));
    }

    /// A key that is not in the file falls back to the value the DLL passes the
    /// getter, not to false.
    #[test]
    fn an_absent_key_uses_the_dlls_own_default() {
        let empty = Config::default();
        assert!(empty.flag(Flag::TextView));
        assert!(empty.flag(Flag::MenVoice));
        assert!(empty.flag(Flag::AutoDraw));
        assert!(!empty.flag(Flag::Skip));
        for channel in Channel::ALL {
            assert_eq!(empty.volume(channel), DEFAULT_VOLUME);
        }
        assert_eq!(empty.master_volume(), DEFAULT_MASTER_VOLUME);
    }

    /// A key that is present but does not convert reads as zero, not as the
    /// default: `VariantChangeType` fails and the getter returns the variant it
    /// initialised anyway. Only a missing key gets the default.
    #[test]
    fn an_unconvertible_value_is_zero_and_not_the_default() {
        let config = Config::parse_text("[AutoDraw]=\"yes\"\n[BgmVolume]=\"loud\"\n");
        assert!(
            Flag::AutoDraw.default_value(),
            "the default this has to differ from"
        );
        assert!(!config.flag(Flag::AutoDraw));
        assert_eq!(config.volume(Channel::Bgm), 0);
        // Absent is the other case, and that one does take the default.
        let empty = Config::default();
        assert!(empty.flag(Flag::AutoDraw));
        assert_eq!(empty.volume(Channel::Bgm), DEFAULT_VOLUME);
    }

    /// `FUN_0046dfb0` stops at the first `"` **or** `,`.
    #[test]
    fn a_comma_ends_a_value_as_the_quote_does() {
        let config = Config::parse_text("[BgmVolume]=\"7,8\"\n");
        assert_eq!(config.get("BgmVolume"), Some("7"));
    }

    /// The first occurrence wins, because the retail reader searches from the
    /// start of the text and stops at the first hit.
    #[test]
    fn a_repeated_key_takes_its_first_value() {
        let config = Config::parse_text("[BgmVolume]=\"3\"\n[BgmVolume]=\"9\"\n");
        assert_eq!(config.volume(Channel::Bgm), 3);
    }

    #[test]
    fn volumes_clamp_to_the_range_the_dll_enforces() {
        let mut config = Config::default();
        config.set_volume(Channel::Bgm, 99);
        assert_eq!(config.volume(Channel::Bgm), MAX_VOLUME);
        config.set_volume(Channel::Bgm, -4);
        assert_eq!(config.volume(Channel::Bgm), 0);
        // A file with an out-of-range value is clamped on the way out too.
        let odd = Config::parse_text("[SeVolume]=\"500\"");
        assert_eq!(odd.volume(Channel::Se), MAX_VOLUME);
    }

    /// `(11 - level) * MasterVolume`, so louder is a smaller attenuation.
    #[test]
    fn the_attenuation_ladder_matches_the_dlls_formula() {
        let mut config = Config::parse_text("[MasterVolume]=\"-1.000000\"");
        config.set_volume(Channel::Bgm, 10);
        assert_eq!(config.attenuation_db(Channel::Bgm), -1.0);
        config.set_volume(Channel::Bgm, 0);
        assert_eq!(config.attenuation_db(Channel::Bgm), -11.0);
        assert_eq!(config.mute_gain_db(), -9.0);
    }

    #[test]
    fn muting_silences_every_channel() {
        let mut config = Config::default();
        assert!(config.gain(Channel::Bgm) > 0.0);
        config.set_flag(Flag::Mute, true);
        for channel in Channel::ALL {
            assert_eq!(config.gain(channel), 0.0);
        }
    }

    /// A rewrite has to survive a round trip *and* keep the keys this engine
    /// has no name for, because the real game reads the file next.
    #[test]
    fn a_saved_file_round_trips_and_keeps_unknown_keys() {
        let mut config = Config::parse_text(SHIPPED);
        config.set_volume(Channel::Bgm, 8);
        config.set_flag(Flag::Skip, true);

        let back = Config::parse(&config.encode()).expect("round trips");
        assert_eq!(back.volume(Channel::Bgm), 8);
        assert!(back.flag(Flag::Skip));
        // Untouched, uninterpreted, still there.
        assert_eq!(back.get("Format"), Some("22"));
        assert_eq!(back.get("WindowWidth"), Some("800"));
        assert_eq!(back.get("UseAgate"), Some("0"));
        assert_eq!(back.get("Wheel"), Some("0"));
    }

    #[test]
    fn writing_the_same_value_does_not_dirty_the_file() {
        let mut config = Config::parse_text(SHIPPED);
        assert!(!config.dirty());
        config.set_volume(Channel::Bgm, 5);
        assert!(!config.dirty(), "5 is what it already was");
        config.set_volume(Channel::Bgm, 6);
        assert!(config.dirty());
    }

    #[test]
    fn a_foreign_file_is_named_rather_than_misparsed() {
        let err = Config::parse(b"FlgH\x00\x00").unwrap_err();
        assert!(matches!(err, Error::BadMagic { .. }), "{err}");
        let err = Config::parse(b"DFLTnot-zlib").unwrap_err();
        assert!(matches!(err, Error::Inflate { .. }), "{err}");
    }

    #[test]
    fn a_missing_file_is_a_first_run() {
        let config = Config::load(Path::new("/nonexistent-game-dir"));
        assert!(!config.dirty());
        assert_eq!(config.volume(Channel::Bgm), DEFAULT_VOLUME);
    }

    #[test]
    fn decibels_convert_to_a_gain_that_is_never_above_unity() {
        assert_eq!(db_to_gain(0.0), 1.0);
        assert_eq!(db_to_gain(3.0), 1.0, "a positive figure is not a boost");
        assert!((db_to_gain(-6.0) - 0.501_187).abs() < 1e-5);
        assert!(db_to_gain(-11.0) < db_to_gain(-1.0));
    }
}
