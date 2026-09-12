//! The endings the player has seen, and the picture behind the title.
//!
//! The title screen is not drawn over one fixed backdrop. `Title.png` is
//! transparent around the logo and the engine chooses what goes underneath: a
//! fresh install gets `TitleBase.png`, a player part-way through gets the title
//! card of the **most recent ending they reached**, and a player who has seen
//! every ending gets a card of their own.
//!
//! # Where the cards come from
//!
//! `FUN_0041fc40` in `SCHOOLDAYS HQ.exe` loads the list. The file it loads is
//! named by `[EndingList]=`, which is not in any shipped `.INI` — it is in the
//! master configuration compiled into the executable, alongside the names of
//! every other INI the game reads:
//!
//! ```text
//! [DXGraphicBase]="Ini/DX9Graphic.ini"    [StartScript]="Ini/StartScript.ini"
//! [DXSoundBase]="Ini/DX8Sound.ini"        [DebugInfo]="Ini/DebugInfo.ini"
//! [FILMEngine]="Ini/FILMEngine.ini"       [EndingList]="Ini/EndList.ini"
//! ```
//!
//! That file is `ENDLIST.INI` in `Ini.GPK` — pack lookups are case-insensitive,
//! so the two spellings are the same file. It is plain ASCII, unlike
//! `STARTSCRIPT.INI`, which ships as UTF-8 with a BOM:
//!
//! ```text
//! [EndingMax]="22"
//! [Ending01]="System/EndTitle/END-05-5H-E00.png"
//! ...
//! [Ending22]="System/EndTitle/END-SETSUNA.png"
//! [AllClear]="System/EndTitle/END-ALL-complete.png"
//! ```
//!
//! The keys are 1-based but `FUN_0041fc40` pushes them into a vector in order
//! and `FUN_00420310` indexes it 0-based, so **slot `n` is `[Ending(n+1)]`**.
//! Two of the 22 cards are `.wmv` rather than `.png`, so the backdrop can be a
//! movie — see [`Backdrop::is_movie`].
//!
//! # Which one
//!
//! [`title_backdrop`] is `FUN_0041fee0`, branch for branch. Note the third
//! branch: reaching the last ending is what *creates* the stored `AllClear`
//! flag, which is why the first branch can rely on it.

use crate::ini::Ini;
use crate::vfs::Vfs;
use days_save::{FlagStore, Value};

/// `Ini/EndList.ini`, parsed: the per-ending title cards.
#[derive(Debug, Default, Clone)]
pub struct EndingList {
    /// `[EndingMax]`. The number of endings, and the number of `[EndNN]` flags.
    max: usize,
    /// `[Ending01]` upwards, 0-based: slot `n` holds `[Ending(n+1)]`.
    cards: Vec<String>,
    /// `[AllClear]`, the card for having seen every ending.
    all_clear: Option<String>,
}

impl EndingList {
    /// The path the executable's built-in master INI names, `[EndingList]=`.
    pub const PATH: &'static str = "Ini/EndList.ini";

    /// Parses the list, following the executable's own loop.
    ///
    /// `[EndingMax]` sets the count and the loop reads exactly that many keys,
    /// so a list that names more cards than it declares keeps only the declared
    /// ones, and a missing key leaves an empty slot rather than shortening the
    /// vector — the executable pushes the empty string `FUN_0046e2c0` returns
    /// when it cannot find a key.
    pub fn parse(ini: &Ini) -> EndingList {
        let max = ini.get_u32("EndingMax").unwrap_or(0) as usize;
        let cards = (1..=max)
            .map(|n| ini.get(&format!("Ending{n:02}")).unwrap_or("").to_string())
            .collect();
        EndingList {
            max,
            cards,
            all_clear: ini.get("AllClear").map(str::to_string),
        }
    }

    /// How many endings the list declares.
    pub fn max(&self) -> usize {
        self.max
    }

    /// The card for ending `index`, 0-based. `None` past the end of the list,
    /// where the executable would abort out of its bounds check.
    pub fn card(&self, index: usize) -> Option<&str> {
        self.cards.get(index).map(String::as_str).filter(non_empty)
    }

    /// The card for having seen every ending.
    pub fn all_clear_card(&self) -> Option<&str> {
        self.all_clear.as_deref().filter(non_empty)
    }

    /// How many endings the save says the player has seen.
    ///
    /// `FUN_00420040`: one flag per ending, named for the INI key syntax it was
    /// formatted with — `[End00]="` through `[End21]="`, punctuation and all,
    /// 0-based, counted up to `[EndingMax]` and no further.
    pub fn seen(&self, flags: &FlagStore) -> usize {
        (0..self.max)
            .filter(|&n| flags.flag(&Self::flag_name(n)))
            .count()
    }

    /// The save-flag name for ending `index`, as the executable formats it.
    pub fn flag_name(index: usize) -> String {
        format!("[End{index:02}]=\"")
    }
}

/// The picture that goes behind the title, and why that one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Backdrop {
    /// The asset path, as the INI spells it.
    pub path: String,
    pub reason: Reason,
    /// The executable sets the stored `AllClear` flag as it takes
    /// [`Reason::EveryEndingSeen`]. This engine does not write save data yet,
    /// so it recomputes the answer each time instead — which gives the same
    /// picture, but leaves the player's file untouched.
    pub sets_all_clear: bool,
}

impl Backdrop {
    /// Whether the card is a movie rather than a still.
    ///
    /// Two of the 22 shipped cards are `.wmv`. Whether the original plays them
    /// or freezes them is **not recovered**: `STARTSCRIPT.INI [EndBGView]="1"`
    /// is the only plausible switch and nothing was found reading it.
    pub fn is_movie(&self) -> bool {
        is_movie(&self.path)
    }
}

/// Why a particular backdrop was chosen — the four branches of `FUN_0041fee0`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// The stored `AllClear` flag is set.
    AllClearFlag,
    /// `EndClear` is clear: nothing finished yet, so the fresh-install picture.
    Fresh,
    /// Every `[EndNN]` flag is set. The executable stores `AllClear` here.
    EveryEndingSeen,
    /// The most recent ending, `EndNo`, which is an **index** and not a tally.
    MostRecentEnding(usize),
    /// The chosen card names nothing this install ships, so the fresh-install
    /// picture stands in. Not a branch of the original — it is what this engine
    /// does rather than leave the title with no backdrop at all.
    CardMissing(Reason2),
}

/// The branch a [`Reason::CardMissing`] fell out of.
///
/// A separate type only because [`Reason`] cannot contain itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason2 {
    AllClearFlag,
    EveryEndingSeen,
    MostRecentEnding(usize),
}

/// Chooses the title backdrop, following `FUN_0041fee0`.
///
/// The order is the executable's:
///
/// 1. `AllClear` set — the `[AllClear]` card.
/// 2. `EndClear` clear — `STARTSCRIPT.INI [BaseFile]`, the fresh-install
///    picture. Nothing has been finished, so there is no ending to show.
/// 3. every `[EndNN]` flag set — the `[AllClear]` card *and* store the
///    `AllClear` flag, which is how branch 1 ever becomes reachable.
/// 4. otherwise — the card for `EndNo`, the most recent ending.
///
/// `base_file` is `[BaseFile]`, and is also the answer whenever the chosen card
/// turns out to be missing from the list.
pub fn title_backdrop(list: &EndingList, flags: &FlagStore, base_file: &str) -> Backdrop {
    let fresh = |reason| Backdrop {
        path: base_file.to_string(),
        reason,
        sets_all_clear: false,
    };
    let card = |path: Option<&str>, reason: Reason, fallback: Reason2, sets: bool| match path {
        Some(path) => Backdrop {
            path: path.to_string(),
            reason,
            sets_all_clear: sets,
        },
        None => fresh(Reason::CardMissing(fallback)),
    };

    if flags.flag("AllClear") {
        return card(
            list.all_clear_card(),
            Reason::AllClearFlag,
            Reason2::AllClearFlag,
            false,
        );
    }
    if !flags.flag("EndClear") {
        return fresh(Reason::Fresh);
    }
    if list.max() > 0 && list.seen(flags) == list.max() {
        return card(
            list.all_clear_card(),
            Reason::EveryEndingSeen,
            Reason2::EveryEndingSeen,
            true,
        );
    }
    // `EndNo` is VT_I4 and an index into the list. A save with no `EndNo` at
    // all cannot have got here — `EndClear` is set — but if one did, the
    // executable would index slot 0, because its getter returns 0 for a name it
    // does not find.
    let index = flags
        .get("EndNo")
        .and_then(Value::as_int)
        .unwrap_or(0)
        .max(0) as usize;
    card(
        list.card(index),
        Reason::MostRecentEnding(index),
        Reason2::MostRecentEnding(index),
        false,
    )
}

/// What went wrong reading a backdrop.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Vfs(#[from] crate::vfs::Error),
    #[error(transparent)]
    Image(#[from] days_ui::Error),
    #[error(transparent)]
    Media(#[from] crate::media::Error),
    #[error("the clip decoded no frames")]
    EmptyClip,
}

/// Reads the ending list out of the packs, if `[EndBGView]` asks for it.
///
/// `FUN_0041f600` only calls the loader when that key is set, so a clear key
/// means the game holds no cards at all and the title keeps the fresh-install
/// picture however far the player has got. The same key is the extra condition
/// on route 0 — see [`crate::menu::end_bg_view`].
///
/// A list that will not load is likewise an empty list rather than an error:
/// it costs the player the ending backdrops and nothing else, which is the
/// bargain every other missing asset gets.
pub fn load_list(vfs: &Vfs, start: &Ini) -> EndingList {
    if !crate::menu::end_bg_view(start) {
        log::info!("[EndBGView] is clear: no ending backdrops");
        return EndingList::default();
    }
    match vfs.read_path(EndingList::PATH) {
        Ok(bytes) => EndingList::parse(&Ini::parse_bytes(&bytes)),
        Err(err) => {
            log::warn!("reading {}: {err}", EndingList::PATH);
            EndingList::default()
        }
    }
}

/// Whether a card is a movie rather than a still.
pub fn is_movie(path: &str) -> bool {
    path.to_ascii_lowercase().ends_with(".wmv")
}

/// Reads a backdrop's picture, still or movie.
///
/// Two of the 22 ending cards are `.wmv`. This engine shows their **first
/// frame**: whether the original animates them is not recovered, and a still
/// frame of the right card beats the wrong picture.
pub fn load_image(vfs: &Vfs, path: &str) -> Result<days_ui::Image, Error> {
    let bytes = vfs.read_path(path)?;
    if !is_movie(path) {
        return Ok(days_ui::Image::decode_png(&bytes)?);
    }
    let frame = crate::media::VideoDecoder::open(bytes)?
        .next_frame()?
        .ok_or(Error::EmptyClip)?;
    Ok(days_ui::Image {
        width: frame.width,
        height: frame.height,
        rgba: frame.rgba,
    })
}

fn non_empty(s: &&str) -> bool {
    !s.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use days_save::Value;

    /// Verbatim from the retail `ENDLIST.INI`, abridged in the middle.
    const ENDLIST: &str = r#"[EndingMax]="4"

[Ending01]="System/EndTitle/END-05-5H-E00.png"
[Ending02]="System/EndTitle/END-05-5H-E01.png"
[Ending03]="System/EndTitle/END-05-SB-E01.wmv"
[Ending04]="System/EndTitle/END-SETSUNA.png"

[AllClear]="System/EndTitle/END-ALL-complete.png"
"#;

    const BASE: &str = "System/Title/TitleBase.png";

    fn list() -> EndingList {
        EndingList::parse(&Ini::parse(ENDLIST))
    }

    fn store(entries: &[(&str, Value)]) -> FlagStore {
        FlagStore::from_entries(entries.iter().map(|(k, v)| ((*k).to_string(), v.clone())))
    }

    fn set(names: &[&str]) -> FlagStore {
        store(
            &names
                .iter()
                .map(|n| (*n, Value::Bool(true)))
                .collect::<Vec<_>>(),
        )
    }

    /// The 1-based keys land in 0-based slots.
    #[test]
    fn slot_n_holds_ending_n_plus_one() {
        let list = list();
        assert_eq!(list.max(), 4);
        assert_eq!(list.card(0), Some("System/EndTitle/END-05-5H-E00.png"));
        assert_eq!(list.card(3), Some("System/EndTitle/END-SETSUNA.png"));
        assert_eq!(list.card(4), None, "past the end, where the game aborts");
        assert_eq!(
            list.all_clear_card(),
            Some("System/EndTitle/END-ALL-complete.png")
        );
    }

    /// `[EndingMax]` sets the count, not the number of keys present.
    #[test]
    fn the_declared_count_wins() {
        let list = EndingList::parse(&Ini::parse(
            "[EndingMax]=\"2\"\n[Ending01]=\"a.png\"\n[Ending02]=\"b.png\"\n[Ending03]=\"c.png\"",
        ));
        assert_eq!(list.max(), 2);
        assert_eq!(list.card(2), None);
    }

    /// A list that fails to load must not take the title down with it.
    #[test]
    fn an_absent_list_falls_back_to_the_fresh_picture() {
        let list = EndingList::default();
        let chosen = title_backdrop(&list, &set(&["AllClear", "EndClear"]), BASE);
        assert_eq!(chosen.path, BASE);
        assert_eq!(chosen.reason, Reason::CardMissing(Reason2::AllClearFlag));
    }

    #[test]
    fn a_fresh_save_gets_the_fresh_picture() {
        let chosen = title_backdrop(&list(), &FlagStore::default(), BASE);
        assert_eq!(chosen.path, BASE);
        assert_eq!(chosen.reason, Reason::Fresh);
        assert!(!chosen.is_movie());
    }

    #[test]
    fn a_stored_all_clear_gets_the_all_clear_card() {
        let chosen = title_backdrop(&list(), &set(&["AllClear"]), BASE);
        assert_eq!(chosen.path, "System/EndTitle/END-ALL-complete.png");
        assert_eq!(chosen.reason, Reason::AllClearFlag);
        assert!(!chosen.sets_all_clear, "it is already stored");
    }

    /// Branch 3: every flag set but `AllClear` not yet stored. This is the one
    /// branch that writes.
    #[test]
    fn the_last_ending_earns_the_all_clear_card_and_stores_the_flag() {
        let seen = set(&[
            "EndClear",
            r#"[End00]=""#,
            r#"[End01]=""#,
            r#"[End02]=""#,
            r#"[End03]=""#,
        ]);
        let chosen = title_backdrop(&list(), &seen, BASE);
        assert_eq!(chosen.path, "System/EndTitle/END-ALL-complete.png");
        assert_eq!(chosen.reason, Reason::EveryEndingSeen);
        assert!(chosen.sets_all_clear);
    }

    /// Branch 4: `EndNo` is an index, so 2 is the *third* ending.
    #[test]
    fn part_way_through_gets_the_most_recent_endings_card() {
        let entries = [
            ("EndClear", Value::Bool(true)),
            (r#"[End00]=""#, Value::Bool(true)),
            (r#"[End02]=""#, Value::Bool(true)),
            ("EndNo", Value::Int(2)),
        ];
        let chosen = title_backdrop(&list(), &store(&entries), BASE);
        assert_eq!(chosen.path, "System/EndTitle/END-05-SB-E01.wmv");
        assert_eq!(chosen.reason, Reason::MostRecentEnding(2));
        assert!(chosen.is_movie(), "two of the shipped cards are movies");
    }

    /// An `EndNo` past the end of the list would abort the original. Here it
    /// costs the player the picture and nothing else.
    #[test]
    fn an_out_of_range_end_no_falls_back() {
        let entries = [("EndClear", Value::Bool(true)), ("EndNo", Value::Int(99))];
        let chosen = title_backdrop(&list(), &store(&entries), BASE);
        assert_eq!(chosen.path, BASE);
        assert_eq!(
            chosen.reason,
            Reason::CardMissing(Reason2::MostRecentEnding(99))
        );
    }

    #[test]
    fn the_flag_names_carry_the_ini_punctuation() {
        assert_eq!(EndingList::flag_name(0), r#"[End00]=""#);
        assert_eq!(EndingList::flag_name(21), r#"[End21]=""#);
    }
}
