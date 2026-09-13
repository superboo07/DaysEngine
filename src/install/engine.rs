//! `DaysEngine.ini` — the settings that are ours rather than the game's.
//!
//! Everything else this engine reads is the player's: their packs, their
//! `FILMENGINE.INI`, their `Config.DAT`. This one file is not. It holds the
//! choices the original never had to make because it handed its frames to
//! Direct3D and took the driver's bilinear filter — which filter to scale a
//! movie with, which to scale the UI art with — and it is the place for the
//! ones after them.
//!
//! # Where it lives
//!
//! **Beside the `daysengine` binary**, not in the game directory. The binaries
//! are what the player drops into their install, so in practice the two are
//! usually the same folder; but the settings belong to the engine, and looking
//! for them where the engine is means running from anywhere still finds them.
//! The file is optional and never written: with no file at all, every value
//! below is its default.
//!
//! # The format
//!
//! Ordinary Windows INI, and deliberately not the game's dialect — the game's
//! `.INI` files have no sections and spell every line `[Key]="value"` (see
//! [`super::ini`]). This is a file for a person to edit:
//!
//! ```text
//! ; DaysEngine.ini
//! [Video]
//! Scaler = bicubic
//!
//! [UI]
//! Scaler = bspline
//! ```
//!
//! Section and key names are case-insensitive, `;` and `#` start a comment, and
//! an unknown key or an unreadable value is a warning and nothing more: a
//! settings file should never be the reason the game will not start.
//!
//! # Not here yet
//!
//! A libavfilter graph — a debander before the scale, say. The decoder ties its
//! scaler to the frame it is converting ([`crate::media::VideoDecoder`]) and a
//! filtergraph would sit between the two, so it wants a `Filters` key in
//! `[Video]` and a graph built alongside the scaler. **That is not
//! implemented**, and there is no key for it rather than a key that quietly
//! does nothing.

pub use crate::media::VideoScaler;
use std::path::PathBuf;

/// How the game's own art — menus, still backgrounds — is scaled.
///
/// These are the Mitchell-Netravali family, which is one kernel with two
/// parameters; see [`crate::playback::scale`].
///
/// There is no unfiltered option, unlike [`VideoScaler`], and the reason is
/// that it could not be honoured. UI art is scaled twice — once from the native
/// 800x450 layout into the display map's space, and again from there onto the
/// window, the second time by the GPU. Turning the first one off would leave
/// the second, so "nearest" would not come out nearest. What it would come out
/// is the stepped look this engine had before it filtered the art at all, which
/// is the bug that started this.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UiScaler {
    /// The default. `B = 1, C = 0`: the smoothest of the family, and the only
    /// one that cannot ring, because its kernel never goes negative.
    #[default]
    BSpline,
    /// `B = 1/3, C = 1/3`: Mitchell's own compromise, sharper than the spline
    /// and with very little ringing.
    Mitchell,
    /// `B = 0, C = 1/2`: sharpest of the three, and the only one that
    /// interpolates — a 1:1 pass through it would change nothing.
    CatmullRom,
}

impl UiScaler {
    /// The `(B, C)` this is, in the Mitchell-Netravali family.
    pub fn mitchell(self) -> (f32, f32) {
        match self {
            UiScaler::BSpline => (1.0, 0.0),
            UiScaler::Mitchell => (1.0 / 3.0, 1.0 / 3.0),
            UiScaler::CatmullRom => (0.0, 0.5),
        }
    }

    fn parse(name: &str) -> Option<UiScaler> {
        Some(
            match name.trim().to_ascii_lowercase().replace(['_', '-'], "") {
                n if n == "bspline" || n == "spline" || n == "cubic" => UiScaler::BSpline,
                n if n == "mitchell" => UiScaler::Mitchell,
                n if n == "catmullrom" || n == "catmull" => UiScaler::CatmullRom,
                _ => return None,
            },
        )
    }

    const NAMES: &'static str = "bspline, mitchell, catmull_rom";
}

/// Everything `DaysEngine.ini` can say.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Settings {
    pub video_scaler: VideoScaler,
    pub ui_scaler: UiScaler,
}

/// The file's name, looked for beside the running binary.
pub const FILE: &str = "DaysEngine.ini";

impl Settings {
    /// Loads the settings from beside the running executable.
    ///
    /// Absent, unreadable, or unparseable all come to the same thing: the
    /// defaults, and a line in the log saying so.
    pub fn load() -> Settings {
        let Some(path) = Settings::path() else {
            log::info!("cannot find this binary's own directory; using default settings");
            return Settings::default();
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                let settings = Settings::parse(&text);
                log::info!("{} : {settings:?}", path.display());
                settings
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                log::info!("no {}; using default settings", path.display());
                Settings::default()
            }
            Err(err) => {
                log::warn!("reading {}: {err}; using default settings", path.display());
                Settings::default()
            }
        }
    }

    /// Where [`Settings::load`] looks.
    pub fn path() -> Option<PathBuf> {
        let exe = std::env::current_exe().ok()?;
        Some(exe.parent()?.join(FILE))
    }

    /// Parses the file's text. Every value that cannot be read keeps its
    /// default and says why.
    pub fn parse(text: &str) -> Settings {
        let mut settings = Settings::default();
        let mut section = String::new();
        for (number, line) in text.lines().enumerate() {
            let line = line.trim().trim_start_matches('\u{feff}');
            let line = match line.find([';', '#']) {
                Some(at) => line[..at].trim(),
                None => line,
            };
            if line.is_empty() {
                continue;
            }
            if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                section = name.trim().to_ascii_lowercase();
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                log::warn!("{FILE} line {}: {line:?} is not `key = value`", number + 1);
                continue;
            };
            let key = key.trim().to_ascii_lowercase();
            // Quotes are not the format, but a player coming from the game's
            // own `.INI` files will write them.
            let value = value.trim().trim_matches('"');
            settings.set(&section, &key, value, number + 1);
        }
        settings
    }

    fn set(&mut self, section: &str, key: &str, value: &str, line: usize) {
        match (section, key) {
            ("video", "scaler") => match VideoScaler::from_name(value) {
                Some(scaler) => self.video_scaler = scaler,
                None => log::warn!(
                    "{FILE} line {line}: {value:?} is not a video scaler; one of: {}",
                    VideoScaler::NAMES
                ),
            },
            ("ui", "scaler") => match UiScaler::parse(value) {
                Some(scaler) => self.ui_scaler = scaler,
                None => log::warn!(
                    "{FILE} line {line}: {value:?} is not a UI scaler; one of: {}",
                    UiScaler::NAMES
                ),
            },
            ("", _) => log::warn!("{FILE} line {line}: {key} is outside any [Section]"),
            _ => log::warn!("{FILE} line {line}: nothing reads [{section}] {key}"),
        }
    }
}

/// A commented file of the defaults, for a player who wants somewhere to start.
/// Never used by the game itself — this is what `days settings` prints.
pub fn template() -> String {
    format!(
        "; {FILE} — DaysEngine's own settings. Not the game's: nothing in here\n\
         ; comes from your install, and deleting this file restores every default.\n\
         ;\n\
         ; This one sits beside the daysengine binary.\n\
         \n\
         [Video]\n\
         ; How a movie frame is scaled to the window. libswscale's filters:\n\
         ;   {}\n\
         ; The original left this to Direct3D's bilinear; bicubic is sharper.\n\
         Scaler = bicubic\n\
         \n\
         [UI]\n\
         ; How the game's own art — menus, still backgrounds — is scaled:\n\
         ;   {}\n\
         ; One cubic family, softest first. There is no unfiltered option:\n\
         ; the original filters this art too.\n\
         Scaler = bspline\n",
        VideoScaler::NAMES,
        UiScaler::NAMES,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The defaults are what the engine shipped doing before there was a file
    /// to say otherwise.
    #[test]
    fn no_file_means_the_engine_defaults() {
        let settings = Settings::parse("");
        assert_eq!(settings.video_scaler, VideoScaler::Bicubic);
        assert_eq!(settings.ui_scaler, UiScaler::BSpline);
    }

    /// Sections and keys are case-insensitive, comments and quotes come off,
    /// and blank lines are nothing.
    #[test]
    fn a_file_a_person_typed_still_parses() {
        let settings = Settings::parse(
            "; a comment\n\
             \n\
             [VIDEO]\n\
             ScAlEr = \"lanczos\"   ; sharper\n\
             \n\
             [ui]\n\
             scaler=Catmull-Rom\n",
        );
        assert_eq!(settings.video_scaler, VideoScaler::Lanczos);
        assert_eq!(settings.ui_scaler, UiScaler::CatmullRom);
    }

    /// A value nobody recognises keeps the default rather than stopping the
    /// game, which is the rule every other file here follows.
    #[test]
    fn a_value_that_makes_no_sense_keeps_the_default() {
        let settings = Settings::parse("[Video]\nScaler = magic\n[UI]\nScaler = \n");
        assert_eq!(settings.video_scaler, VideoScaler::Bicubic);
        assert_eq!(settings.ui_scaler, UiScaler::BSpline);
    }

    /// Every name the template offers is one the parser takes: the two lists
    /// are written out separately and would otherwise drift.
    #[test]
    fn every_name_the_template_lists_parses() {
        for name in VideoScaler::NAMES.split(',') {
            assert!(
                VideoScaler::from_name(name).is_some(),
                "video scaler {name:?} is listed but not accepted"
            );
        }
        for name in UiScaler::NAMES.split(',') {
            assert!(
                UiScaler::parse(name).is_some(),
                "UI scaler {name:?} is listed but not accepted"
            );
        }
    }

    /// The template is a file this parser reads back to the defaults it claims.
    #[test]
    fn the_template_parses_to_the_defaults() {
        assert_eq!(Settings::parse(&template()), Settings::default());
    }

    /// `B = 1, C = 0` is the B-spline and `B = 0, C = 1/2` is Catmull-Rom,
    /// which is what [`crate::playback::scale`] is handed.
    #[test]
    fn the_ui_kernels_are_the_family_they_claim() {
        assert_eq!(UiScaler::BSpline.mitchell(), (1.0, 0.0));
        assert_eq!(UiScaler::CatmullRom.mitchell(), (0.0, 0.5));
        assert_eq!(UiScaler::Mitchell.mitchell(), (1.0 / 3.0, 1.0 / 3.0));
    }
}
