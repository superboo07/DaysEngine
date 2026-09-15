//! The play-data list — `Replay_PlayData`, the replay module's second view.
//!
//! # It is the save/load list, read through the same host call
//!
//! `FUN_1001b6d0` asks the host `+0x9c` for each slot of the page, which is
//! `FUN_0042a980` — the very call [`crate::ui::saveload`] models. Ten slots a
//! page, ten pages, `page * 10 + row`, three strings a row: the timestamp, the
//! chapter and the player's comment. So this screen lists the player's saves,
//! and everything [`crate::ui::saveload::Slots`] already reads is what fills it.
//!
//! Every layout constant is the same global as the save/load screen's, read at
//! the same width: 548 and 986 wide on the surface, 252 and 494 on screen, a
//! 48-pixel surface row coming down to 24. What differs is which records place
//! them and three details recorded below.
//!
//! # The widgets, from `FUN_1001dfe0` and `FUN_1001dd80`
//!
//! ```text
//! 0, 1        the two tab headers
//! 2           CLOSE
//! 3 .. 0xc    the ten rows, left band
//! 0xd .. 0x16 the ten page buttons
//! 0x17 .. 0x20 the ten rows, right band — the comment column
//! ```
//!
//! `FUN_1001dd80` answers true for every one of them and `FUN_1001dce0` makes
//! widgets 0 to 2 live unconditionally, so **nothing on this screen is greyed
//! out**. A row whose slot has no file is live and does nothing: the dispatch
//! tests `+0x17c + row * 4` — the host's own answer for that slot — before it
//! acts. The save/load screen behaves the same way.
//!
//! Both bands pick the same row. `FUN_1001a670` lights `+0xc4 + row * 4` when
//! the selection is `row + 3` **or** `row + 0x17`, so pointing at either half
//! highlights the whole row from the left band's full-width record. Only the
//! right band raises the expanded comment — `FUN_1001a060` calls
//! `FUN_1001bc80(this, selection - 0x17)` and nothing else does.
//!
//! # The records this screen's rows are laid out from
//!
//! The hit map does not reproduce them, so [`days_ui::atlas::find`] cannot place
//! them: a row's box is the half of the row the band covers, while the record is
//! the row entire. The indices come from the code instead —
//!
//! ```text
//!  0 ..  2   the tab headers and CLOSE          FUN_1001ae50
//!  7 .. 16   the ten page buttons, resting      FUN_1001ae50
//! 17 .. 26   the page buttons selected          FUN_1001c850, `page + 0x11`
//! 27 .. 36   the page buttons' third state      FUN_1001c850, `page + 0x1b`
//! 37 .. 46   the row bars, 799x31               FUN_1001ae50, `row + 0x25`
//! 47 .. 56   the comment column, 472x97         FUN_1001c850, `row + 0x2f`
//! ```
//!
//! — and [`relocate`] puts the two row runs back into the atlas once the table
//! itself is anchored. The anchor cannot be the tab headers: `REPLAY_PLAYDATA`
//! and `REPLAY_HSCENE` share their first seven records byte for byte, and the
//! two tables sit back to back in `.data`, so the tabs alone match both. The ten
//! page buttons are what tell them apart — the h-scene screen has four, at
//! different x — so the anchor is all thirteen boxes at their own indices.
//!
//! The 472x97 record is three rows tall for the same reason the save/load
//! screen's second band is: it doubles as the expanded comment's panel.
//!
//! # Three things it does not do that save/load does
//!
//! `FUN_1001c850`, `FUN_1001b6d0` and `FUN_1001bc80` never ask the host `+0x5c`,
//! which is the English question. So on this screen:
//!
//! * no column is shifted for English — save/load moves the timestamp 5 right
//!   and the chapter 15,
//! * no comment is centred — save/load's `comment_centre` has no counterpart
//!   here,
//! * every column is cut at twenty characters, where save/load gives English
//!   one more on the timestamp and twenty more on the comment.
//!
//! It also rasterises two pixels higher than it cuts. `FUN_1001b6d0` starts its
//! pen at `row * 0x30` and the comment's at `row * 0x30 + 0x200`, while
//! `FUN_1001c850` cuts the sprites at `row * 48 + 2` and `row * 48 + 514`. The
//! save/load pair agree; these two do not, and the two-pixel offset is theirs.

use crate::ui::options::Dir;
use crate::ui::replay::View;
use crate::ui::saveload::{
    self, Column, Line, Quad, Rows, Slots, Tooltip, DEST_HEIGHT, DEST_Y, LAST_ROW_OPENING_DOWN,
    PANEL_ROWS, PER_PAGE, SURFACE, SURFACE_ROW_HEIGHT, SURFACE_ROW_PITCH,
};
use days_ui::atlas::{self, Atlas, Widget};
use days_ui::cmap::Rect;

/// How many widgets the screen has, from its own hit map.
pub const WIDGETS: usize = 0x21;

/// The first widget of the left band of rows, from `FUN_1001dfe0`.
pub const FIRST_ROW: usize = 3;

/// The first page button.
pub const FIRST_PAGE: usize = 0xd;

/// The first widget of the right band — the comment column.
pub const FIRST_COMMENT: usize = 0x17;

/// One step of keyboard navigation on the play-data list: `FUN_1001e7e0`, the
/// sibling `FUN_1001e200` dispatches to when `+0x2b0` says this view is
/// showing. See [`crate::ui::replay::navigate`] for the members.
///
/// This view is a list, not a grid, so it has no `% 4` guard and nothing to
/// fix. Vertically it is a ring: CLOSE, the page button of the page showing,
/// the ten rows in order, back to CLOSE. Sideways is the top strip's ring
/// alone — the two view tabs and then the ten page buttons — and a row has no
/// sideways move, which is why the comment column at [`FIRST_COMMENT`] is not
/// in any arm: the pointer is the only thing that reaches it.
pub fn navigate(current: usize, dir: Dir, page: usize) -> usize {
    let c = current;
    let rows = FIRST_ROW..FIRST_ROW + PER_PAGE;
    let pages = FIRST_PAGE..FIRST_PAGE + PER_PAGE;
    let last_row = FIRST_ROW + PER_PAGE - 1;
    let last_page = FIRST_PAGE + PER_PAGE - 1;
    match dir {
        Dir::Up => match c {
            CLOSE => last_row,
            FIRST_ROW => FIRST_PAGE + page,
            _ if rows.contains(&c) => c - 1,
            _ => CLOSE,
        },
        Dir::Down => match c {
            CLOSE => FIRST_PAGE + page,
            _ if c == last_row => CLOSE,
            _ if rows.contains(&c) => c + 1,
            _ => FIRST_ROW,
        },
        Dir::Left => match c {
            0 => last_page,
            1 => 0,
            FIRST_PAGE => 1,
            _ if pages.contains(&c) => c - 1,
            _ => c,
        },
        Dir::Right => match c {
            0 => 1,
            1 => FIRST_PAGE,
            _ if c == last_page => 0,
            _ if pages.contains(&c) => c + 1,
            _ => c,
        },
    }
}

/// CLOSE, which this table treats as a place of its own.
const CLOSE: usize = 2;

/// The page a keyboard step opens, if it landed on a page button.
///
/// The same guard the other view has, on `c - 0xd` against `+0x2a4`.
pub fn opens_page(dir: Dir, next: usize) -> Option<usize> {
    match dir {
        Dir::Left | Dir::Right => (FIRST_PAGE..FIRST_PAGE + PER_PAGE)
            .contains(&next)
            .then(|| next - FIRST_PAGE),
        Dir::Up | Dir::Down => None,
    }
}

/// Record indices, from the functions named in the module doc.
const PAGE_RECORD: usize = 7;
const ROW_RECORD: usize = 0x25;
const COMMENT_RECORD: usize = 0x2f;

/// The tab of the view showing, and the same tab with the pointer on it:
/// `FUN_1001ae50` binds `+0x160` to record `view + 3` and `+0x164` to record
/// `view + 5`, and `FUN_1001a670` picks between them.
const TAB_CURRENT: usize = 3;
const TAB_SELECTED: usize = 5;

/// The button of the page showing, and the same button with the pointer on it:
/// `FUN_1001c850` binds `+0x124` to record `page + 0x11` and `+0x150` to record
/// `page + 0x1b`.
const PAGE_CURRENT: usize = 0x11;
const PAGE_SELECTED: usize = 0x1b;

/// Where this screen's alternate-state records begin. [`relocate`] puts records
/// `ALTERNATES ..ROW_RECORD` into [`Atlas::extras`], so `extras[n]` is record
/// `ALTERNATES + n` — the two tabs and the ten page buttons in their two other
/// states each. Record `ROW_RECORD` is the first row bar, which is a widget.
const ALTERNATES: usize = TAB_CURRENT;

/// How many characters of any column are drawn, from `FUN_1001b6d0`'s
/// `if (0x14 < n) n = 0x14`. The same cap on all three, English or not.
pub const CAP: usize = 0x14;

/// Where `FUN_1001b6d0`'s pen starts down the surface for each column, against
/// `FUN_1001c850`'s own cut two pixels below it.
const PEN_LINE_Y: f32 = 0.0;
const PEN_COMMENT_Y: f32 = 512.0;
const CUT_LINE_Y: f32 = 2.0;
const CUT_COMMENT_Y: f32 = 514.0;

/// The expanded comment's pen, cut and drawn size, from `FUN_1001bc80`.
///
/// It starts at `(0x400, 0x200)` and steps `0x40` a line, and the sprite cuts
/// `_DAT_1003b120` (1024.0f), `_DAT_1003cf74` (514.0f), `_DAT_1003b110`
/// (986.0f) by `_DAT_1003cf78` (192.0f) — three lines' worth in one piece,
/// where save/load gives each line a sprite of its own. It is drawn
/// `_DAT_1003b0d0` (494.0) by `_DAT_1003cf80` (96.0), so the three line slots
/// are 32 screen pixels each.
const TIP_PEN: (f32, f32) = (1024.0, 512.0);
const TIP_PEN_PITCH: f32 = 64.0;
const TIP_CUT: (f32, f32, f32, f32) = (1024.0, 514.0, 986.0, 192.0);
const TIP_DEST_HEIGHT: f32 = 96.0;

/// How far down the surface a deep row's lines are rasterised so the fixed cut
/// picks them up in the right slot, from `FUN_1001bc80`: `0x84` for one line
/// and `0x41` for two. Three lines fill the slice and are not moved.
const TIP_DEEP_PEN: [f32; 2] = [132.0, 65.0];

/// What activating a widget does, from `FUN_1001dfe0`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Act {
    /// Nothing on this screen.
    None,
    /// Show the other view.
    View(View),
    /// Leave the replay screen.
    Back,
    /// Show a page of the list.
    Page(usize),
    /// Play the slot this row names.
    Row(usize),
}

/// The play-data list's dispatch, from `FUN_1001dfe0`.
///
/// Both bands of a row give the same [`Act::Row`]; whether that row has a slot
/// to play is the caller's to ask, the way the shipped dispatch asks
/// `+0x17c + row * 4` before it acts.
pub fn action(widget: usize) -> Act {
    if let Some(view) = View::from_widget(widget) {
        return Act::View(view);
    }
    match widget {
        2 => Act::Back,
        FIRST_ROW..=0xc => Act::Row(widget - FIRST_ROW),
        FIRST_PAGE..=0x16 => Act::Page(widget - FIRST_PAGE),
        FIRST_COMMENT..=0x20 => Act::Row(widget - FIRST_COMMENT),
        _ => Act::None,
    }
}

/// Whether a widget can be chosen, from `FUN_1001dd80` and `FUN_1001dce0`.
///
/// Every one of them, including a row whose slot is empty — see the module doc.
pub fn enabled(widget: usize) -> bool {
    widget < WIDGETS
}

/// Which widget's sprite the selection lights, from `FUN_1001a670`.
///
/// Either band of a row lights the left band's record, which is the row entire.
pub fn highlight(widget: usize) -> Option<usize> {
    match widget {
        FIRST_ROW..=0xc => Some(widget),
        FIRST_COMMENT..=0x20 => Some(widget - FIRST_COMMENT + FIRST_ROW),
        _ => Some(widget).filter(|w| *w < WIDGETS),
    }
}

/// The row whose comment is expanded, from `FUN_1001a060`'s `0x16 < selection`.
///
/// Only the right band opens it.
pub fn tooltip_row(widget: usize) -> Option<usize> {
    (FIRST_COMMENT..WIDGETS)
        .contains(&widget)
        .then(|| widget - FIRST_COMMENT)
}

/// Puts the two runs of row records into an atlas that could not place them.
///
/// Returns whether it did. A screen whose table cannot be anchored is left
/// exactly as the generic search left it and logged — the rows then draw from
/// whatever the search extrapolated, which is wrong, so the caller treats a
/// false here as "this screen has no list" rather than drawing one.
pub fn relocate(atlas: &mut Atlas, dll: &[u8], boxes: &[Rect]) -> bool {
    if boxes.len() != WIDGETS || atlas.widgets.len() != WIDGETS {
        log::warn!(
            "the play-data list has {} regions and {} widgets, not {WIDGETS} of each",
            boxes.len(),
            atlas.widgets.len()
        );
        return false;
    }
    // The page buttons are the anchor; the tabs alone match the h-scene table
    // too. See the module doc.
    let mut anchors: Vec<(usize, Rect)> = (0..3).map(|i| (i, boxes[i])).collect();
    anchors.extend((0..PER_PAGE).map(|row| (PAGE_RECORD + row, boxes[FIRST_PAGE + row])));
    let Some(base) = atlas::table_at(dll, &anchors) else {
        log::warn!("no record table in the DLL places the play-data list's rows");
        return false;
    };

    let mut rows: Vec<(Widget, Widget)> = Vec::with_capacity(PER_PAGE);
    for row in 0..PER_PAGE {
        let bar = atlas::record_at(dll, base, ROW_RECORD + row);
        let panel = atlas::record_at(dll, base, COMMENT_RECORD + row);
        let (Some(bar), Some(panel)) = (bar, panel) else {
            log::warn!("the play-data table stops before row {row}");
            return false;
        };
        if !spans(&bar, &boxes[FIRST_ROW + row]) || !spans(&panel, &boxes[FIRST_COMMENT + row]) {
            log::warn!("the play-data table's row {row} does not cover the band it is drawn over");
            return false;
        }
        rows.push((bar, panel));
    }
    for (row, (bar, panel)) in rows.into_iter().enumerate() {
        atlas.widgets[FIRST_ROW + row] = bar;
        atlas.widgets[FIRST_COMMENT + row] = panel;
    }
    atlas.extras = (ALTERNATES..ROW_RECORD)
        .map(|index| atlas::record_at(dll, base, index))
        .collect::<Option<_>>()
        .unwrap_or_default();
    atlas.offset = base;
    log::info!(
        "play-data list: {WIDGETS} widgets and {} alternate records from the table at {base:#x}",
        atlas.extras.len()
    );
    true
}

/// Which alternate record a widget draws instead of its resting or active one.
///
/// `FUN_1001a670` draws two sprites the resting/active pair cannot express, and
/// both say *this is where you are*: the tab of the view showing and the button
/// of the page showing. Each has a second form for the pointer being on it. The
/// answer indexes [`Atlas::extras`] as [`relocate`] fills it.
pub fn extra_for(widget: usize, view: View, page: usize, selected: bool) -> Option<usize> {
    let record = if View::from_widget(widget) == Some(view) {
        widget + if selected { TAB_SELECTED } else { TAB_CURRENT }
    } else if widget == FIRST_PAGE + page {
        page + if selected {
            PAGE_SELECTED
        } else {
            PAGE_CURRENT
        }
    } else {
        return None;
    };
    record.checked_sub(ALTERNATES)
}

/// Whether a record covers the hit band it is drawn over: the same top edge,
/// no shorter, and no narrower at either end.
fn spans(record: &Widget, band: &Rect) -> bool {
    record.dst.y == band.y
        && record.dst.height >= band.height
        && record.dst.x <= band.x
        && record.dst.x + record.dst.width >= band.x + band.width
}

/// Which record places a column of a row.
///
/// `FUN_1001c850` reads `row + 0x25` for the timestamp and the chapter and
/// `row + 0x2f` for the comment, which is the same split save/load makes
/// between its two bands — and in this atlas those are the widgets
/// [`relocate`] put them in.
pub fn record_of(column: Column, row: usize) -> usize {
    match column {
        Column::When | Column::Chapter => FIRST_ROW + row,
        Column::Comment => FIRST_COMMENT + row,
    }
}

/// Where in the surface a column of a row is rasterised, from `FUN_1001b6d0`.
pub fn surface_pen(column: Column, row: usize) -> (i32, i32) {
    let base = match column {
        Column::When | Column::Chapter => PEN_LINE_Y,
        Column::Comment => PEN_COMMENT_Y,
    };
    (
        column.surface_span().0 as i32,
        (row as f32 * SURFACE_ROW_PITCH + base) as i32,
    )
}

/// The rectangle of the surface a column of a row is cut from, from
/// `FUN_1001c850`. Two pixels below [`surface_pen`] — see the module doc.
pub fn source_rect(column: Column, row: usize) -> (f32, f32, f32, f32) {
    let (x, width) = column.surface_span();
    let base = match column {
        Column::When | Column::Chapter => CUT_LINE_Y,
        Column::Comment => CUT_COMMENT_Y,
    };
    (
        x,
        row as f32 * SURFACE_ROW_PITCH + base,
        width,
        SURFACE_ROW_HEIGHT,
    )
}

/// Where a column of a row is drawn, in the 800x450 layout space the records
/// use, from `FUN_1001c850`.
///
/// `english` is not a parameter because the shipped function never asks: the
/// offsets are the ones save/load uses for Japanese, on every language.
pub fn dest_rect(column: Column, record: Rect) -> (f32, f32, f32, f32) {
    (
        record.x as f32 + column.dest_x(false),
        record.y as f32 + DEST_Y,
        column.dest_width(),
        DEST_HEIGHT,
    )
}

/// Rasterises a page of the list.
///
/// A row whose slot has no file is skipped entirely, which is what
/// `FUN_1001b6d0` does: it only draws when the host's slot query answers 1.
/// `comments` is host `+0xd8` — `FILMENGINE.INI [TextInput]` — which gates the
/// comment column here exactly as it does on the save/load screen.
pub fn render(
    font: &days_font::Font,
    slots: &Slots,
    page: usize,
    records: &[Widget],
    comments: bool,
    hovered: Option<usize>,
) -> Rows {
    let (width, height) = SURFACE;
    let mut surface = days_ui::Image::empty(width, height);
    let mut quads = Vec::new();

    for row in 0..PER_PAGE {
        let Some(line) = slots.get(saveload::slot_of(page, row)) else {
            continue;
        };
        for column in Column::ALL {
            if column == Column::Comment && !comments {
                continue;
            }
            let text: String = text_of(line, column).chars().take(CAP).collect();
            if text.is_empty() {
                continue;
            }
            let Some(record) = records.get(record_of(column, row)) else {
                continue;
            };
            // English never shifts anything on this screen, but the glyph
            // advances are still the menu font's own, so the flag still reaches
            // the rasteriser.
            saveload::draw_line(&mut surface, font, &text, surface_pen(column, row), &|c| {
                crate::playback::text::menu_advance(c, false)
            });
            let (sx, sy, sw, sh) = source_rect(column, row);
            quads.push(Quad {
                src: (sx as u32, sy as u32, sw as u32, sh as u32),
                dst: dest_rect(column, record.dst),
            });
        }
    }

    let tooltip = hovered
        .filter(|_| comments)
        .and_then(|row| expand(&mut surface, font, slots, page, row, records));
    Rows {
        surface,
        quads,
        tooltip,
        tip_surface: None,
    }
}

fn text_of(line: &Line, column: Column) -> &str {
    match column {
        Column::When => &line.when,
        Column::Chapter => &line.chapter,
        Column::Comment => &line.comment,
    }
}

/// Rasterises the expanded comment and lays it out, from `FUN_1001bc80`.
///
/// The wrap is [`saveload::wrap_comment`]'s Japanese arm — break every twenty
/// characters, stop after three lines' worth — which is the rule this function
/// spells out in full, having no English arm to choose between.
fn expand(
    surface: &mut days_ui::Image,
    font: &days_font::Font,
    slots: &Slots,
    page: usize,
    row: usize,
    records: &[Widget],
) -> Option<Tooltip> {
    let comment = &slots.get(saveload::slot_of(page, row))?.comment;
    let lines = saveload::wrap_comment(comment, false);
    if lines.is_empty() {
        return None;
    }
    let record = *records.get(record_of(Column::Comment, Tooltip::record_row(row)))?;

    // How many line slots the panel needs, from the capped character count
    // rather than from the wrap — `FUN_1001bc80` divides and switches on that,
    // and for this screen's twenty-character wrap the two always agree.
    let slots_used = comment.chars().count().min(CAP * saveload::TIP_LINES) / CAP;
    let deep = row > LAST_ROW_OPENING_DOWN;
    let height = record.dst.height as f32;
    let row_of_panel = height / PANEL_ROWS;

    // `FUN_1001bc80` writes the panel's shift only on the branches a deep row
    // takes and zeroes it only on the three-line one, so a shallow row with one
    // or two lines reads an uninitialised float — the same shipped bug the
    // save/load tooltip has. Zero is what the branch that does initialise it
    // uses, and what puts the panel on the row it belongs to.
    let (panel_shift, panel_height, pen_shift) = match slots_used {
        0 => (
            if deep { row_of_panel * 2.0 } else { 0.0 },
            row_of_panel - 2.0,
            if deep { TIP_DEEP_PEN[0] } else { 0.0 },
        ),
        1 => (
            if deep { row_of_panel } else { 0.0 },
            row_of_panel * 2.0,
            if deep { TIP_DEEP_PEN[1] } else { 0.0 },
        ),
        _ => (0.0, height, 0.0),
    };

    for (n, line) in lines.iter().enumerate() {
        let pen = (
            TIP_PEN.0 as i32,
            (TIP_PEN.1 + pen_shift + n as f32 * TIP_PEN_PITCH) as i32,
        );
        saveload::draw_line(surface, font, line, pen, &|c| {
            crate::playback::text::menu_advance(c, false)
        });
    }

    let panel = Quad {
        // The half-pixel outset is the DLL's, on this sprite as on every other.
        src: (
            record.src_x,
            record.src_y,
            record.dst.width,
            record.dst.height,
        ),
        dst: (
            record.dst.x as f32 - 0.5,
            record.dst.y as f32 - 0.5 + panel_shift,
            record.dst.width as f32 + 1.0,
            panel_height + 1.0,
        ),
    };
    // One sprite, three line slots. Which slot the text lands in was decided by
    // where it was rasterised, not by where this is drawn.
    let text = Quad {
        src: (
            TIP_CUT.0 as u32,
            TIP_CUT.1 as u32,
            TIP_CUT.2 as u32,
            TIP_CUT.3 as u32,
        ),
        dst: (
            record.dst.x as f32 + Column::Comment.dest_x(false),
            record.dst.y as f32 + DEST_Y,
            Column::Comment.dest_width(),
            TIP_DEST_HEIGHT,
        ),
    };
    Some(Tooltip {
        panel,
        lines: vec![text],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Both bands of a row pick the same row and light the same sprite, and
    /// only the right band expands the comment — `FUN_1001dfe0` and
    /// `FUN_1001a670` against `FUN_1001a060`.
    #[test]
    fn either_band_picks_the_row_and_only_the_right_one_expands_it() {
        for row in 0..PER_PAGE {
            let left = FIRST_ROW + row;
            let right = FIRST_COMMENT + row;
            assert_eq!(action(left), Act::Row(row));
            assert_eq!(action(right), Act::Row(row));
            assert_eq!(highlight(left), Some(left));
            assert_eq!(highlight(right), Some(left));
            assert_eq!(tooltip_row(left), None);
            assert_eq!(tooltip_row(right), Some(row));
            assert_eq!(action(FIRST_PAGE + row), Act::Page(row));
        }
        assert_eq!(action(0), Act::View(View::HScene));
        assert_eq!(action(1), Act::View(View::PlayData));
        assert_eq!(action(2), Act::Back);
        assert_eq!(action(WIDGETS), Act::None);
    }

    /// The panel opens downwards for the first eight rows and upwards for the
    /// last two, and its height is the record's own divided by the slots the
    /// comment needs — `FUN_1001bc80`'s `param_1 < 8` and its thirds.
    #[test]
    fn a_deep_rows_panel_opens_upwards_from_a_borrowed_record() {
        assert_eq!(Tooltip::record_row(7), 7);
        assert_eq!(Tooltip::record_row(8), 6);
        assert_eq!(Tooltip::record_row(9), 7);
        // 97 is the shipped record height; a third of it, less two, is the
        // one-line panel.
        let third = 97.0 / PANEL_ROWS;
        assert!((third - 32.333332).abs() < 1e-4);
        assert!((third - 2.0 - 30.333332).abs() < 1e-4);
    }
}
