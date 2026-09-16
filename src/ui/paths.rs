//! Where each menu screen's art lives, taken from the user's own menu module.
//!
//! # Why this is not a table in this file
//!
//! Every screen is three files sharing a stem — `NAME.png`, `NAME_Chip.png`,
//! `NAME*.cmap` — and the module spells that stem itself: each screen's cmap
//! loader ends in a `wchar_t *` holding one of four literals, the plain map and
//! three widescreen ones, and hands it to the loader. So the stems are not
//! knowledge this engine has to carry. They are UTF-16 literals in the module's
//! `.rdata`, and they are read back out of the file the player installed.
//!
//! Two titles on this engine spell them differently. School Days HQ gives each
//! variant of a screen its own map; Shiny Days consolidates several into one
//! and puts the popups under a `Popup/` directory:
//!
//! ```text
//! mode 4  Option    System/Option/Option_%s.cmap     System/Option/OptionBase.cmap
//! mode 5  Replay    System/Replay/Replay_%s.cmap     System/Replay/ReplayBase.cmap
//! mode 6  RouteMap  System/RouteMap/%02d/RouteMap%s  System/RouteMap/RouteMap.cmap
//! mode 7  Pop_Som   System/Option/Pop_Som.cmap       System/Option/Popup/Pop_Som.cmap
//! mode 8  Pop_Reply System/Replay/Pop_Replay_%s      System/Replay/Popup/Pop_Replay_%s
//! ```
//!
//! [`Paths`] does not decide which title it is looking at, and there is no
//! name for either generation anywhere in this file. For each mode it tries the
//! spellings it knows in order and keeps **the one the module actually holds**,
//! so a module holding neither leaves that mode without a screen — which is
//! the right answer, because that module has no such screen — and a third
//! title's spelling is one more line in [`CANDIDATES`].
//!
//! # Provenance
//!
//! Each Shiny Days spelling below was read out of the cmap loader that builds
//! it, in `SysMenuSD.dll`: `FUN_10007c50` (Option), `FUN_100295e0` (Replay),
//! `FUN_100125b0` (RouteMap), `FUN_1001b0f0` (SaveLoad), `FUN_1002f3f0`
//! (Title), `FUN_1000faf0` (Exit), `FUN_1002e950` (Pop_Som), `FUN_10022d80`
//! (Pop_Replay), `FUN_1000d7f0` (DressSelect). The School Days HQ spellings
//! are the ones this engine already ran on.

use crate::install::config::Sound;
use days_route::pe::Image;

/// Every spelling of every screen's hit map this engine knows, best first.
///
/// The first entry for a mode that the module actually holds is the one used,
/// so order within a mode is only a tie-break and no module has ever held two.
/// A mode whose spellings are all absent has no screen in that module.
const CANDIDATES: &[(i32, &str)] = &[
    (2, "System/Title/%s.cmap"),
    (3, "System/SaveLoad/SaveLoad.cmap"),
    (4, "System/Option/OptionBase.cmap"),
    (4, "System/Option/Option_%s.cmap"),
    (5, "System/Replay/ReplayBase.cmap"),
    (5, "System/Replay/Replay_%s.cmap"),
    (6, "System/RouteMap/RouteMap.cmap"),
    (6, "System/RouteMap/%02d/RouteMap%s.cmap"),
    (7, "System/Option/Popup/Pop_Som.cmap"),
    (7, "System/Option/Pop_Som.cmap"),
    (8, "System/Replay/Popup/Pop_Replay_%s.cmap"),
    (8, "System/Replay/Pop_Replay_%s.cmap"),
    (9, "System/DressSelect/DressSelect.cmap"),
    (-1, "System/Exit/Popup.cmap"),
];

/// The cleared-title art, for a module that keeps it beside the plain title's
/// hit map rather than giving it one of its own.
///
/// `FUN_1002f550` picks between `Title.png` and `Clear/Title_Clear.png`, and
/// `FUN_1002f3f0` fills the `%s` of `System/Title/%s.cmap` from a literal that
/// is `L"Title"` either way — so the cleared title is the same hit map under
/// different art. The pair is the base and the chip sheet, in that order.
const TITLE_CLEAR: (&str, &str) = (
    "System/Title/Clear/Title_Clear.png",
    "System/Title/Clear/Title_Clear_Chip.png",
);

/// The dress-select screen's background, which is not named after its stem and
/// is not art of its own.
///
/// `FUN_1000d980` hands `FUN_10010620` the pair
/// `(System/Screen/Transparence.png, System/DressSelect/DressSelect_Chip.png)`
/// — the full-screen transparent plate every install ships, because the moving
/// background is `STARTSCRIPT.INI`'s `[DressBG]` movie playing behind it. The
/// screen's own `DressSelect_Text.png` is drawn over that afterwards, by
/// `FUN_1000cce0`, and is a caption rather than the background — see
/// [`DRESS_SELECT_TEXT`].
const DRESS_SELECT_BASE: &str = "System/Screen/Transparence.png";

/// The dress-select screen's caption plate, drawn over its base art.
///
/// `FUN_1000d980` hands this literal to `FUN_1000cce0`, which builds it as a
/// full-screen sprite rather than a widget; `FUN_1000c740` draws it only while
/// the main hit map is loaded. See [`crate::ui::dress`].
const DRESS_SELECT_TEXT: &str = "System/DressSelect/DressSelect_Text.png";

/// The dress-select screen's confirm popup, which is not a mode of its own.
///
/// `MENU::DressSelect` swaps its one hit map between `FUN_1000d7f0`'s and this,
/// which `FUN_1000d8c0` loads — so the popup has no `SystemInit` code and
/// cannot be a [`crate::ui::menu::Mode`]. Its art is named after the stem like
/// any other screen's, so only the stem is needed here.
const DRESS_SELECT_POPUP: &str = "System/DressSelect/Popup/Popup_Select.cmap";

/// The backlog screen's two hit maps, horizontal first.
///
/// It has no `SystemInit` mode to key off — `setSystemInit` reaches it by a
/// code of its own — and `FILMENGINE.INI`'s `[BackLogType]` picks between two
/// whole screens rather than two layouts of one, so this is a pair rather than
/// a `%s`. `FUN_10003820` is where the module spells all eight of them, and
/// these two are the plain-resolution pair the rest are suffixes of. See
/// [`crate::ui::backlog`].
const BACKLOG: (&str, &str) = (
    "System/BackLog/BackLog_Horizon.cmap",
    "System/BackLog/BackLog_Vertical.cmap",
);

/// The two view backgrounds a one-map replay screen draws **inside** its frame.
///
/// The frame itself is named after the stem like every other screen's:
/// `FUN_10029550` hands `FUN_10010620` the pair
/// `(System/Replay/ReplayBase.png, System/Replay/ReplayBase_Chip.png)`. These
/// two are the page layer under it — `FUN_10029020` takes one of them on a view
/// change — and are what tells a one-map module apart from School Days HQ,
/// which has neither. The paths themselves belong to
/// [`crate::ui::replay_pages::base_art`], which records where each is spelled;
/// here they are only the signature.
const REPLAY_PAGE_BACKGROUNDS: [&str; 2] = [
    "System/Replay/Replay_HScene.png",
    "System/Replay/Replay_PlayData.png",
];

/// The screen paths one install's menu module holds.
#[derive(Debug, Clone, Default)]
pub struct Paths {
    /// Mode to the hit-map spelling the module holds for it, `.cmap` included.
    stems: Vec<(i32, &'static str)>,
    /// Set when the module keeps its cleared title under `Clear/`, sharing the
    /// plain title's hit map.
    title_clear: Option<(&'static str, &'static str)>,
    /// Set when the module's two replay views are a page layer inside one
    /// frame rather than two screens with a hit map each.
    replay_pages: bool,
    /// Set when the module has a dress-select screen, whose background is
    /// [`DRESS_SELECT_BASE`] rather than art named after its stem.
    dress_select_base: bool,
    /// Set when the module also holds that screen's caption plate.
    dress_select_text: bool,
    /// Set when the module also holds that screen's confirm popup, which has
    /// no mode of its own to key off.
    dress_select_popup: bool,
    /// Which of the backlog screen's two hit maps the module holds, horizontal
    /// first. Both, in every module seen so far.
    backlog: (bool, bool),
}

impl Paths {
    /// Reads the screen paths out of a menu module's own string literals.
    ///
    /// `dll` is the bytes of the user's menu module. An empty or unparseable
    /// one yields a [`Paths`] that knows no screens, which reports every mode
    /// unavailable rather than guessing — the same answer the rest of the
    /// install layer gives for a missing file.
    pub fn from_module(dll: &[u8]) -> Paths {
        // A file this engine cannot read as a PE image is not a menu module,
        // and is refused here rather than string-searched, so that a stray text
        // file holding one of these paths cannot be read as one.
        if Image::parse(dll).is_err() {
            return Paths::default();
        }
        Paths::from_literals(|text| holds_wide(dll, text))
    }

    /// The same, from any answer to "does this module hold that literal".
    fn from_literals(holds: impl Fn(&str) -> bool) -> Paths {
        let mut stems: Vec<(i32, &'static str)> = Vec::new();
        for (mode, literal) in CANDIDATES {
            if stems.iter().any(|(m, _)| m == mode) {
                continue;
            }
            if holds(literal) {
                stems.push((*mode, literal));
            }
        }
        Paths {
            stems,
            title_clear: holds(TITLE_CLEAR.0).then_some(TITLE_CLEAR),
            replay_pages: REPLAY_PAGE_BACKGROUNDS.iter().all(|path| holds(path)),
            dress_select_base: holds(DRESS_SELECT_BASE),
            dress_select_text: holds(DRESS_SELECT_TEXT),
            dress_select_popup: holds(DRESS_SELECT_POPUP),
            backlog: (holds(BACKLOG.0), holds(BACKLOG.1)),
        }
    }

    /// The path stem for one mode and variant, or `None` when this module has
    /// no screen for that mode.
    ///
    /// The stem is the module's own literal with its `.cmap` dropped and its
    /// placeholders filled: `%s` takes the variant whole, and the route map's
    /// `%02d` takes the episode, which is the variant up to its page suffix.
    pub fn stem(&self, mode: i32, variant: &str) -> Option<String> {
        let literal = self.literal(mode)?;
        let stem = literal.strip_suffix(".cmap").unwrap_or(literal);
        let episode = variant.split_once('-').map_or(variant, |(head, _)| head);
        Some(stem.replace("%02d", episode).replace("%s", variant))
    }

    /// Whether this module has a screen for `mode` at all.
    ///
    /// The two modules do not hold the same set. Asking this rather than which
    /// title an install is keeps the answer where every other screen path's
    /// comes from — the module's own literals — and it agrees with the module's
    /// own dispatch: `_SystemInit@8` bounds-checks the mode against 9 in
    /// `SysMenuSD.dll` and against 8 in `SysMenuSDHQ.dll`, which is mode 9,
    /// the dress-select screen, being one module's and not the other's.
    pub fn has_screen(&self, mode: i32) -> bool {
        self.literal(mode).is_some()
    }

    /// The variant to build the title's stem from, which is not always the
    /// variant its art is chosen by.
    ///
    /// A module that keeps its cleared title under `Clear/` gives it no hit map
    /// of its own, so every title state loads the same one and only the art
    /// changes. See [`Paths::title_art`].
    pub fn title_stem_variant<'a>(&self, variant: &'a str) -> &'a str {
        match self.title_clear {
            Some(_) if variant != "Title" => "Title",
            _ => variant,
        }
    }

    /// The title's base art and chip sheet when they are not named after the
    /// stem, or `None` when they are.
    pub fn title_art(&self, variant: &str) -> Option<(&'static str, &'static str)> {
        match self.title_clear {
            Some(art) if variant != "Title" => Some(art),
            _ => None,
        }
    }

    /// Whether the module's two replay views are a page layer inside one
    /// frame, which is what [`crate::ui::replay_pages`] draws.
    ///
    /// The other module gives each view a hit map of its own carrying all of
    /// that view's widgets, and [`crate::ui::replay`] reads them out of it.
    pub fn replay_has_pages(&self) -> bool {
        self.replay_pages
    }

    /// The dress-select screen's background, or `None` when this module has no
    /// such screen.
    pub fn dress_select_art(&self) -> Option<&'static str> {
        self.dress_select_base.then_some(DRESS_SELECT_BASE)
    }

    /// The dress-select screen's caption plate, or `None` when this module has
    /// no such screen.
    pub fn dress_select_text(&self) -> Option<&'static str> {
        self.dress_select_text.then_some(DRESS_SELECT_TEXT)
    }

    /// The stem of that screen's confirm popup, or `None` when this module has
    /// no such popup.
    ///
    /// The popup is a second hit map inside mode 9 rather than a mode of its
    /// own, so it is not in [`CANDIDATES`] and does not go through
    /// [`Paths::stem`].
    pub fn dress_select_popup(&self) -> Option<&'static str> {
        self.dress_select_popup
            .then(|| DRESS_SELECT_POPUP.strip_suffix(".cmap"))
            .flatten()
    }

    /// The stem of the backlog screen for one flow, or `None` when this module
    /// has no such screen.
    ///
    /// Like the dress-select popup, this is not in [`CANDIDATES`] and does not
    /// go through [`Paths::stem`]: the backlog has no `SystemInit` mode to be
    /// keyed by.
    pub fn backlog_stem(&self, flow: crate::ui::backlog::Flow) -> Option<&'static str> {
        let (held, literal) = match flow {
            crate::ui::backlog::Flow::Horizontal => (self.backlog.0, BACKLOG.0),
            crate::ui::backlog::Flow::Vertical => (self.backlog.1, BACKLOG.1),
        };
        held.then(|| literal.strip_suffix(".cmap")).flatten()
    }

    /// Whether the module's Option screen keeps each tab's widgets in a hit map
    /// of its own, which is what [`crate::ui::options`]'s widget numbering is.
    ///
    /// A module whose Option hit map is shared by every tab lays the tabs'
    /// contents out against rectangles in the module instead — see
    /// [`crate::ui::option_pages`], which is that half of the screen. The four
    /// widgets the shared map does have are the three tab headers and the close
    /// button, and those are the first four on the per-tab maps too, so they
    /// work either way.
    pub fn option_tabs_have_own_map(&self) -> bool {
        self.literal(4).is_some_and(|l| l.contains("%s"))
    }

    /// Which sound model this module's settings drive.
    ///
    /// The same shape of the Option hit map answers this, because it is the
    /// same fact: a module that lays its tabs out against rectangles of its own
    /// is the module whose Sound tab has continuous sliders, and a continuous
    /// slider carries a fraction. `FUN_1000bd20` commits one as the knob's
    /// position across its travel and `FUN_100075d0` reads it back through the
    /// settings object's `VT_R4` getter; the module with a hit map per tab
    /// steps through ten levels instead, clamped by `FUN_10007140` and read
    /// back through `VT_I4` in `FUN_10006ce0`.
    ///
    /// A module holding no Option screen at all gets [`Sound::Levels`], which
    /// is the model this engine ran on first; it has no Sound tab to disagree
    /// with.
    pub fn sound(&self) -> Sound {
        match self.option_tabs_have_own_map() {
            true => Sound::Levels,
            false => Sound::Fractions,
        }
    }

    fn literal(&self, mode: i32) -> Option<&'static str> {
        self.stems
            .iter()
            .find(|(m, _)| *m == mode)
            .map(|(_, literal)| *literal)
    }
}

/// Whether `dll` holds `text` as a UTF-16LE literal.
///
/// Both titles' menu modules keep every path as UTF-16, and the search is over
/// the whole file rather than a parsed `.rdata`, because the literals are the
/// same bytes wherever the section table puts them.
fn holds_wide(dll: &[u8], text: &str) -> bool {
    let wide: Vec<u8> = text.encode_utf16().flat_map(u16::to_le_bytes).collect();
    dll.windows(wide.len()).any(|w| w == wide)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A module that holds exactly these literals and nothing else.
    fn module(literals: &[&str]) -> Paths {
        Paths::from_literals(|text| literals.contains(&text))
    }

    /// The two spellings recovered so far, each resolving to the stems their
    /// own install ships. `daysengine menu --check-all` checks these against the real
    /// packs; this pins that neither spelling's substitution drifts.
    #[test]
    fn each_spelling_resolves_to_the_stems_its_own_install_ships() {
        let hq = module(&[
            "System/Title/%s.cmap",
            "System/SaveLoad/SaveLoad.cmap",
            "System/Option/Option_%s.cmap",
            "System/Replay/Replay_%s.cmap",
            "System/RouteMap/%02d/RouteMap%s.cmap",
            "System/Option/Pop_Som.cmap",
            "System/Replay/Pop_Replay_%s.cmap",
            "System/Exit/Popup.cmap",
        ]);
        for (mode, variant, stem) in [
            (2, "Title", "System/Title/Title"),
            (4, "Def", "System/Option/Option_Def"),
            (5, "HScene", "System/Replay/Replay_HScene"),
            (6, "01", "System/RouteMap/01/RouteMap01"),
            // Episode 3's later pages carry the page in the variant and only
            // the episode in the directory.
            (6, "03-2", "System/RouteMap/03/RouteMap03-2"),
            (7, "", "System/Option/Pop_Som"),
            (8, "2", "System/Replay/Pop_Replay_2"),
        ] {
            assert_eq!(hq.stem(mode, variant).as_deref(), Some(stem));
        }
        assert_eq!(
            hq.stem(9, ""),
            None,
            "no dress-select screen in that module"
        );
        // The popup is keyed off its own literal rather than a mode, because
        // it is a second hit map inside mode 9 — so a module without the
        // screen must not report one either.
        assert_eq!(hq.dress_select_popup(), None);
        assert_eq!(hq.dress_select_text(), None);

        let sd = module(&[
            "System/Title/%s.cmap",
            "System/Option/OptionBase.cmap",
            "System/Replay/ReplayBase.cmap",
            "System/RouteMap/RouteMap.cmap",
            "System/Option/Popup/Pop_Som.cmap",
            "System/Replay/Popup/Pop_Replay_%s.cmap",
            "System/DressSelect/DressSelect.cmap",
        ]);
        for (mode, variant, stem) in [
            (4, "Def", "System/Option/OptionBase"),
            (5, "HScene", "System/Replay/ReplayBase"),
            (6, "01", "System/RouteMap/RouteMap"),
            (7, "", "System/Option/Popup/Pop_Som"),
            (8, "4", "System/Replay/Popup/Pop_Replay_4"),
            (9, "", "System/DressSelect/DressSelect"),
        ] {
            assert_eq!(sd.stem(mode, variant).as_deref(), Some(stem));
        }
        // The popup's stem carries no variant placeholder, so it is the same
        // string whatever the display mode — the four spellings are the
        // `_Wide*` suffixes `Screen::load_with_art` appends, not stems.
        let sd_popup = module(&[
            "System/DressSelect/DressSelect.cmap",
            "System/DressSelect/Popup/Popup_Select.cmap",
        ]);
        assert_eq!(
            sd_popup.dress_select_popup(),
            Some("System/DressSelect/Popup/Popup_Select")
        );
        // That module ships no save/load or confirm literal in this fixture,
        // so those modes have no screen — the answer for a mode a module really
        // does not have, rather than a path nothing resolves.
        assert_eq!(sd.stem(3, ""), None);
    }

    /// A module whose cleared title lives under `Clear/` keeps it on the plain
    /// title's hit map, so the stem must not move with the art. Getting this
    /// the other way round asks for a `.cmap` that does not ship and loses the
    /// title screen to every player who has cleared the game once.
    #[test]
    fn a_cleared_title_under_clear_shares_the_plain_titles_map() {
        let sd = module(&[
            "System/Title/%s.cmap",
            "System/Title/Clear/Title_Clear.png",
            "System/Title/Clear/Title_Clear_Chip.png",
        ]);
        assert_eq!(sd.title_stem_variant("Title_Clear"), "Title");
        assert_eq!(
            sd.stem(2, sd.title_stem_variant("Title_Clear")).as_deref(),
            Some("System/Title/Title")
        );
        assert_eq!(sd.title_art("Title_Clear"), Some(TITLE_CLEAR));
        assert_eq!(sd.title_art("Title"), None);

        let hq = module(&["System/Title/%s.cmap"]);
        assert_eq!(hq.title_stem_variant("Title_Clear"), "Title_Clear");
        assert_eq!(hq.title_art("Title_Clear"), None);
    }

    /// A module with no menu module at all — the engine tolerates that — knows
    /// no screens rather than naming every one of them.
    #[test]
    fn no_module_means_no_screens() {
        let none = Paths::from_module(&[]);
        for mode in [-1, 2, 3, 4, 5, 6, 7, 8, 9] {
            assert_eq!(none.stem(mode, "Title"), None, "mode {mode}");
        }
    }
}
