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
//! the user's own `.CMAP`, search the DLL's data sections for records whose
//! destination rects reproduce them at a 24-byte stride, and read the source
//! coordinates out of the match. Those records are not always one run — see
//! below.
//!
//! That is self-validating. It is also tolerant in one direction and strict in
//! the other: a region's bounding box can legitimately differ from its sprite
//! rect — regions that abut each other have their boxes clipped by the
//! neighbour — so a table is accepted on a scored match rather than a perfect
//! one, and [`Atlas::matched`] reports how many regions agreed. What is not
//! tolerated is a weak match: below the threshold we report that the table was
//! not found rather than drawing sprites from the wrong offsets.
//!
//! # A table is not always one run
//!
//! `TITLE` and the three `OPTION` screens keep one record per region, in region
//! order, back to back. The replay grid does not: its nineteen regions are
//! three separate stretches of one larger table — the two tab headers and the
//! back button, then the four page buttons eight records later, then the twelve
//! thumbnails eight records after that — because the records in between are the
//! other states of the same widgets. Insisting on one run there does not fail
//! cleanly; it lands on a stretch that reproduces fifteen of the nineteen boxes
//! and draws every sprite from the wrong offset.
//!
//! So the search is **segmented**: it takes the longest run it can anchor at
//! region 1, continues from wherever that stops, and repeats. A screen whose
//! table really is one run comes out as a single segment, which is the old
//! behaviour exactly. [`Atlas::segments`] reports how many it took, because two
//! is a fact about the screen and twelve would be a sign the search is fitting
//! noise.
//!
//! # Some screens still have no table
//!
//! `SAVELOAD` and `REPLAY_PLAYDATA` lay their slot rows out with a loop at
//! runtime — ten rows at a 33px pitch — so those rects exist nowhere in the
//! binary. Both need their layout reproduced in code; the search says so rather
//! than guessing.
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
    /// **Best effort.** Nothing in the data marks where a table ends — the
    /// shipped code reaches these by fixed address, so how many there are and
    /// what each one means is per-screen knowledge rather than something the
    /// bytes carry. The run starts after the last segment and is cut at the
    /// first record that cannot be a sprite for this sheet, so trailing entries
    /// may belong to another screen.
    pub extras: Vec<Widget>,
    /// Byte offset of region 1's record in the DLL image, for diagnostics.
    ///
    /// For a screen whose leading regions are laid out at runtime this is
    /// extrapolated back from the first segment, so it is where the record
    /// *would* be rather than somewhere a match was seen.
    pub offset: usize,
    /// Where each stretch of consecutive regions was found, as
    /// `(first region index, byte offset, how many regions)`.
    ///
    /// One entry means the table is a single run. More means the screen's
    /// records are interleaved with other states of the same widgets, which is
    /// how the replay grid is laid out.
    pub segments: Vec<(usize, usize, usize)>,
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

/// A run this long is believable on its own, whatever proportion of the
/// screen's regions it covers.
///
/// The save/load screen is why: only twelve of its thirty-two regions can ever
/// match a record, because ten rows are one full-width sprite behind two
/// half-width hit regions each, and the other ten regions are the comment
/// panels, which are taller than the rows they sit on. Twelve consecutive
/// records reproducing twelve consecutive regions exactly is not a
/// coincidence — sixteen bytes each, in order — so the table is read from that
/// run and the rest filled in at its stride.
const MIN_LONG_RUN: usize = 8;

/// Fewest regions a segment should average before the split looks like the
/// search fitting noise rather than reading a table.
///
/// The replay grid is nineteen regions in three segments; a screen that came
/// back as one segment per region would be matching individual records
/// anywhere in the DLL, which proves nothing.
const MIN_SEGMENT_REGIONS: usize = 3;

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

    // Walk the regions, taking the longest run of records that reproduces them
    // from wherever the last run stopped. Most screens need one pass; the
    // replay grid needs three.
    let mut segments: Vec<(usize, usize, usize)> = Vec::new();
    let mut at = 0usize;
    while at < boxes.len() {
        let Some((offset, len)) = longest_run(dll, boxes, at) else {
            // No record anywhere reproduces this region's box. That is a region
            // laid out at runtime; skip it and look for the next segment.
            at += 1;
            continue;
        };
        segments.push((at, offset, len));
        at += len;
    }

    // Segments must not be so short that the search is really matching
    // coincidences rather than reading a table.
    if segments.is_empty() || segments.len() > boxes.len() / MIN_SEGMENT_REGIONS + 1 {
        return Err(Error::NoAtlas);
    }

    // The table, not the hit map, is what drawing uses: a clipped bounding box
    // would place the sprite a pixel or two off, and the table is the rect the
    // original blits with.
    let mut slots: Vec<Option<Widget>> = vec![None; boxes.len()];
    for &(first, offset, len) in &segments {
        for i in 0..len {
            slots[first + i] = record(dll, offset + i * RECORD);
        }
    }

    // A region no segment reproduced is one laid out at runtime — the save/load
    // screen's slot rows are the case this exists for. Its record is still in
    // the table, at the stride from the nearest segment; it just does not look
    // like its own hit region.
    for (i, slot) in slots.iter_mut().enumerate() {
        if slot.is_some() {
            continue;
        }
        let near = segments
            .iter()
            .min_by_key(|(first, _, len)| {
                i.abs_diff(if i < *first { *first } else { first + len - 1 })
            })
            .expect("checked non-empty above");
        let (first, offset, _) = *near;
        let at = if i >= first {
            offset.checked_add((i - first) * RECORD)
        } else {
            offset.checked_sub((first - i) * RECORD)
        };
        *slot = at.and_then(|at| record(dll, at));
    }

    let widgets: Vec<Widget> = slots
        .into_iter()
        .collect::<Option<_>>()
        .ok_or(Error::NoAtlas)?;
    let matched = widgets
        .iter()
        .zip(boxes)
        .filter(|(w, want)| w.dst == **want)
        .count();
    let longest = segments.iter().map(|&(_, _, len)| len).max().unwrap_or(0);
    let believable = matched * 2 >= boxes.len() || longest >= MIN_LONG_RUN;
    if matched < MIN_MATCHES.min(boxes.len()) || !believable {
        return Err(Error::NoAtlas);
    }

    // Alternate states follow the last segment, which is where the screen's own
    // records carry on past the ones the hit map names.
    let (_, last_offset, last_len) = *segments.last().expect("covered every region");
    let mut extras = Vec::new();
    let mut p = last_offset + last_len * RECORD;
    while extras.len() < MAX_EXTRAS {
        let Some(w) = record(dll, p) else { break };
        // A sprite for this screen has to fit inside this screen's sheet.
        if w.src_x + w.dst.width > chip.0 || w.src_y + w.dst.height > chip.1 {
            break;
        }
        extras.push(w);
        p += RECORD;
    }

    let (first, first_offset, _) = segments[0];
    Ok(Atlas {
        offset: first_offset.saturating_sub(first * RECORD),
        widgets,
        extras,
        segments,
        matched,
    })
}

/// The longest run of consecutive records reproducing `boxes[from..]`.
///
/// Returns where it starts and how many regions it covers. A record is taken as
/// this region's when its rect is the box exactly, or when the box merely
/// contains it — a hit map's region can be clipped by a neighbour, or run wider
/// than the sprite it belongs to, and neither means the record is the wrong one.
/// The run is anchored on an exact match so that a containment rule can never
/// start one.
fn longest_run(dll: &[u8], boxes: &[Rect], from: usize) -> Option<(usize, usize)> {
    let anchor = boxes[from];
    let needle: Vec<u8> = [anchor.x, anchor.y, anchor.width, anchor.height]
        .iter()
        .flat_map(|v| (*v as f32).to_le_bytes())
        .collect();

    let mut best: Option<(usize, usize)> = None;
    let mut at = 0usize;
    while let Some(found) = find_bytes(dll, &needle, at) {
        at = found + 4;
        if record(dll, found).is_none() {
            continue;
        }
        let mut len = 1usize;
        while from + len < boxes.len() {
            let Some(w) = record(dll, found + len * RECORD) else {
                break;
            };
            if !fits(&w.dst, &boxes[from + len]) {
                break;
            }
            len += 1;
        }
        if best.is_none_or(|(_, b)| len > b) {
            best = Some((found, len));
        }
    }
    best
}

/// Whether a record's rect can be the sprite for a region with this box.
fn fits(rect: &Rect, box_: &Rect) -> bool {
    rect == box_
        || (rect.x >= box_.x
            && rect.y >= box_.y
            && rect.x + rect.width <= box_.x + box_.width.max(rect.width)
            && rect.y + rect.height <= box_.y + box_.height.max(rect.height)
            && rect.x < box_.x + box_.width
            && rect.y < box_.y + box_.height)
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
