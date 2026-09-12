//! The save/load screen — mode 3, `System/SaveLoad`.
//!
//! One screen serves both jobs. Which one it is doing is a member the DLL
//! keeps at `+0x94`, and everything else follows from it: the base art is
//! `Load.png` or `Save.png`, picking a row either loads that slot or writes
//! one, and the route-map button is live on the Load screen only.
//!
//! # The list
//!
//! Ten slots to a page, ten pages, so a hundred slots. The widget table has
//! two bands of ten for the rows — the timestamp side and the comment side —
//! and both do the same thing:
//!
//! ```text
//! 0x00 .. 0x09   the ten rows
//! 0x0a .. 0x13   the ten page buttons
//! 0x14           leave
//! 0x15           the route map, Load screen only
//! 0x16 .. 0x1f   the ten rows again, the other band
//! ```
//!
//! `FUN_10014990` is that dispatch and [`action`] is its transcription. The
//! slot a row stands for is `page * 10 + row` (`FUN_10011d50`), which is why
//! the shipped `[SaveFileName]="Save/SaveFile00%d.DAT"` puts slot 14 in
//! `SaveFile0014.DAT` rather than in a padded name.
//!
//! Every widget is live unless the confirm popup is up (`FUN_10014910`): the
//! screen does not grey out an empty slot, it just does nothing when the row
//! it names has no file.
//!
//! # The display line
//!
//! A slot's file says nothing about itself. The line the screen shows lives in
//! the **global** store, under the key `[SaveConfig]="FILMEngine/SaveFile00%d"`
//! formats, with the player's comment under the same name plus `_Sub`.
//!
//! `FUN_10011b40` builds it from the clock and the chapter number, as one
//! string with the chapter appended:
//!
//! ```text
//! Japanese   "%4d年%2d月%2d日(%s)%02d:%02d"  +  "第%d話"
//! English    "%2d/%2d/%4d(%s)%02d:%02d"      +  "%02d"
//! ```
//!
//! The reader splits the chapter back off by character count — three for
//! Japanese, two for English (`FUN_0042a980`) — which is exactly the length
//! each of those tails has, and is why the split is a constant rather than a
//! search.
//!
//! # What is not recovered
//!
//! **Where a row's text sits.** `FUN_10011ec0` does not draw the three columns
//! onto the screen: it renders them into an off-screen surface — the object at
//! `+0x110`, its pixels at `+0x114` and its pitch at `+0x118` — putting the
//! timestamp at `(0, row * 0x30 + 2)`, the chapter at `(0x400, row * 0x30 + 2)`
//! and the comment at `(0, row * 0x30 + 0x202)`, and then blits it. Which
//! rectangles that surface is blitted through has not been worked out, so a
//! composited screen here draws the rows empty rather than putting the text
//! somewhere this engine chose. [`Slots`] has the lines; `days menu` prints
//! them.
//!
//! # What is not implemented
//!
//! The popup that asks the player to confirm overwriting a slot, and the
//! comment editor behind host `+0xdc`, which is a text field with an IME
//! attached. Saving here writes the timestamp line and keeps whatever comment
//! the slot already had. Keyboard navigation through the list
//! (`FUN_10014c90`) is a transition table that is **not recovered**; the
//! pointer works, and the arrow keys fall back to the generic order.

use crate::install::clock::Civil;
use crate::install::ini::Ini;
use crate::install::save;
use days_save::{FlagStore, Value};
use std::collections::BTreeMap;
use std::path::Path;

/// Which job the screen is doing, the DLL's `+0x94`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// `+0x94 == 0`: `Load.png`, and picking a row loads it.
    Load,
    /// Anything else: `Save.png`, and picking a row writes it.
    Save,
}

impl Kind {
    /// The base art the screen loads, from the DLL's own `+0x94` test.
    pub fn base(self) -> &'static str {
        match self {
            Kind::Load => "Load.png",
            Kind::Save => "Save.png",
        }
    }
}

/// Slots to a page, and pages. Ten rows are drawn per page and there are ten
/// page buttons, both from `FUN_10011ec0`'s loop and the widget bands.
pub const PER_PAGE: usize = 10;
pub const PAGES: usize = 10;

/// What activating a widget asks for, from `FUN_10014990`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Act {
    /// The widget is not one the screen answers.
    None,
    /// Use this row of the page in whichever way the screen is for.
    Row(usize),
    /// Show this page of ten slots.
    Page(usize),
    /// Leave the screen: host `+0x4c(0)`.
    Leave,
    /// Open the route map: host `+0x4c(6)`. Only on the Load screen.
    RouteMap,
}

/// The screen's dispatch, from `FUN_10014990`.
pub fn action(kind: Kind, widget: usize) -> Act {
    match widget {
        0..PER_PAGE => Act::Row(widget),
        0x0a..0x14 => Act::Page(widget - 0x0a),
        0x14 => Act::Leave,
        // Guarded by `+0x94 == 0` in the DLL, so the Save screen's copy of
        // this widget does nothing at all.
        0x15 if kind == Kind::Load => Act::RouteMap,
        0x16..0x20 => Act::Row(widget - 0x16),
        _ => Act::None,
    }
}

/// Whether a widget can be chosen, from `FUN_10014910`.
///
/// Every widget the screen has, until the confirm popup goes up and takes them
/// all at once. An empty slot is still live — picking it is what does nothing,
/// not the pointing.
pub fn enabled(popup_up: bool, widget: usize) -> bool {
    !popup_up && widget < 0x20
}

/// The slot a row of a page stands for, from `FUN_10011d50`.
pub fn slot_of(page: usize, row: usize) -> u32 {
    (page * PER_PAGE + row) as u32
}

/// The weekday names the timestamp uses.
///
/// Two tables of seven in the DLL, indexed by `SYSTEMTIME.wDayOfWeek`, which
/// counts from Sunday.
const WEEKDAYS_JP: [&str; 7] = ["日", "月", "火", "水", "木", "金", "土"];
const WEEKDAYS_EN: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

/// The two halves of a slot's display line, from `FUN_10011b40`.
///
/// Returned separately because the game stores them joined and splits them
/// back apart by length; `story` is the chapter number `_GetStory@4` reports.
pub fn display_line(now: Civil, story: u32, english: bool) -> (String, String) {
    let weekday = |names: [&'static str; 7]| names[(now.weekday as usize).min(6)];
    if english {
        (
            format!(
                "{:2}/{:2}/{:4}({}){:02}:{:02}",
                now.month,
                now.day,
                now.year,
                weekday(WEEKDAYS_EN),
                now.hour,
                now.minute
            ),
            format!("{story:02}"),
        )
    } else {
        (
            format!(
                "{:4}年{:2}月{:2}日({}){:02}:{:02}",
                now.year,
                now.month,
                now.day,
                weekday(WEEKDAYS_JP),
                now.hour,
                now.minute
            ),
            format!("第{story}話"),
        )
    }
}

/// How many characters of a stored line are the chapter, from `FUN_0042a980`.
///
/// A constant, not a search: the Japanese tail is always `第N話` and the
/// English one always two digits.
pub fn tail_len(english: bool) -> usize {
    if english {
        2
    } else {
        3
    }
}

/// Splits a stored display line back into its timestamp and its chapter.
///
/// A line too short to hold a chapter comes back whole, with no tail, rather
/// than being cut into nonsense — the shipped reader only splits when the
/// string is longer than one character.
pub fn split_line(line: &str, english: bool) -> (String, String) {
    let chars: Vec<char> = line.chars().collect();
    let tail = tail_len(english);
    if chars.len() <= 1 || chars.len() < tail {
        return (line.to_owned(), String::new());
    }
    let at = chars.len() - tail;
    (chars[..at].iter().collect(), chars[at..].iter().collect())
}

/// What the screen shows for one slot.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Line {
    /// The timestamp half of the stored line.
    pub when: String,
    /// The chapter half, `第N話` or two digits.
    pub chapter: String,
    /// The player's own comment, from the `_Sub` key.
    pub comment: String,
}

/// What the save/load screen knows about the slots, as host `+0x9c` answers.
///
/// `FUN_0042a980` reports a slot as present only when **the file opens** — it
/// tests that and nothing else, then fills the three strings from the global
/// store. So a slot whose file is gone shows as empty even with its line still
/// in the store, and a slot whose line is missing shows as present with
/// nothing written on it.
#[derive(Debug, Clone, Default)]
pub struct Slots {
    rows: BTreeMap<u32, Line>,
}

impl Slots {
    /// Reads every slot the screen can show.
    pub fn read(game: &Path, film: &Ini, flags: &FlagStore, english: bool) -> Slots {
        let mut rows = BTreeMap::new();
        for slot in 0..(PAGES * PER_PAGE) as u32 {
            if !save::slot_path(game, film, slot).is_file() {
                continue;
            }
            let (key, sub) = save::slot_keys(film, slot);
            let stored = flags.get(&key).and_then(Value::as_str).unwrap_or_default();
            let (when, chapter) = split_line(stored, english);
            rows.insert(
                slot,
                Line {
                    when,
                    chapter,
                    comment: flags
                        .get(&sub)
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                },
            );
        }
        Slots { rows }
    }

    /// Whether a slot has a file, which is the only thing the screen tests.
    pub fn filled(&self, slot: u32) -> bool {
        self.rows.contains_key(&slot)
    }

    /// The line a slot shows, if it has one.
    pub fn get(&self, slot: u32) -> Option<&Line> {
        self.rows.get(&slot)
    }

    /// How many slots have a file.
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Records a slot the player has just written, so the screen shows it
    /// without re-reading the install.
    pub fn insert(&mut self, slot: u32, line: Line) {
        self.rows.insert(slot, line);
    }

    /// The ten rows of a page, with the slot each stands for.
    pub fn page(&self, page: usize) -> Vec<(u32, Option<&Line>)> {
        (0..PER_PAGE)
            .map(|row| {
                let slot = slot_of(page, row);
                (slot, self.get(slot))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(year: i32, month: u32, day: u32, weekday: u32, hour: u32, minute: u32) -> Civil {
        Civil {
            year,
            month,
            day,
            weekday,
            hour,
            minute,
            second: 0,
        }
    }

    #[test]
    fn the_two_row_bands_pick_the_same_row() {
        for row in 0..PER_PAGE {
            assert_eq!(action(Kind::Load, row), Act::Row(row));
            assert_eq!(action(Kind::Load, row + 0x16), Act::Row(row));
        }
    }

    #[test]
    fn the_page_buttons_and_the_exits() {
        assert_eq!(action(Kind::Load, 0x0a), Act::Page(0));
        assert_eq!(action(Kind::Load, 0x13), Act::Page(9));
        assert_eq!(action(Kind::Save, 0x14), Act::Leave);
    }

    /// The route map is guarded by `+0x94 == 0`, so the Save screen's copy of
    /// the widget answers nothing.
    #[test]
    fn the_route_map_is_the_load_screens_alone() {
        assert_eq!(action(Kind::Load, 0x15), Act::RouteMap);
        assert_eq!(action(Kind::Save, 0x15), Act::None);
    }

    #[test]
    fn a_widget_past_the_table_asks_for_nothing() {
        assert_eq!(action(Kind::Load, 0x20), Act::None);
        assert!(!enabled(false, 0x20));
    }

    #[test]
    fn the_popup_takes_every_widget_at_once() {
        assert!(enabled(false, 0));
        assert!(enabled(false, 0x15));
        assert!(!enabled(true, 0));
    }

    #[test]
    fn a_row_names_a_slot_the_way_the_file_name_is_numbered() {
        assert_eq!(slot_of(0, 0), 0);
        assert_eq!(slot_of(1, 4), 14);
        assert_eq!(slot_of(9, 9), 99);
    }

    /// The line in the player's own `GlobalFlag.DAT` for slot 0 reads
    /// `2012年 1月28日(土)18:02第6話` — the space before the `1` is `%2d`
    /// padding, which is why this is a format and not a join.
    #[test]
    fn builds_the_line_a_real_save_carries() {
        let (head, tail) = display_line(at(2012, 1, 28, 6, 18, 2), 6, false);
        assert_eq!(head, "2012年 1月28日(土)18:02");
        assert_eq!(tail, "第6話");
        assert_eq!(format!("{head}{tail}"), "2012年 1月28日(土)18:02第6話");
    }

    #[test]
    fn the_english_line_is_the_other_order_and_a_two_digit_chapter() {
        let (head, tail) = display_line(at(2012, 1, 28, 6, 18, 2), 6, true);
        assert_eq!(head, " 1/28/2012(Sat)18:02");
        assert_eq!(tail, "06");
    }

    #[test]
    fn a_line_splits_back_into_what_built_it() {
        for english in [false, true] {
            let (head, tail) = display_line(at(2012, 1, 28, 6, 18, 2), 6, english);
            let joined = format!("{head}{tail}");
            assert_eq!(split_line(&joined, english), (head, tail));
        }
    }

    #[test]
    fn a_line_too_short_to_hold_a_chapter_is_left_alone() {
        assert_eq!(split_line("", false), (String::new(), String::new()));
        assert_eq!(split_line("x", false), ("x".to_owned(), String::new()));
    }
}
