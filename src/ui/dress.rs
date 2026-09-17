//! The clothing-selection screen, which only one of the two titles has.
//!
//! `_SystemInit@8` in `SysMenuSD.dll` has a case for mode 9 that hands over the
//! static object at `DAT_1005b898`; its static-init thunk `FUN_10048860` calls
//! the constructor `FUN_1000c470`, which installs `MENU::DressSelect::vftable`
//! at `0x1004a6ec`. `SysMenuSDHQ.dll` has no case 9 at all, so on that module
//! [`crate::ui::paths::Paths::stem`] finds no screen and the mode is
//! unavailable — see [`crate::ui::menu::Mode::DRESS_SELECT`].
//!
//! # One object, two hit maps
//!
//! The screen is not two modes. `MENU::DressSelect` keeps one hit map at a time
//! and swaps it: `FUN_1000d7f0` loads `System/DressSelect/DressSelect*.cmap` and
//! `FUN_1000d8c0` loads `System/DressSelect/Popup/Popup_Select*.cmap` into the
//! same member, each picking its widescreen variant from host `+0xcc`, `+0xd0`
//! and `+0xe4` the way every other screen does. [`Phase`] is which of the two is
//! loaded, and it is the module's `+0x140`.
//!
//! # The records, from `FUN_1000d980`, `FUN_1000ded0` and `FUN_1000ef80`
//!
//! One table of six 24-byte records at `DAT_10054920`, `x y w h src_x src_y` as
//! floats:
//!
//! ```text
//! rec 0   ( 64, 0) 273x450  src (  1,   1)   left dress, resting
//! rec 1   (463, 0) 273x450  src (275,   1)   right dress, resting
//! rec 2   ( 64, 0) 273x450  src (  1, 452)   left dress, lit
//! rec 3   (463, 0) 273x450  src (275, 452)   right dress, lit
//! rec 4   (274, 398) 115x26 src (  1,   1)   popup, widget 0
//! rec 5   (411, 398) 115x26 src (117,   1)   popup, widget 1
//! ```
//!
//! The first four are cut from `DressSelect_Chip.png` and the last two from
//! `Popup/Popup_Select_Chip.png`. Records 2 and 3 hold the *same destination
//! rectangle* as 0 and 1 and differ only in `src_y`, which is why the three
//! functions above can disagree about which record they anchor a slide to and
//! still place the sprite identically — `FUN_1000e440` slides against records 0
//! and 1 where `FUN_1000ded0` sets the same sprites up from records 1 and 2.
//! The player's own sheets confirm the split: `DressSelect_Chip.png` is 548x902,
//! exactly two columns of 273 and two rows of 450, and
//! `Popup_Select_Chip.png` is 232x27, a single row with no lit variant at all.
//!
//! # The dress is drawn, not revealed
//!
//! Every other screen in these modules has opaque base art and lights a widget
//! by drawing its chip sprite over it. This one's base is
//! `System/Screen/Transparence.png`, the shared full-screen transparent plate
//! (`FUN_1000d980` hands it to `FUN_10010620` paired with the chip sheet), so
//! **both dresses are sprites that are always drawn** — `FUN_1000c740` walks
//! `+0xc8` and `+0xcc` unconditionally — and the lit record goes over the one
//! under the pointer. The moving background is `STARTSCRIPT.INI`'s `[DressBG]`
//! playing behind the plate; see [`background`].
//!
//! `DressSelect_Text.png` is a caption over that, loaded by `FUN_1000cce0` as a
//! full-screen plate and drawn by `FUN_1000c740` only while [`Phase::Choosing`]
//! — the popup covers it rather than sitting under it.
//!
//! # The dispatch, from `FUN_1000ded0`
//!
//! `FUN_1000dea0` is the availability test and is `0 <= widget <= 1`: neither
//! dress is ever locked, on either hit map. What a click does depends on which
//! map is loaded, which is the whole of [`action`]:
//!
//! * While choosing, either widget commits. The module tells the host the
//!   choice at once — host `+0x48(1)` for widget 0 and `+0x48(0)` for widget 1
//!   — slides the two dresses together over [`SLIDE_FRAMES`] frames, then
//!   `FUN_1000e440` phase 1 loads the popup's art and hit map and raises
//!   `+0x140`.
//! * While confirming, widget 0 sets `+0xfc` and widget 1 clears `+0x140` and
//!   asks `FUN_1000e440` phase 3 to slide the dresses back apart and reload the
//!   main hit map.
//!
//! `+0xfc` is what `getNextMode` case 9 reads through `FUN_10001a10`, and it
//! answers mode 1 — leave the menus and play. `+0x88` answers mode -1, the
//! confirm popup, and anything else falls through to mode 2, the title.
//!
//! # What the choice reaches
//!
//! Host slot `+0x48` is `FUN_0041dc50` in `SHINYDAYS.exe`, which stores the
//! argument and raises a flag beside it. The interface is a secondary base
//! subobject installed at `[object + 0x2c]` — `FUN_0041d660` writes the vtable
//! `0x0048e50c` there, and `FUN_004167e0` hands the DLL `this + 0x2c` — so the
//! member the setter spells `+0x7d0` is the object's `+0x7fc`.
//!
//! Slot `+0x44` is the matching reader, `FUN_0041dc40`, and its callers are in
//! `RouteProcSD.dll`: `FUN_1004da70` and `FUN_100515e0` each play a scene block
//! and then append a letter to its name from this value — `A` when it is
//! non-zero, `B` when it is zero.
//!
//! ```text
//! FUN_1004da70   REP04_S1_B03  ->  REP04_S1_B03A / REP04_S1_B03B
//! FUN_100515e0   REP04_YX_A01  ->  REP04_YX_A01A / REP04_YX_A01B
//! ```
//!
//! So widget 0 is the `A` dress and widget 1 the `B` dress, and exactly two
//! scenes in the shipped route branch on it. That the interface is the one
//! `RouteProcSD` holds is confirmed by its own use of the neighbouring slots:
//! `+0x8(wstr)`, `+0xc(wstr, int)`, `+0x10(wstr) -> int` and `+0x1c(wstr, int)`
//! match the signatures at `0x0048e50c` exactly.
//!
//! # How the screen is entered
//!
//! The title screen's `START` widget asks for it. `FUN_1002fc60` case 0 — the
//! title module's dispatch — writes **9** into its next-mode member `+0xe0`,
//! and `_getNextMode@8` case 2 returns that member verbatim through
//! `FUN_100019b0`, a plain load with no clamp. `SysMenuSDHQ.dll`'s matching
//! arm, `FUN_100207a0` case 0, writes 1 instead, which is the whole of the
//! difference between the two titles here; see [`crate::ui::menu::Mode`].
//!
//! The executable takes it from there without ever naming the number.
//! `FUN_004158c0` is the mode pump: it loads `+0x2d8`, adds one, and jumps
//! through a four-entry table for modes -1 through 2 — every other mode,
//! 9 among them, falls to the **default arm**, which is
//! `FUN_00413250(mode)` with the mode passed as a parameter and its return
//! stored back into `+0x2d8`. That handler switches on its own phase counter
//! `+0x2dc` and, in phase 1, tests the mode twice: `mode == -1` and
//! `mode == 9` both skip `FUN_0041aa10`, the call that silences what is
//! already playing, and `mode == 9` additionally takes the arm described under
//! [`BACKGROUND_KEY`] before handing the mode to `_SystemInit@8` through
//! `FUN_004167e0`.
//!
//! So nothing writes a literal 9 in the executable at all: it arrives as a
//! parameter from the module, which is why a scan of the exe for the constant
//! finds nothing.

use days_ui::atlas::{Atlas, Widget};

use crate::install::ini::Ini;
use crate::ui::screen::Cut;

/// Which of the screen's two hit maps is loaded: `MENU::DressSelect` `+0x140`.
///
/// The module raises this once the commit slide has finished and
/// `FUN_1000e440` phase 1 has swapped the map, which is why choosing and
/// confirming cannot be told apart by the widget count — both maps carry two.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Phase {
    /// The main hit map: the two dresses.
    #[default]
    Choosing,
    /// The popup's hit map, over the dress the player committed to.
    Confirming {
        /// The widget that was clicked, which is also the index the two
        /// dresses are drawn in — not the value the host was told. See
        /// [`host_value`].
        chosen: usize,
    },
}

/// What a click on the screen does, from `FUN_1000ded0`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Act {
    /// The widget is not one of the two, or the click is not live.
    None,
    /// Commit to this dress and raise the popup. The host is told at once.
    Commit(usize),
    /// The popup's yes: `+0xfc`, which `getNextMode` answers with mode 1.
    Accept,
    /// The popup's no: back to the two dresses.
    Cancel,
}

/// `STARTSCRIPT.INI`'s key for what plays behind the transparent plate.
///
/// The shipped file sets it to `System/DressSelect/sentakuBG_03.wmv`, and the
/// install ships `sentakuBG_03.png` beside it. See [`background`] for which of
/// the two arms the value takes.
pub const BACKGROUND_KEY: &str = "DressBG";

/// What the mode-9 arm does with [`BACKGROUND_KEY`]'s value.
///
/// **The key chooses between the two by its own spelling**, in
/// `FUN_00413250`'s mode-9 arm: it hands the value and the literal `L".png"`
/// at `0x0048df38` to `wcsstr` — the import at IAT slot `0x0048c228`,
/// `MSVCR90.dll!wcsstr`, reached through the thunk at `0x0048330a` — and a hit
/// takes the still arm while a miss takes the movie arm. It is `wcsstr` and
/// not a suffix test, so what it asks is whether `.png` appears anywhere in
/// the value at all. Either way `FUN_00408da0` starts what was set up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Background {
    /// One picture, loaded straight into a texture and held: `FUN_00420df0`
    /// on the object at `this + 0x74`, with `+0x304` cleared so the per-frame
    /// arm below never runs.
    Still(String),
    /// A clip, played on a loop from its first frame: `FUN_004212a0` opens it
    /// on the `MENU::MovieView` at `this + 0x178` — the class `FUN_00421b20`
    /// names — `FUN_00421110` sets its frame to 0, and `+0x304` is set so the
    /// pump advances it. See [`BACKGROUND_FPS`].
    Movie(String),
}

impl Background {
    /// The asset path, as the INI spells it.
    pub fn path(&self) -> &str {
        match self {
            Background::Still(path) | Background::Movie(path) => path,
        }
    }
}

/// What goes behind the transparent plate, from `STARTSCRIPT.INI`.
///
/// `None` when the key is absent or empty — the arm still runs in the
/// original, on an empty string, and both loaders report a file they cannot
/// open; here that is one more missing asset, so the screen draws over what is
/// already behind it.
pub fn background(start: &Ini) -> Option<Background> {
    let value = start.get(BACKGROUND_KEY)?;
    if value.is_empty() {
        return None;
    }
    Some(if value.contains(".png") {
        Background::Still(value.to_string())
    } else {
        Background::Movie(value.to_string())
    })
}

/// The clock the movie arm advances its frame on: 24 frames a second.
///
/// The frame is a member of its own — `MENU::MovieView` `+0x3c`, which
/// `FUN_00421b20` zeroes and only `FUN_00421110` ever writes — and the mode
/// pump sets it from the wall clock: `FUN_00413250` stores `timeGetTime()` in
/// `+0x30c` as the screen comes up (case 2) and its two running phases (cases
/// 3 and 8) set the frame to `round(elapsed_ms * DAT_004b31e0) / 1000`.
/// `DAT_004b31e0` is written once, `FUN_00437e00`'s `mov dword ptr
/// [0x004b31e0], 0x18` — 24, the same frame clock `.ORS` timelines are
/// scheduled on.
///
/// # The clip loops
///
/// That frame number only ever goes up, and it is **not** evidence that the
/// clip is played once: the loader rebases its timestamps instead of rewinding
/// the count. `wmvLoader`'s pump `FUN_004532f0` asks
/// `IWMSyncReader::GetNextSample` for a sample, and on
/// `NS_E_NO_MORE_SAMPLES` — `FUN_00451820`'s `0xc00d0bcf` — it raises the
/// play counter `+0xfc`, adds the clip's whole length `+0x88` to the running
/// offset `+0x100` that every later sample's timestamp is taken from, and
/// calls `FUN_00451fe0`. That seeks the reader back to the start —
/// `IWMSyncReader::SetRange(0, 0)` through `FUN_004511a0`, vtable slot
/// `+0x14` — and says so in its own debug line, `L"先頭へシーク\n"`.
///
/// It stops only when it has played a set number of times: `FUN_00451fe0`
/// returns without seeking when `+0xf8` is non-zero and has been reached.
/// `+0xf8` is that play count, `FUN_004523c0` sets it, and `FUN_004212a0`
/// passes `(its second argument == 0)` — `setz cl` at `0x004213de`. The
/// mode-9 arm's second argument is 1 (`push ebx` at `0x00413379`, with `ebx`
/// 1 throughout `FUN_00413250`), so the count is zero and **the background
/// loops for as long as the screen is up**.
pub const BACKGROUND_FPS: f64 = 24.0;

/// How many frames the commit slide runs for: `_DAT_1004a750`, a double.
///
/// `FUN_1000e440` adds `_DAT_100497b0` — 1.0, also a double — to `+0x12c` each
/// tick and finishes when it reaches this, moving each dress a thirtieth of its
/// travel per frame.
pub const SLIDE_FRAMES: f32 = 30.0;

/// Where the left dress ends up once the slide finishes: `_DAT_1004a740`.
pub const SLID_LEFT_X: f32 = 258.5;

/// Where the right dress ends up: `_DAT_1004a748`.
pub const SLID_RIGHT_X: f32 = 268.5;

/// How many widgets each of the screen's two hit maps carries.
pub const WIDGETS: usize = 2;

/// The dispatch, from `FUN_1000ded0`.
///
/// `FUN_1000dea0` rejects anything outside `0..=1` before the switch, on both
/// maps, so an out-of-range widget is [`Act::None`] rather than a no-op click
/// that still plays a sound.
pub fn action(phase: Phase, widget: usize) -> Act {
    if widget >= WIDGETS {
        return Act::None;
    }
    match phase {
        Phase::Choosing => Act::Commit(widget),
        // The popup's widget 0 sets `+0xfc` and returns 0 — the update pump
        // reads that as "this screen is finished" — and widget 1 drops
        // `+0x140` and sets the slide-back phase.
        Phase::Confirming { .. } => match widget {
            0 => Act::Accept,
            _ => Act::Cancel,
        },
    }
}

/// What the host is told for a chosen widget: host `+0x48`'s argument.
///
/// `FUN_1000ded0` calls it with 1 for widget 0 and 0 for widget 1, and
/// `RouteProcSD.dll` reads the value back through slot `+0x44` to pick the `A`
/// variant of a scene block when it is non-zero and the `B` variant when it is
/// zero. So widget 0 is `A`.
pub fn host_value(widget: usize) -> u32 {
    u32::from(widget == 0)
}

/// The lit sprite for one of the two dresses: records 2 and 3.
///
/// These follow the two the hit map covers, so they are the leading entries of
/// [`Atlas::extras`]. They are looked up rather than assumed: a module whose
/// table does not carry them yields `None` and the dress is drawn resting,
/// which is a screen missing its highlight rather than a screen that will not
/// draw.
pub fn lit(atlas: &Atlas, widget: usize) -> Option<Widget> {
    if widget >= WIDGETS {
        return None;
    }
    atlas.extras.get(widget).copied()
}

/// The two dresses' resting sprites, which this screen draws itself.
///
/// `FUN_1000c740` walks `+0xc8` and `+0xcc` before anything else and draws both
/// every frame, because the base art under them is the transparent plate. Every
/// other screen leaves a resting widget to its base art.
pub fn resting(atlas: &Atlas) -> Vec<Widget> {
    atlas.widgets.iter().take(WIDGETS).copied().collect()
}

/// Where the two dresses sit once the popup is up.
///
/// `FUN_1000ded0` sets each sprite's travel to the difference between its
/// resting `x` and the constant for the side it lands on, and `FUN_1000ef80`
/// re-derives the same two positions when the display mode changes mid-commit.
/// The left dress always ends at [`SLID_LEFT_X`] and the right at
/// [`SLID_RIGHT_X`] whichever one was chosen; only which of them is lit
/// changes.
pub fn committed(atlas: &Atlas, chosen: usize) -> Vec<Cut> {
    // `FUN_1000c740` draws `+0xc8` and then `+0xcc`, and `FUN_1000ded0` loads
    // the *unchosen* dress's resting record into the first and the chosen
    // dress's lit record into the second — so the chosen one is always the one
    // on top, whichever side it is.
    let order = if chosen == 0 { [1, 0] } else { [0, 1] };
    order
        .into_iter()
        .filter_map(|widget| {
            let source = if widget == chosen {
                lit(atlas, widget)?
            } else {
                atlas.widgets.get(widget).copied()?
            };
            let x = if widget == 0 {
                SLID_LEFT_X
            } else {
                SLID_RIGHT_X
            };
            Some(cut_at(source, x))
        })
        .collect()
}

/// One record as a sprite, moved to a new `x` and left where it is in `y`.
fn cut_at(widget: Widget, x: f32) -> Cut {
    let (w, h) = (widget.dst.width as f32, widget.dst.height as f32);
    Cut {
        src: (widget.src_x as f32, widget.src_y as f32, w, h),
        dst: (x, widget.dst.y as f32, w, h),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use days_ui::Rect;

    fn widget(x: u32, y: u32, width: u32, height: u32, src_x: u32, src_y: u32) -> Widget {
        Widget {
            dst: Rect {
                x,
                y,
                width,
                height,
            },
            src_x,
            src_y,
        }
    }

    /// The six records `DAT_10054920` holds, as the atlas splits them: the two
    /// the hit map covers, then the two lit ones behind them.
    fn atlas() -> Atlas {
        Atlas {
            widgets: vec![
                widget(64, 0, 273, 450, 1, 1),
                widget(463, 0, 273, 450, 275, 1),
            ],
            extras: vec![
                widget(64, 0, 273, 450, 1, 452),
                widget(463, 0, 273, 450, 275, 452),
            ],
            offset: 0,
            segments: vec![(0, 0, 2)],
            matched: 2,
        }
    }

    /// The retail key names a clip, so the arm `wcsstr` misses is the one
    /// taken: `sentakuBG_03.wmv` plays rather than being held as a picture.
    #[test]
    fn the_shipped_key_is_a_clip() {
        let start = Ini::parse("[DressBG]=\"System/DressSelect/sentakuBG_03.wmv\"");
        assert_eq!(
            background(&start),
            Some(Background::Movie(
                "System/DressSelect/sentakuBG_03.wmv".to_string()
            ))
        );
    }

    /// Pointing the key at the `.png` the install ships beside the clip takes
    /// the still arm instead. `wcsstr` is a substring test, not a suffix one,
    /// so a value that only mentions `.png` takes it too.
    #[test]
    fn a_png_anywhere_in_the_value_takes_the_still_arm() {
        let start = Ini::parse("[DressBG]=\"System/DressSelect/sentakuBG_03.png\"");
        assert_eq!(
            background(&start),
            Some(Background::Still(
                "System/DressSelect/sentakuBG_03.png".to_string()
            ))
        );
        let odd = Ini::parse("[DressBG]=\"System/.png/clip.wmv\"");
        assert!(matches!(background(&odd), Some(Background::Still(_))));
    }

    /// A key the file does not carry is a missing asset, not a screen that
    /// refuses to draw.
    #[test]
    fn no_key_is_no_background() {
        assert_eq!(background(&Ini::parse("[TitleBGM]=\"x\"")), None);
        assert_eq!(background(&Ini::parse("[DressBG]=\"\"")), None);
    }

    /// Widget 0 is the `A` dress: `FUN_1000ded0` calls host `+0x48` with 1 for
    /// it, and `RouteProcSD.dll` takes non-zero to mean `A`.
    #[test]
    fn widget_zero_is_the_a_dress() {
        assert_eq!(host_value(0), 1);
        assert_eq!(host_value(1), 0);
    }

    /// The popup's two widgets are yes then no, and while choosing the same two
    /// indices commit instead — one hit map's widget 1 is the other's cancel.
    #[test]
    fn the_same_widget_means_different_things_on_each_map() {
        assert_eq!(action(Phase::Choosing, 1), Act::Commit(1));
        assert_eq!(action(Phase::Confirming { chosen: 1 }, 1), Act::Cancel);
        assert_eq!(action(Phase::Confirming { chosen: 1 }, 0), Act::Accept);
    }

    /// `FUN_1000dea0` bounds the widget before the switch on both maps.
    #[test]
    fn a_third_widget_does_nothing() {
        assert_eq!(action(Phase::Choosing, 2), Act::None);
        assert_eq!(action(Phase::Confirming { chosen: 0 }, 2), Act::None);
    }

    /// Records 2 and 3 share records 0 and 1's destination and differ only in
    /// `src_y` — the lit row of the sheet.
    #[test]
    fn the_lit_records_only_move_in_the_sheet() {
        let atlas = atlas();
        for widget in 0..WIDGETS {
            let lit = lit(&atlas, widget).expect("the lit record");
            assert_eq!(lit.dst, atlas.widgets[widget].dst);
            assert_eq!(lit.src_x, atlas.widgets[widget].src_x);
            assert_eq!(lit.src_y, 452);
        }
    }

    /// Both dresses land on the same two `x` positions whichever was chosen,
    /// and the chosen one is drawn last so it is on top.
    #[test]
    fn committing_slides_both_dresses_together() {
        let atlas = atlas();
        for chosen in 0..WIDGETS {
            let cuts = committed(&atlas, chosen);
            assert_eq!(cuts.len(), 2);
            let xs: Vec<f32> = cuts.iter().map(|cut| cut.dst.0).collect();
            assert!(xs.contains(&SLID_LEFT_X) && xs.contains(&SLID_RIGHT_X));
            assert_eq!(cuts.last().expect("the top sprite").src.1, 452.0);
        }
    }

    /// A module whose table stops at the two the map covers still draws: the
    /// dresses come up resting rather than the screen refusing.
    #[test]
    fn a_table_without_the_lit_records_still_draws() {
        let mut atlas = atlas();
        atlas.extras.clear();
        assert_eq!(lit(&atlas, 0), None);
        assert_eq!(resting(&atlas).len(), WIDGETS);
        assert_eq!(committed(&atlas, 0).len(), 1);
    }
}
