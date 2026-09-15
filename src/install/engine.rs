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
//! The engine keeps it complete, and never keeps it any other way. With no
//! file there, [`Settings::load`] writes [`template`] — every default, spelled
//! out, with the comment that says what each one does — so the first thing a
//! player who wants to change something finds is a file with the knobs already
//! in it rather than a name from a README they have to type out. With a file
//! that is missing something, [`top_up`] writes the missing part into it and
//! leaves the rest exactly as it was: a setting nobody has written down is a
//! setting nobody can find, and a file from an older build would otherwise
//! never mention the twenty rebindable actions at all.
//!
//! That is the only editing of the file there is. Nothing already in it is
//! reordered, reworded or re-valued, every value written is the default the
//! engine was about to use anyway, and deleting the file restores every
//! default.
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
//! # Input and rumble
//!
//! `[Input]` is the binding table — every key and every controller button,
//! rebindable. See [`super::binding`].
//!
//! `[Rumble] Strength` scales what the game's own `MoveSom` statements ask
//! for. The toy those statements were written for is a serial device nobody
//! can buy any more; a controller's rumble motor takes the same numbers. The
//! Option screen's SOMCON tab is the switch, exactly as it always was, and
//! `Strength = 0` turns the whole thing off here instead.
//!
pub use super::binding::{Action, Bindings, Trigger};
use super::binding::{
    DEFAULT_CURSOR_SPEED, DEFAULT_DEADZONE, DEFAULT_REPEAT_DELAY, DEFAULT_REPEAT_INTERVAL,
};
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
/// Three filters for the two artifacts the retail encode has — 8x8 transform
/// blocks, and gradients quantised into bands — both of which are measured in
/// *source* pixels, which is why this stage runs before the scale. See
/// [`crate::media::filter`].
///
/// `deblock` takes the blocks. `filter=weak` is the gentler of its two and
/// `block=8` is the transform size WMV3 uses, against a default of 16.
///
/// `deband` and `gradfun` are both debanders and both are here because they
/// fail differently, which is measurable. On a close-up of dark hair — where
/// the banding in this encode is worst, the steps being a level or two apart in
/// a region the eye is most sensitive in — the fraction of the frame lying in
/// flat runs of eight pixels or more goes 5.9% unfiltered, 5.1% with `gradfun`
/// alone, 2.7% with `deband` alone, and 1.5% with both. `gradfun` fits a
/// gradient and can only move a pixel by its strength, so it repairs a shallow
/// ramp and leaves a step; `deband` replaces a pixel from references a radius
/// away when they are all within a threshold of it, so it breaks the step and
/// leaves the ramp alone.
///
/// Both are left at their own defaults — `deband`'s threshold of 0.02 and
/// radius of 16, `gradfun`'s strength of 1.2 and radius of 16 — because past
/// them the filters stop repairing and start inventing: at a `deband` threshold
/// of 0.028 the *strong* edges in the same frame, the ones a picture is made
/// of, measure 150% of the unfiltered frame's, and at 0.035 they measure 179%.
/// That is not detail being kept. It is contrast being manufactured.
pub const DEFAULT_FILTERS: &str = "deblock=filter=weak:block=8,deband=r=16,gradfun=1.2:16";

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

/// How hard a `MoveSom` level is felt, as a percentage of what it asks for.
///
/// 100 is the level the script named, unaltered: the game's own five
/// intensities are `0x33`, `0x66`, `0x99`, `0xcc`, `0xff` — a fifth of full
/// scale apiece — and at 100 they arrive as a fifth of the motor apiece. Above
/// 100 for a weak motor, 0 to turn rumble off without touching the game's own
/// `UseSOM` setting. See [`crate::playback::som`].
pub const DEFAULT_RUMBLE_STRENGTH: u16 = 100;

/// The most [`Settings::rumble_strength`] will take.
pub const MAX_RUMBLE_STRENGTH: u16 = 400;

/// Everything `DaysEngine.ini` can say.
///
/// Not `Copy`, because two of these are filter chains and a chain is a string.
#[derive(Debug, Clone, PartialEq)]
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
    /// Which key and which controller button does what. See
    /// [`super::binding`].
    pub bindings: Bindings,
    /// Percentage of the level a `MoveSom` asks for that reaches the motor.
    /// See [`DEFAULT_RUMBLE_STRENGTH`].
    pub rumble_strength: u16,
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
            bindings: Bindings::default(),
            rumble_strength: DEFAULT_RUMBLE_STRENGTH,
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
                Settings::top_up_file(&path, &text);
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
    /// This one *replaces*, so it is only ever reached for a file that is not
    /// there. A file that cannot be read, or that a player has half-edited, is
    /// theirs; [`top_up`] is what adds to one of those, and it never takes
    /// anything away.
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

    /// Writes anything the file does not mention into it, keeping every line
    /// the player wrote.
    ///
    /// A setting nobody has written down is a setting nobody can find. The
    /// bindings are the case that makes this worth doing — twenty actions
    /// whose names are the only documentation of what can be rebound, and a
    /// file written by an older build has none of them — but the rule is the
    /// same for every key: if the engine reads it, the file says so.
    ///
    /// This is the one thing that edits a file the player owns, so it is
    /// strictly additive. Nothing is reordered, nothing is reworded and no
    /// value is ever changed; a missing section arrives as the whole commented
    /// block from [`template`], and a missing key arrives as one line at the
    /// end of the section it belongs to. Every value written is the default,
    /// which is what the engine was about to use anyway — so the file after
    /// this and the file before it mean exactly the same thing, which is what
    /// makes doing it unannounced safe.
    ///
    /// A failure is a line in the log. A read-only install is not a reason to
    /// refuse to start, and the settings are already in hand.
    fn top_up_file(path: &std::path::Path, text: &str) {
        let Some(filled) = top_up(text) else {
            return;
        };
        match std::fs::write(path, filled) {
            Ok(()) => log::info!(
                "{} did not mention every setting; wrote the missing ones in",
                path.display()
            ),
            Err(err) => log::info!(
                "{} is missing settings and cannot be written ({err})",
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
            ("rumble", "strength") => match value.trim().parse::<u16>() {
                Ok(percent) if percent <= MAX_RUMBLE_STRENGTH => self.rumble_strength = percent,
                _ => log::warn!(
                    "{FILE} line {line}: {value:?} is not a rumble strength; 0 to {MAX_RUMBLE_STRENGTH}"
                ),
            },
            // The binding table owns its own keys, because there are twenty of
            // them and they are the actions themselves.
            ("input", _) => {
                if !self.bindings.set_from_ini(key, value, line) {
                    log::warn!("{FILE} line {line}: nothing reads [{section}] {key}");
                }
            }
            ("", _) => log::warn!("{FILE} line {line}: {key} is outside any [Section]"),
            _ => log::warn!("{FILE} line {line}: nothing reads [{section}] {key}"),
        }
    }
}

/// One `[Section]` of [`template`]: its lowercased name, and every line of it
/// including the header.
///
/// The blocks are read back out of the generated template rather than kept
/// beside it, so the two cannot drift: whatever `daysengine settings --template`
/// prints is exactly what a file missing a section is given.
fn template_blocks() -> Vec<(String, Vec<String>)> {
    let mut blocks: Vec<(String, Vec<String>)> = Vec::new();
    for line in template().lines() {
        match section_header(line) {
            Some(name) => blocks.push((name, vec![line.to_string()])),
            // Everything before the first header is the file's own preamble
            // and belongs to no section, so it is dropped.
            None => {
                if let Some((_, body)) = blocks.last_mut() {
                    body.push(line.to_string());
                }
            }
        }
    }
    blocks
}

/// The lowercased name of a `[Section]` header, or `None` for any other line.
fn section_header(line: &str) -> Option<String> {
    let line = line.trim().trim_start_matches('\u{feff}');
    let name = line.strip_prefix('[')?.strip_suffix(']')?;
    Some(name.trim().to_ascii_lowercase())
}

/// The lowercased key a `key = value` line sets, ignoring comments.
fn setting_key(line: &str) -> Option<String> {
    let line = line.trim().trim_start_matches('\u{feff}');
    let line = match line.find([';', '#']) {
        Some(at) => line[..at].trim(),
        None => line,
    };
    if line.is_empty() || section_header(line).is_some() {
        return None;
    }
    let (key, _) = line.split_once('=')?;
    Some(key.trim().to_ascii_lowercase())
}

/// `text` with every setting it does not mention written into it, or `None`
/// when it already mentions them all.
///
/// See [`Settings::top_up_file`] for why this exists and what it promises.
/// Additive only: existing lines come back untouched and in their own order.
pub fn top_up(text: &str) -> Option<String> {
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();

    // Where each section the file already has begins, where its content ends,
    // and which keys it carries. A key before any header belongs to no
    // section and so matches nothing in the template.
    struct Present {
        name: String,
        /// One past the section's last non-blank line, which is where a
        /// missing key is inserted — tight against the content rather than
        /// after the gap before the next header.
        end: usize,
        keys: Vec<String>,
    }
    let mut present: Vec<Present> = Vec::new();
    for (at, line) in lines.iter().enumerate() {
        if let Some(name) = section_header(line) {
            present.push(Present {
                name,
                end: at + 1,
                keys: Vec::new(),
            });
            continue;
        }
        let Some(section) = present.last_mut() else {
            continue;
        };
        if !line.trim().is_empty() {
            section.end = at + 1;
        }
        if let Some(key) = setting_key(line) {
            section.keys.push(key);
        }
    }

    // What to add: whole blocks for the sections that are absent, and single
    // lines for the keys that are not.
    let mut append: Vec<String> = Vec::new();
    let mut insert: Vec<(usize, Vec<String>)> = Vec::new();
    for (name, body) in template_blocks() {
        let Some(at) = present.iter().position(|p| p.name == name) else {
            if !append.is_empty() {
                append.push(String::new());
            }
            append.extend(body);
            continue;
        };
        let missing: Vec<String> = body
            .iter()
            .filter(|line| setting_key(line).is_some_and(|key| !present[at].keys.contains(&key)))
            .cloned()
            .collect();
        if !missing.is_empty() {
            insert.push((present[at].end, missing));
        }
    }
    if append.is_empty() && insert.is_empty() {
        return None;
    }

    // Back to front, so an insertion does not move the ones still to come.
    insert.sort_by_key(|(at, _)| *at);
    for (at, missing) in insert.into_iter().rev() {
        lines.splice(at..at, missing);
    }
    if !append.is_empty() {
        if lines.last().is_some_and(|line| !line.trim().is_empty()) {
            lines.push(String::new());
        }
        lines.extend(append);
    }
    let mut out = lines.join("\n");
    out.push('\n');
    Some(out)
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
/// Never used by the game itself — this is what `daysengine settings` prints.
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
         Scaler = pixel\n\
         \n\
         [Input]\n\
         ; Every control, rebindable. The keys below are the ones the original\n\
         ; game uses; the pad: entries beside them are this engine's own, and\n\
         ; are why the game can be played with a controller at all.\n\
         ;\n\
         ; A trigger is a key by SDL's name for it (`up`, `space`, `keypad\n\
         ; enter`), a pad button (`pad:a`, `pad:dpup`, `pad:leftshoulder`), or\n\
         ; a pad axis pushed one way (`pad:-lefty` is the left stick up,\n\
         ; `pad:+righttrigger` is the right trigger pulled).\n\
         ;\n\
         ; Naming an action REPLACES its list — so one trigger here means one\n\
         ; trigger, not one more. An empty value turns the action off.\n\
         ;\n\
         ; Left and Right move the selection where there is one; during\n\
         ; playback with nothing selected they seek, which is what the arrow\n\
         ; keys have always done. FocusBar puts the selection on the control\n\
         ; bar, which is otherwise reachable only with a pointer.\n\
         {}\n\
         \n\
         ; How far a stick has to move before it counts as pressed, of 32767.\n\
         Deadzone = {}\n\
         \n\
         ; A held direction waits this long, then repeats this often (ms).\n\
         RepeatDelay = {}\n\
         RepeatInterval = {}\n\
         \n\
         ; How fast the right stick moves the pointer, in pixels a second.\n\
         ; 0 leaves the pointer to the mouse.\n\
         CursorSpeed = {}\n\
         \n\
         [Rumble]\n\
         ; The game's scripts carry `MoveSom` statements: five intensities for\n\
         ; a peripheral that talks over a COM port, which is a device nobody\n\
         ; can buy any more. They are levels, and a controller's rumble motor\n\
         ; takes levels, so this engine sends them there.\n\
         ;\n\
         ; The switch is the game's own: the Option screen's SOMCON tab, where\n\
         ; `Port number` picks which connected controller feels them. This is\n\
         ; only how hard, as a percentage of what the script asked for. 0 to\n\
         ; {}; 0 turns rumble off without touching the game's own setting.\n\
         Strength = {}\n",
        VideoScaler::NAMES,
        DEFAULT_FILTERS,
        DEFAULT_FILTERS_AFTER,
        crate::media::grain::MAX,
        DEFAULT_GRAIN,
        UiScaler::NAMES,
        Bindings::default().template_body().trim_end(),
        DEFAULT_DEADZONE,
        DEFAULT_REPEAT_DELAY,
        DEFAULT_REPEAT_INTERVAL,
        DEFAULT_CURSOR_SPEED,
        MAX_RUMBLE_STRENGTH,
        DEFAULT_RUMBLE_STRENGTH,
    )
}

#[cfg(test)]
mod tests {

    /// A file from a build that had no bindings gains the whole `[Input]`
    /// block — with its comments, so the player can see what a trigger looks
    /// like — and every line they wrote survives, values and all.
    #[test]
    fn a_file_without_a_section_gains_the_whole_block() {
        let theirs = "[Video]\nScaler = lanczos\nFilters =\nFiltersAfterScale =\nGrain = 0\n";
        let filled = top_up(theirs).expect("the input and rumble sections are missing");

        assert!(
            filled.starts_with(theirs),
            "their own lines come back first"
        );
        assert!(filled.contains("[Input]"));
        assert!(filled.contains("[Rumble]"));
        assert!(
            filled.contains("; A trigger is a key by SDL's name for it"),
            "the block arrives commented, not as bare keys"
        );
        // Their values are the point: topping up must never reset one.
        let settings = Settings::parse(&filled);
        assert_eq!(settings.video_scaler, VideoScaler::Lanczos);
        assert_eq!(settings.video_grain, 0);
        assert_eq!(settings.bindings, Bindings::default());
    }

    /// A section that is present but short gains only the keys it is short
    /// of, inside its own section rather than appended to the file — a
    /// `Confirm` line written after `[Rumble]` would set nothing.
    #[test]
    fn a_short_section_gains_only_what_it_is_missing_and_keeps_its_own() {
        let theirs = "[Input]\nConfirm = pad:x\n\n[UI]\nScaler = mitchell\n";
        let filled = top_up(theirs).expect("nineteen actions are missing");

        let input_at = filled.find("[Input]").unwrap();
        let ui_at = filled.find("[UI]").unwrap();
        let cancel_at = filled
            .find("Cancel")
            .expect("a missing action was written in");
        assert!(
            input_at < cancel_at && cancel_at < ui_at,
            "the missing keys land inside [Input], not at the end of the file"
        );
        assert_eq!(
            filled.matches("Confirm").count(),
            1,
            "no key is written twice"
        );

        let settings = Settings::parse(&filled);
        assert_eq!(
            settings.bindings.triggers(Action::Confirm),
            [Trigger::Button("x".into())],
            "their binding is untouched"
        );
        assert_eq!(
            settings.bindings.triggers(Action::Cancel),
            Bindings::default().triggers(Action::Cancel)
        );
        assert_eq!(settings.ui_scaler, UiScaler::Mitchell);
    }

    /// A file that already says everything is left alone — which is what
    /// makes topping up on every start safe, and what stops the file growing
    /// a copy of itself once a run.
    #[test]
    fn a_complete_file_is_not_touched() {
        assert_eq!(top_up(&template()), None);
        // And the pass that does fill one in is itself complete.
        let filled = top_up("[Video]\nGrain = 1\n").expect("almost everything is missing");
        assert_eq!(top_up(&filled), None, "topping up is done in one pass");
    }

    /// A key commented out is a key the file does not mention, so it comes
    /// back — a player who deletes a line to get the default gets the line
    /// that says what the default is.
    #[test]
    fn a_commented_out_key_counts_as_missing() {
        let filled = top_up("[Rumble]\n; Strength = 40\n").expect("Strength is commented out");
        assert!(filled.contains("\nStrength = 100"));
        assert!(filled.contains("; Strength = 40"), "their comment survives");
    }
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
