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
/// `FUN_1000cce0`, and is a caption rather than the background; this engine
/// does not draw it yet.
const DRESS_SELECT_BASE: &str = "System/Screen/Transparence.png";

/// The two backgrounds a one-map replay screen draws, keyed by the variant the
/// screen was opened with. `FUN_100295e0` loads one map for both.
const REPLAY_BASE: [(&str, &str); 2] = [
    ("HScene", "System/Replay/Replay_HScene.png"),
    ("PlayData", "System/Replay/Replay_PlayData.png"),
];

/// The screen paths one install's menu module holds.
#[derive(Debug, Clone, Default)]
pub struct Paths {
    /// Mode to the hit-map spelling the module holds for it, `.cmap` included.
    stems: Vec<(i32, &'static str)>,
    /// Set when the module keeps its cleared title under `Clear/`, sharing the
    /// plain title's hit map.
    title_clear: Option<(&'static str, &'static str)>,
    /// Set when the module names its replay backgrounds separately from the
    /// one hit map they share.
    replay_base: bool,
    /// Set when the module has a dress-select screen, whose background is
    /// [`DRESS_SELECT_BASE`] rather than art named after its stem.
    dress_select_base: bool,
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
            replay_base: REPLAY_BASE.iter().all(|(_, path)| holds(path)),
            dress_select_base: holds(DRESS_SELECT_BASE),
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

    /// The replay screen's base art when the module names it apart from the
    /// one hit map its two views share.
    pub fn replay_art(&self, variant: &str) -> Option<&'static str> {
        if !self.replay_base {
            return None;
        }
        REPLAY_BASE
            .iter()
            .find(|(view, _)| *view == variant)
            .map(|(_, path)| *path)
    }

    /// The dress-select screen's background, or `None` when this module has no
    /// such screen.
    pub fn dress_select_art(&self) -> Option<&'static str> {
        self.dress_select_base.then_some(DRESS_SELECT_BASE)
    }

    /// Whether the module's Option screen keeps each tab's widgets in a hit map
    /// of its own, which is what [`crate::ui::options`]'s widget numbering is.
    ///
    /// A module whose Option hit map is shared by every tab lays the tabs'
    /// contents out at runtime instead, and what those rows are is **not
    /// recovered** — see `docs/FORMATS.md`. The four widgets its map does have
    /// are the three tab headers and the close button, which are the first four
    /// on the per-tab maps too, so those work either way.
    pub fn option_tabs_have_own_map(&self) -> bool {
        self.literal(4).is_some_and(|l| l.contains("%s"))
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
    /// own install ships. `days menu --check-all` checks these against the real
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
