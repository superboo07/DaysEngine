//! Where each widget's art lives inside a `_CHIP` sprite sheet.
//!
//! # The problem
//!
//! A FILMEngine screen is three files: `NAME.PNG` (base art), `NAME_CHIP.PNG`
//! (a sheet of widget state sprites) and `NAME*.CMAP` (the hit map). The hit
//! map says *where each widget is on screen*. Nothing in the data says *where
//! that widget's sprite is in the sheet*, and the packing is not recoverable by
//! looking at the sheet:
//!
//! - `TITLE` looks regular — five 116x38 regions, a 584x78 sheet, five across
//!   and two rows.
//! - `MENUBAR` is not. Its 25 region widths sum to 920 against a 799-wide
//!   sheet, in four different widget sizes, and the sheet is 408 tall.
//!
//! # The answer
//!
//! The mapping is a **table of 32-bit floats in `SysMenuSDHQ.dll`**, six per
//! widget (24 bytes), laid out as:
//!
//! ```text
//! dst_x  dst_y  width  height  src_x  src_y
//! ```
//!
//! `SystemMenuSDHQ`'s screen setup walks that array, and for each record calls
//! a destination-rect setter with `(dst_x, dst_y + y_offset, width, height)`
//! scaled by the display scale, then a source-rect setter with
//! `(src_x, src_y, width, height)`. Destination and source share one size, so a
//! chip sprite is never scaled relative to its widget.
//!
//! There is no rule to derive: `MENUBAR` widget 1 draws from `src_y = 67` while
//! widgets 2..15 draw from rows 1 and 29, and the sheet's lower two thirds hold
//! alternate captions in no particular order. It had to come out of the code.
//!
//! # Locating the table without hardcoding an address
//!
//! We do not embed the offsets — partly because that is game data we must not
//! ship, and partly because a hardcoded RVA is a silent breakage waiting for a
//! different build of the DLL. Instead the table is **found by its content**:
//! the first four floats of record *i* are exactly the bounding box of region
//! *i + 1* in the screen's native-resolution `.CMAP`. So we take the boxes from
//! the user's own `.CMAP`, search the DLL's data sections for a run of records
//! whose destination rects match all of them at a 24-byte stride, and read the
//! source coordinates out of the match.
//!
//! That is self-validating. It is also tolerant in one direction and strict in
//! the other: a region's bounding box can legitimately differ from its sprite
//! rect — regions that abut each other have their boxes clipped by the
//! neighbour — so a table is accepted on a scored match rather than a perfect
//! one, and [`Atlas::matched`] reports how many regions agreed. What is not
//! tolerated is a weak match: below the threshold we report that the table was
//! not found rather than drawing sprites from the wrong offsets.
//!
//! Some screens have no table to find. The save/load slot rows and the replay
//! thumbnail grids are laid out by a loop at runtime — ten rows at a 33px pitch
//! — so their rects exist nowhere in the binary. Those screens need their
//! layout reproduced in code; the search says so rather than guessing.
//!
//! # Native resolution
//!
//! The stored rects are in **800x450** space. Everything else is derived:
//! `_WIDE_NOTE` (1024x576) is that times 1.28 and `_WIDE_FULL` (1280x720) times
//! 1.6, while 4:3 800x600 is the same 800x450 content offset 75px down between
//! letterbox bars. The shipped `.CMAP` at each resolution is exactly the
//! 800x450 map put through that transform, which is how the scale factors here
//! were confirmed. See [`crate::Resolution`].

use crate::cmap::Rect;
use crate::Error;

/// One widget's placement: where it goes, and where its art comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Widget {
    /// Destination rectangle in 800x450 layout space.
    pub dst: Rect,
    /// Top-left of the sprite in the `_CHIP` sheet. Its size is `dst`'s size.
    pub src_x: u32,
    pub src_y: u32,
}

/// A screen's widget table, recovered from the DLL.
#[derive(Debug, Clone)]
pub struct Atlas {
    /// One entry per region ID, in ID order starting at 1.
    pub widgets: Vec<Widget>,
    /// Records that follow the per-region run: alternate states (hover,
    /// pressed, disabled) and caption sprites.
    ///
    /// **Best effort.** The table is contiguous but nothing in the data marks
    /// its end — the shipped code reaches these by fixed address, so how many
    /// there are and what each one means is per-screen knowledge, not something
    /// the bytes carry. The run is cut at the first record that cannot be a
    /// sprite for this sheet, so trailing entries may belong to another screen.
    pub extras: Vec<Widget>,
    /// Byte offset of the table in the DLL image, for diagnostics.
    pub offset: usize,
    /// How many regions' bounding boxes the table reproduced exactly, out of
    /// `widgets.len()`. Anything short of all of them means those widgets'
    /// boxes are clipped by a neighbour, not that the table is wrong.
    pub matched: usize,
}

/// Bytes per record: six 32-bit floats.
const RECORD: usize = 24;

/// How far past the per-region run to keep reading alternate-state records.
const MAX_EXTRAS: usize = 64;

fn f32le(b: &[u8], o: usize) -> Option<f32> {
    b.get(o..o + 4)
        .map(|s| f32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

/// Reads a record as six floats, requiring each to be a non-negative whole
/// number that fits a `u32` — every real entry is an integral pixel coordinate,
/// so this rejects unrelated float data cheaply.
fn record(dll: &[u8], at: usize) -> Option<Widget> {
    let mut v = [0u32; 6];
    for (i, slot) in v.iter_mut().enumerate() {
        let f = f32le(dll, at + i * 4)?;
        if !f.is_finite() || f < 0.0 || f > 8192.0 || f.fract() != 0.0 {
            return None;
        }
        *slot = f as u32;
    }
    if v[2] == 0 || v[3] == 0 {
        return None;
    }
    Some(Widget {
        dst: Rect {
            x: v[0],
            y: v[1],
            width: v[2],
            height: v[3],
        },
        src_x: v[4],
        src_y: v[5],
    })
}

/// Fewest exactly-matching regions that will be believed, for screens with
/// enough regions to make a coincidence conceivable.
const MIN_MATCHES: usize = 3;

/// Finds the widget table for a screen.
///
/// `boxes` are the region bounding boxes from the screen's **native 800x450**
/// `.CMAP` (or the plain `.CMAP` for screens shipped only at one size, such as
/// `MENUBAR`), in region-ID order. `chip` is the `_CHIP` sheet's size, used to
/// bound the trailing alternate-state records.
pub fn find(dll: &[u8], boxes: &[Rect], chip: (u32, u32)) -> Result<Atlas, Error> {
    if boxes.is_empty() {
        return Err(Error::NoAtlas);
    }

    // Anchor on every region in turn, not just the first. A screen can have
    // widgets that are laid out at runtime sitting ahead of tabled ones in ID
    // order — the save/load screen's slot rows do exactly that — so insisting
    // the table start at region 1 would miss tables that are really there.
    let mut best: Option<(usize, usize)> = None;
    let mut seen = Vec::new();
    for (k, anchor) in boxes.iter().enumerate() {
        let needle: Vec<u8> = [anchor.x, anchor.y, anchor.width, anchor.height]
            .iter()
            .flat_map(|v| (*v as f32).to_le_bytes())
            .collect();
        let mut at = 0usize;
        while let Some(found) = find_bytes(dll, &needle, at) {
            at = found + 4;
            let Some(start) = found.checked_sub(k * RECORD) else {
                continue;
            };
            if seen.contains(&start) {
                continue;
            }
            seen.push(start);
            let score = boxes
                .iter()
                .enumerate()
                .filter(|(i, want)| {
                    record(dll, start + i * RECORD).is_some_and(|w| w.dst == **want)
                })
                .count();
            if best.is_none_or(|(_, b)| score > b) {
                best = Some((start, score));
            }
        }
    }

    let (start, matched) = best.ok_or(Error::NoAtlas)?;
    if matched < MIN_MATCHES.min(boxes.len()) || matched * 2 < boxes.len() {
        return Err(Error::NoAtlas);
    }

    // The table, not the hit map, is what drawing uses: a clipped bounding box
    // would place the sprite a pixel or two off, and the table is the rect the
    // original blits with.
    let mut widgets = Vec::with_capacity(boxes.len());
    for i in 0..boxes.len() {
        widgets.push(record(dll, start + i * RECORD).ok_or(Error::NoAtlas)?);
    }

    let mut extras = Vec::new();
    let mut p = start + boxes.len() * RECORD;
    while extras.len() < MAX_EXTRAS {
        let Some(w) = record(dll, p) else { break };
        // A sprite for this screen has to fit inside this screen's sheet.
        if w.src_x + w.dst.width > chip.0 || w.src_y + w.dst.height > chip.1 {
            break;
        }
        extras.push(w);
        p += RECORD;
    }

    Ok(Atlas {
        widgets,
        extras,
        offset: start,
        matched,
    })
}

/// `slice::find` for bytes, from `from`.
fn find_bytes(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if from >= haystack.len() {
        return None;
    }
    haystack[from..]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| p + from)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(v: [f32; 6]) -> Vec<u8> {
        v.iter().flat_map(|f| f.to_le_bytes()).collect()
    }

    fn boxes(v: &[[u32; 4]]) -> Vec<Rect> {
        v.iter()
            .map(|b| Rect {
                x: b[0],
                y: b[1],
                width: b[2],
                height: b[3],
            })
            .collect()
    }

    #[test]
    fn a_run_matching_every_region_yields_the_source_coordinates() {
        let mut dll = vec![0xaau8; 64];
        dll.extend(rec([10.0, 20.0, 30.0, 40.0, 1.0, 1.0]));
        dll.extend(rec([50.0, 20.0, 30.0, 40.0, 32.0, 1.0]));
        dll.extend(vec![0u8; 32]);
        let a = find(
            &dll,
            &boxes(&[[10, 20, 30, 40], [50, 20, 30, 40]]),
            (128, 128),
        )
        .unwrap();
        assert_eq!(a.offset, 64);
        assert_eq!(a.widgets.len(), 2);
        assert_eq!((a.widgets[1].src_x, a.widgets[1].src_y), (32, 1));
    }

    #[test]
    fn a_partial_match_is_skipped_in_favour_of_the_real_run() {
        // Region 1's rect appears once on its own, then again at the head of a
        // run that matches both regions.
        let mut dll = rec([10.0, 20.0, 30.0, 40.0, 0.0, 0.0]);
        dll.extend(vec![0u8; 24]);
        let real = dll.len();
        dll.extend(rec([10.0, 20.0, 30.0, 40.0, 1.0, 1.0]));
        dll.extend(rec([50.0, 20.0, 30.0, 40.0, 32.0, 1.0]));
        let a = find(
            &dll,
            &boxes(&[[10, 20, 30, 40], [50, 20, 30, 40]]),
            (128, 128),
        )
        .unwrap();
        assert_eq!(a.offset, real);
    }

    #[test]
    fn trailing_state_records_stop_at_the_edge_of_the_sheet() {
        let mut dll = rec([10.0, 20.0, 30.0, 40.0, 1.0, 1.0]);
        dll.extend(rec([10.0, 20.0, 30.0, 40.0, 1.0, 42.0])); // fits a 128-tall sheet
        dll.extend(rec([10.0, 20.0, 30.0, 40.0, 1.0, 400.0])); // does not
        let a = find(&dll, &boxes(&[[10, 20, 30, 40]]), (128, 128)).unwrap();
        assert_eq!(a.extras.len(), 1);
        assert_eq!(a.extras[0].src_y, 42);
    }

    #[test]
    fn a_region_whose_box_is_clipped_still_lands_on_the_table() {
        // Region 3's box is a pixel narrower than the sprite rect, as happens
        // when a neighbouring region clips it. The table is still found, and
        // drawing uses the table's rect.
        let mut dll = vec![0u8; 8];
        dll.extend(rec([10.0, 20.0, 30.0, 40.0, 1.0, 1.0]));
        dll.extend(rec([50.0, 20.0, 30.0, 40.0, 32.0, 1.0]));
        dll.extend(rec([90.0, 20.0, 30.0, 40.0, 63.0, 1.0]));
        dll.extend(rec([130.0, 20.0, 30.0, 40.0, 94.0, 1.0]));
        let a = find(
            &dll,
            &boxes(&[
                [10, 20, 30, 40],
                [50, 20, 30, 40],
                [90, 20, 29, 40],
                [130, 20, 30, 40],
            ]),
            (128, 128),
        )
        .unwrap();
        assert_eq!(a.matched, 3);
        assert_eq!(a.widgets[2].dst.width, 30);
        assert_eq!(a.widgets[2].src_x, 63);
    }

    #[test]
    fn a_table_that_does_not_start_at_region_one_is_still_found() {
        // Regions 1 and 2 are laid out at runtime and appear nowhere, so the
        // run has to be reached by anchoring on region 3 instead.
        let mut dll = vec![0u8; 16];
        let start = dll.len();
        dll.extend(rec([0.0, 0.0, 1.0, 1.0, 0.0, 0.0]));
        dll.extend(rec([0.0, 0.0, 1.0, 1.0, 0.0, 0.0]));
        dll.extend(rec([90.0, 20.0, 30.0, 40.0, 63.0, 1.0]));
        dll.extend(rec([130.0, 20.0, 30.0, 40.0, 94.0, 1.0]));
        dll.extend(rec([170.0, 20.0, 30.0, 40.0, 125.0, 1.0]));
        let a = find(
            &dll,
            &boxes(&[
                [7, 7, 9, 9],
                [8, 8, 9, 9],
                [90, 20, 30, 40],
                [130, 20, 30, 40],
                [170, 20, 30, 40],
            ]),
            (128, 128),
        )
        .unwrap();
        assert_eq!(a.offset, start);
        assert_eq!(a.matched, 3);
        assert_eq!(a.widgets[4].src_x, 125);
    }

    #[test]
    fn too_few_matching_regions_is_refused_rather_than_believed() {
        // Two agreeing records out of four is coincidence territory: drawing
        // from that offset would put every sprite somewhere arbitrary.
        let mut dll = vec![0u8; 16];
        dll.extend(rec([0.0, 0.0, 1.0, 1.0, 0.0, 0.0]));
        dll.extend(rec([0.0, 0.0, 1.0, 1.0, 0.0, 0.0]));
        dll.extend(rec([90.0, 20.0, 30.0, 40.0, 63.0, 1.0]));
        dll.extend(rec([130.0, 20.0, 30.0, 40.0, 94.0, 1.0]));
        assert!(find(
            &dll,
            &boxes(&[
                [7, 7, 9, 9],
                [8, 8, 9, 9],
                [90, 20, 30, 40],
                [130, 20, 30, 40]
            ]),
            (128, 128),
        )
        .is_err());
    }

    #[test]
    fn no_match_is_an_error_rather_than_an_empty_atlas() {
        let dll = vec![0u8; 256];
        assert!(find(&dll, &boxes(&[[10, 20, 30, 40]]), (64, 64)).is_err());
    }
}
