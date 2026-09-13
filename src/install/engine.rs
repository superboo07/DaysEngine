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
//!
//! It is written exactly once: when [`Settings::load`] finds no file there, it
//! writes [`template`] — every default, spelled out, with the comment that says
//! what each one does — so the first thing a player who wants to change
//! something finds is a file with the knobs already in it rather than a name
//! from a README they have to type out. Nothing after that touches it; the
//! values are its own and deleting it restores every default.
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
//! # Filtering
//!
//! `[Video] Filters` and `[Video] FiltersAfterScale` are libavfilter chains in
//! `ffmpeg -vf` syntax, run on either side of that scale, and `[Video] Grain`
//! is the dither laid over the result. What they are for and why there are two
//! of them is [`crate::media::filter`] and [`crate::media::grain`]; the short
//! of it is that repairing the encode belongs before the scale, where the
//! artifacts are still the size the encoder made them, and anything added to
//! the picture belongs after it, at the size it will be seen.
//!
//! An empty chain is no graph at all, which is the original's path exactly.
//!
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

/// The chain movie frames go through before they are scaled, by default.
///
/// `deblock` and `gradfun` are the two artifacts the retail encode actually
/// has, and both are worked out in [`crate::media::filter`]: 8x8 transform
/// blocks, and gradients quantised into bands. Both are measured in *source*
/// pixels, which is why this stage is before the scale.
///
/// `filter=weak` is the gentlest of `deblock`'s three, and `block=8` is the
/// transform size WMV3 uses. `gradfun`'s `1.2` is a shade under its default
/// strength of 1.2 rounded up from nothing — it is the amount of a step it will
/// smooth, and above about 1.5 it starts eating real detail — over a radius of
/// 16 pixels, which is the widest band the encoder's quantiser produces at this
/// bitrate.
pub const DEFAULT_FILTERS: &str = "deblock=filter=weak:block=8,gradfun=1.2:16";

/// The chain movie frames go through after they are scaled, by default.
///
/// Empty. The stage exists — it is where anything *added* to the picture
/// belongs, rather than anything repaired in it — but the one thing the engine
/// adds by default is the grain, and that is [`DEFAULT_GRAIN`] rather than a
/// filter: see [`crate::media::grain`] for the three measured reasons
/// libavfilter's `noise` is the wrong tool on a packed RGBA frame.
pub const DEFAULT_FILTERS_AFTER: &str = "";

/// The amplitude of the grain laid over the finished frame, by default.
///
/// Levels of 255, and deliberately at the bottom of the range: enough to break
/// a step between two flat areas into noise, not enough to be seen as grain on
/// a still frame. [`crate::media::grain`] is what it is for and
/// `[Video] Grain = 0` turns it off.
pub const DEFAULT_GRAIN: u8 = 2;

/// Everything `DaysEngine.ini` can say.
///
/// Not `Copy`, because two of these are filter chains and a chain is a string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub video_scaler: VideoScaler,
    /// libavfilter chain applied to each movie frame before it is scaled to
    /// the window, empty for none. See [`crate::media::filter`].
    pub video_filters: String,
    /// libavfilter chain applied after the scale, in RGBA at the window's size.
    pub video_filters_after: String,
    /// Amplitude of the grain laid over the finished frame, in levels of 255.
    /// See [`crate::media::grain`].
    pub video_grain: u8,
    pub ui_scaler: UiScaler,
    /// Draw the game at a whole-number multiple of its own size. See
    /// [`Settings::pixel_perfect`].
    pub pixel_perfect: bool,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            video_scaler: VideoScaler::default(),
            video_filters: DEFAULT_FILTERS.to_string(),
            video_filters_after: DEFAULT_FILTERS_AFTER.to_string(),
            video_grain: DEFAULT_GRAIN,
            ui_scaler: UiScaler::default(),
            pixel_perfect: false,
        }
    }
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
    /// Loads the settings from beside the running executable, writing the
    /// defaults out when there is no file there yet.
    ///
    /// Absent, unreadable, or unparseable all come to the same thing: the
    /// defaults, and a line in the log saying so. Absent is the one that also
    /// leaves a file behind — see [`Settings::write_template`], and note that
    /// it is only ever the *absent* case. A file that cannot be read or that a
    /// player has half-edited is theirs, and overwriting it would throw away
    /// the very thing they were editing.
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
                Settings::write_template(&path);
                Settings::default()
            }
            Err(err) => {
                log::warn!("reading {}: {err}; using default settings", path.display());
                Settings::default()
            }
        }
    }

    /// Writes [`template`] to `path`, for the player to edit.
    ///
    /// Every value in it is the default, so the file this leaves behind and no
    /// file at all mean exactly the same thing — which is what makes writing it
    /// unannounced safe, and what
    /// `the_template_parses_to_the_defaults` holds to.
    ///
    /// A failure here is a line in the log and nothing else. The engine is
    /// about to run on the defaults either way, and a read-only folder — a
    /// game installed under `Program Files`, an install on a mounted image —
    /// is not a reason to refuse to start.
    fn write_template(path: &std::path::Path) {
        match std::fs::write(path, template()) {
            Ok(()) => log::info!("no {}; wrote the defaults there", path.display()),
            Err(err) => log::info!(
                "no {} and it cannot be created ({err}); using default settings",
                path.display()
            ),
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
            // A chain is free text — libavfilter is the only thing that can say
            // whether it is valid, and it says so when the graph is built. An
            // empty value is the way to ask for no filtering at all.
            ("video", "filters") => value.clone_into(&mut self.video_filters),
            ("video", "filtersafterscale") => value.clone_into(&mut self.video_filters_after),
            ("video", "grain") => match value.trim().parse::<u8>() {
                Ok(amount) if amount <= crate::media::grain::MAX => self.video_grain = amount,
                _ => log::warn!(
                    "{FILE} line {line}: {value:?} is not a grain amplitude; 0 to {}",
                    crate::media::grain::MAX
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
         ; This one sits beside the daysengine binary, with every default\n\
         ; already filled in.\n\
         \n\
         [Video]\n\
         ; How a movie frame is scaled to the window. libswscale's filters:\n\
         ;   {}\n\
         ; The original left this to Direct3D's bilinear; bicubic is sharper.\n\
         Scaler = bicubic\n\
         \n\
         ; libavfilter chains, `ffmpeg -vf` syntax, run over every movie frame.\n\
         ; Empty means no filtering at all, which is what the original did.\n\
         ;\n\
         ; The movies are 800x452 at about 3 Mbit/s, and on a modern window\n\
         ; every frame is blown up more than twice: the 8x8 blocks and the\n\
         ; banded gradients the encoder left come up with it.\n\
         ;\n\
         ; Filters runs before the scale, where those artifacts are still the\n\
         ; size the encoder made them, so this is where repair goes.\n\
         Filters = {}\n\
         \n\
         ; FiltersAfterScale runs after it, on the frame at the size it will\n\
         ; be seen, which is where anything *added* to the picture belongs —\n\
         ; laid down before an upscale it comes out magnified with everything\n\
         ; else. Empty by default.\n\
         FiltersAfterScale = {}\n\
         \n\
         ; A very light grain over the finished frame, in levels of 255, 0 to\n\
         ; {}. It covers the last of the banding: what the debander judged too\n\
         ; wide to touch, and the contouring the upscale adds by interpolating\n\
         ; between levels that were already quantised. 0 turns it off.\n\
         Grain = {}\n\
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
        DEFAULT_FILTERS,
        DEFAULT_FILTERS_AFTER,
        crate::media::grain::MAX,
        DEFAULT_GRAIN,
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

    /// The template is a file this parser reads back to the defaults it claims
    /// — which is what lets [`Settings::load`] write it out on a first run
    /// without changing how the engine behaves.
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
