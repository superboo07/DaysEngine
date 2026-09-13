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
//! `[Video] Scaler` is the picture: movie frames and still backgrounds alike,
//! both through libswscale. They are two ways of filling the same 800x452
//! stage, and a still that went through a different filter did not match the
//! clip it cut to. `[UI] Scaler` is the menus and the control bar, which are
//! art of a different kind and go through this engine's own kernel.
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
pub use crate::playback::scale::Kernel;
use std::path::PathBuf;

/// How the game's own art — menus, still backgrounds — is scaled.
///
/// The default is [`UiScaler::Pixel`], the band-limited pixel filter. The rest
/// are the Mitchell-Netravali family, one kernel with two parameters. See
/// [`crate::playback::scale`] for both.
///
/// There is no unfiltered option. Nearest neighbour at a scale that is not a
/// whole number lands some source pixels on two output pixels and some on
/// three, which is the stepped look this engine had before it filtered the art
/// at all. If whole pixels are what you want, ask for them: that is
/// [`Settings::pixel_perfect`], and it is a different thing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UiScaler {
    /// The default: crisp at any scale, without a border and without stepping.
    /// See [`crate::playback::scale::band_limited`].
    #[default]
    Pixel,
    /// `B = 1, C = 0`: the smoothest of the family, and the only one that
    /// cannot ring, because its kernel never goes negative.
    BSpline,
    /// `B = 1/3, C = 1/3`: Mitchell's own compromise, sharper than the spline
    /// and with very little ringing.
    Mitchell,
    /// `B = 0, C = 1/2`: sharpest of the three, and the only one that
    /// interpolates — a 1:1 pass through it would change nothing.
    CatmullRom,
}

impl UiScaler {
    /// The kernel this names.
    pub fn kernel(self) -> Kernel {
        match self {
            UiScaler::Pixel => Kernel::Pixel,
            UiScaler::BSpline => Kernel::B_SPLINE,
            UiScaler::Mitchell => Kernel::MITCHELL,
            UiScaler::CatmullRom => Kernel::CATMULL_ROM,
        }
    }

    fn parse(name: &str) -> Option<UiScaler> {
        Some(
            match name.trim().to_ascii_lowercase().replace(['_', '-'], "") {
                n if n == "pixel" || n == "bandlimited" || n == "sharp" => UiScaler::Pixel,
                n if n == "bspline" || n == "spline" || n == "cubic" => UiScaler::BSpline,
                n if n == "mitchell" => UiScaler::Mitchell,
                n if n == "catmullrom" || n == "catmull" => UiScaler::CatmullRom,
                _ => return None,
            },
        )
    }

    const NAMES: &'static str = "pixel, bspline, mitchell, catmull_rom";
}

/// Everything `DaysEngine.ini` can say.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Settings {
    pub video_scaler: VideoScaler,
    pub ui_scaler: UiScaler,
    /// Draw the game at a whole-number multiple of its own size. See
    /// [`Settings::pixel_perfect`].
    pub pixel_perfect: bool,
}

impl Settings {
    /// Whether the picture is scaled by a whole number, centred, with a border
    /// around whatever is left over.
    ///
    /// The game is authored at 800x450 and nothing it ships is bigger, so on
    /// any modern window every pixel of it has to become more than one. At
    /// 1920x1200 the fit-the-window scale is 2.4, and 2.4 is where the softness
    /// comes from: two source pixels in five land between destination pixels,
    /// and there is nothing a filter can do about that but blur across the gap.
    ///
    /// At a whole number there is no gap. Every source pixel becomes an exact
    /// `N` x `N` block of destination pixels, the same block every time, and
    /// nothing is resampled at all — 1920x1200 takes `N = 2`, so the game draws
    /// at 1600x900 with a border. The cost is that border, and it is the whole
    /// of the cost: this is not nearest-neighbour scaling, which is what 2.4
    /// would give if the filter were simply turned off, and which lands some
    /// source pixels on two destination pixels and some on three.
    ///
    /// Movies are still filtered on their way to the same box — they are
    /// photographic and they want it. This is about the art.
    pub fn pixel_perfect(&self) -> bool {
        self.pixel_perfect
    }
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
            ("ui", "pixelperfect") => match parse_bool(value) {
                Some(on) => self.pixel_perfect = on,
                None => log::warn!("{FILE} line {line}: {value:?} is not on or off"),
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

/// Reads the spellings of yes a person might type.
fn parse_bool(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "on" | "true" | "yes" => Some(true),
        "0" | "off" | "false" | "no" => Some(false),
        _ => None,
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
         ; Draw the game at a whole-number multiple of its own 800x450 and put\n\
         ; a border around the rest, so every pixel of the art becomes an exact\n\
         ; square block and nothing is resampled. Costs some of the screen.\n\
         PixelPerfect = off\n\
         \n\
         ; How the art is scaled when PixelPerfect is off:\n\
         ;   {}\n\
         ; `pixel` is the band-limited pixel filter, crisp at any scale, which\n\
         ; is what gamescope's PIXEL filter does. The rest are one cubic\n\
         ; family, softest first.\n\
         Scaler = pixel\n",
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
        assert_eq!(settings.ui_scaler, UiScaler::Pixel);
        assert!(!settings.pixel_perfect());
    }

    /// Whole-number scaling is off unless it is turned on, and a player may
    /// spell that several ways.
    #[test]
    fn pixel_perfect_takes_the_spellings_of_yes() {
        for on in ["1", "on", "true", "YES"] {
            let settings = Settings::parse(&format!("[UI]\nPixelPerfect = {on}\n"));
            assert!(settings.pixel_perfect(), "{on}");
        }
        for off in ["0", "off", "FALSE", "no"] {
            let settings = Settings::parse(&format!("[UI]\nPixelPerfect = {off}\n"));
            assert!(!settings.pixel_perfect(), "{off}");
        }
        assert!(
            !Settings::parse("[UI]\nPixelPerfect = maybe\n").pixel_perfect(),
            "a value nobody recognises keeps the default"
        );
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
        assert_eq!(settings.ui_scaler, UiScaler::Pixel);
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

    /// Each name hands [`crate::playback::scale`] the kernel it claims.
    #[test]
    fn the_ui_scalers_name_the_kernels_they_claim() {
        assert_eq!(UiScaler::Pixel.kernel(), Kernel::Pixel);
        assert_eq!(UiScaler::BSpline.kernel(), Kernel::Mitchell(1.0, 0.0));
        assert_eq!(UiScaler::CatmullRom.kernel(), Kernel::Mitchell(0.0, 0.5));
        assert_eq!(
            UiScaler::Mitchell.kernel(),
            Kernel::Mitchell(1.0 / 3.0, 1.0 / 3.0)
        );
    }
}
