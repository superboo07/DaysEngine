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
//! # A hit map need not agree with the table to the pixel
//!
//! School Days HQ's maps do: every run there anchors on a box that reproduces a
//! record's first four floats byte for byte. Shiny Days' do not. Its `TITLE`
//! regions each sit one pixel left of the sprite they select — boxes at
//! `x = 635` against records at `x = 636`, and three of the five a pixel up as
//! well — so no box anywhere reproduces a record and an exact anchor finds
//! nothing at all.
//!
//! Nothing is recoverable about that: the shipped code never matches a box to a
//! record. It reaches each table by hardcoded address, so the hit maps are free
//! to be as approximate as whoever drew them left them, and the search by
//! content is ours rather than theirs. So it anchors twice: once requiring the
//! box exactly, which is what School Days HQ needs and leaves its results
//! byte-identical, and then, only if that found nothing, allowing each edge to
//! be off by [`SLOP`] pixels. A whole run of records landing within a pixel of
//! a whole run of regions, in order, at a 24-byte stride, is not something
//! unrelated float data does.
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
//! # Some screens' rows cannot be found by their boxes
//!
//! `SAVELOAD` and `REPLAY_PLAYDATA` both list ten slot rows, and both have a
//! record for every one of them — the save/load screen's at `DAT_1004b048`, the
//! play-data list's at `DAT_1004c430` — but the search cannot reach them from
//! the hit map. A row is two hit regions, each covering half of it, while its
//! record is the row entire, so no box reproduces a record and no run anchors.
//! [`table_at`] is the other way in: it anchors a table on records whose
//! **index** is known from the code that reads them, and [`relocate`] puts the
//! records that follow from it back into the atlas.
//!
//! Leaving a screen to the generic search there is not merely imprecise, it can
//! land on another screen's table outright. Shiny Days' `SysMenuSD.dll` holds a
//! second run whose records reproduce `SAVELOAD`'s thirty-two hit boxes to the
//! pixel — it is the play-data list's, which splits a row into the two records
//! the save/load screen only splits into two hit regions — so the search
//! anchored there and drew the rows from it, at a third of their width.
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
    /// For a screen whose leading regions no run reproduces this is
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

/// How far a record's rect may sit from a region's bounding box and still
/// anchor a run, once an exact anchor has been looked for and not found.
///
/// One pixel. That is what Shiny Days' hit maps are out by, and it is small
/// enough that a near match is still a statement about the bytes: a record has
/// to be within a pixel on all four edges, and its neighbours within a pixel of
/// the neighbouring regions, before the run is taken.
pub const SLOP: u32 = 1;

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
    match find_within(dll, boxes, chip, 0) {
        Ok(atlas) => Ok(atlas),
        Err(_) => find_within(dll, boxes, chip, SLOP),
    }
}

/// [`find`], with how far a box may sit from a record and still anchor a run.
///
/// `slop` of zero is the exact search; see [`SLOP`] for why there is another.
fn find_within(dll: &[u8], boxes: &[Rect], chip: (u32, u32), slop: u32) -> Result<Atlas, Error> {
    if boxes.is_empty() {
        return Err(Error::NoAtlas);
    }

    // Walk the regions, taking the longest run of records that reproduces them
    // from wherever the last run stopped. Most screens need one pass; the
    // replay grid needs three.
    let mut segments: Vec<(usize, usize, usize)> = Vec::new();
    let mut at = 0usize;
    while at < boxes.len() {
        let Some((offset, len)) = longest_run(dll, boxes, at, slop) else {
            // No record anywhere reproduces this region's box — a region whose
            // record does not look like its own hit box, which is what the
            // module doc's rows are. Skip it and look for the next segment.
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

    // A region no segment reproduced still has a record — the save/load screen's
    // slot rows are the case this exists for. It is still in the table, at the
    // stride from the nearest segment; it just does not look like its own hit
    // region.
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
    // The gate counts what the search was allowed to accept. With no slop that
    // is the exact matches and nothing changes; with slop it has to be the near
    // ones, or a screen whose every box is a pixel out would be found and then
    // refused for not being exact.
    let credited = widgets
        .iter()
        .zip(boxes)
        .filter(|(w, want)| near(&w.dst, want, slop))
        .count();
    let longest = segments.iter().map(|&(_, _, len)| len).max().unwrap_or(0);
    let believable = credited * 2 >= boxes.len() || longest >= MIN_LONG_RUN;
    if credited < MIN_MATCHES.min(boxes.len()) || !believable {
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
/// The run is anchored on a match no looser than `slop` so that a containment
/// rule can never start one.
fn longest_run(dll: &[u8], boxes: &[Rect], from: usize, slop: u32) -> Option<(usize, usize)> {
    let anchor = boxes[from];
    let mut best: Option<(usize, usize)> = None;
    for found in anchors(dll, &anchor, slop) {
        let mut len = 1usize;
        while from + len < boxes.len() {
            let Some(w) = record(dll, found + len * RECORD) else {
                break;
            };
            if !(fits(&w.dst, &boxes[from + len]) || near(&w.dst, &boxes[from + len], slop)) {
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

/// Every offset in the image holding a record that could be `anchor`'s.
///
/// With no slop this is a byte search for the box's own four floats, which is
/// both the cheapest way to do it and the only way that can start a run. With
/// slop there is no byte pattern to search for, so every aligned offset is read
/// as a record and kept when its rect lands within `slop` of the box on all
/// four edges.
fn anchors(dll: &[u8], anchor: &Rect, slop: u32) -> Vec<usize> {
    if slop == 0 {
        let needle: Vec<u8> = [anchor.x, anchor.y, anchor.width, anchor.height]
            .iter()
            .flat_map(|v| (*v as f32).to_le_bytes())
            .collect();
        let mut out = Vec::new();
        let mut at = 0usize;
        while let Some(found) = find_bytes(dll, &needle, at) {
            at = found + 4;
            if record(dll, found).is_some() {
                out.push(found);
            }
        }
        return out;
    }
    (0..dll.len().saturating_sub(RECORD - 1))
        .step_by(4)
        .filter(|&at| record(dll, at).is_some_and(|w| near(&w.dst, anchor, slop)))
        .collect()
}

/// Whether a record's rect is within `slop` pixels of a box on all four edges.
fn near(rect: &Rect, box_: &Rect, slop: u32) -> bool {
    rect.x.abs_diff(box_.x) <= slop
        && rect.y.abs_diff(box_.y) <= slop
        && rect.width.abs_diff(box_.width) <= slop
        && rect.height.abs_diff(box_.height) <= slop
}

/// Finds a record table by boxes it must reproduce at fixed relative indices.
///
/// [`find`] walks a screen's regions in order and takes whatever run of records
/// reproduces them, which is all a screen with one run per region needs. A
/// screen whose rows are laid out from records the hit map does not reproduce
/// needs the other half: the **index** of each record is known from the code
/// that reads it, and the table has to be anchored well enough for those
/// indices to mean something.
///
/// `anchors` are `(record index, the box that record must account for)` pairs.
/// The return is the byte offset of record 0, or `None` when no position in the
/// image satisfies every pair. Read records out of it with [`record_at`].
///
/// One anchor is matched exactly, because its four floats are the byte pattern
/// the search scans for; the rest are matched the way [`find`] matches a run, so
/// a region whose box is a pixel wider than the sprite it belongs to still
/// anchors. **Which** one that is, the anchors are tried in order to find out:
/// whether a map's author drew a region on its sprite or a pixel around it is
/// their business and not something a caller can know, and `SAVELOAD`'s page
/// buttons are a set where the first box is not the exact one and the fourth
/// is.
///
/// A caller that passes one anchor learns nothing a byte search would not tell
/// it; the point is passing enough of them that the position is unambiguous.
/// `REPLAY_PLAYDATA` shares its first seven records with `REPLAY_HSCENE` byte
/// for byte, so its tabs alone match both tables and the page buttons are what
/// tell them apart.
pub fn table_at(dll: &[u8], anchors: &[(usize, Rect)]) -> Option<usize> {
    anchors
        .iter()
        .find_map(|(first, anchor)| table_anchored_on(dll, anchors, *first, anchor))
}

/// [`table_at`] with the anchor to scan for already chosen.
fn table_anchored_on(
    dll: &[u8],
    anchors: &[(usize, Rect)],
    first: usize,
    anchor: &Rect,
) -> Option<usize> {
    let needle: Vec<u8> = [anchor.x, anchor.y, anchor.width, anchor.height]
        .iter()
        .flat_map(|v| (*v as f32).to_le_bytes())
        .collect();

    let mut at = 0usize;
    while let Some(found) = find_bytes(dll, &needle, at) {
        at = found + 4;
        let Some(base) = found.checked_sub(first * RECORD) else {
            continue;
        };
        if anchors
            .iter()
            .all(|(index, want)| record_at(dll, base, *index).is_some_and(|w| fits(&w.dst, want)))
        {
            return Some(base);
        }
    }
    None
}

/// A stretch of a table the hit map cannot anchor: which region it starts at,
/// which record places that region, and how many regions follow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Band {
    pub region: usize,
    pub record: usize,
    pub count: usize,
}

/// Puts records the hit map cannot reach into an atlas, by the indices the
/// shipped code reads them at.
///
/// `anchors` fix the table — see [`table_at`] for what makes a good set — and
/// each [`Band`] then names a run of records and the regions they place. A
/// record must **cover** the region it is drawn over: the same top edge, no
/// shorter, and no narrower at either end. That is what catches an anchor that
/// landed on a different table with the same leading bytes, and it is a test a
/// row passes and a half-row does not.
///
/// Returns the table's base offset, having written [`Atlas::offset`] too, or
/// `None` with `atlas` untouched when nothing anchors or a record does not
/// cover its region. Nothing here fills [`Atlas::extras`]: which records follow
/// a table and what they mean is per-screen knowledge, so the caller does that.
pub fn relocate(
    atlas: &mut Atlas,
    dll: &[u8],
    boxes: &[Rect],
    anchors: &[(usize, Rect)],
    bands: &[Band],
) -> Option<usize> {
    let Some(base) = table_at(dll, anchors) else {
        log::warn!(
            "no table in the image satisfies all {} anchors",
            anchors.len()
        );
        return None;
    };

    let mut placed: Vec<(usize, Widget)> = Vec::new();
    for band in bands {
        for i in 0..band.count {
            let region = band.region + i;
            let (Some(widget), Some(box_)) = (
                record_at(dll, base, band.record + i),
                boxes.get(region).filter(|_| region < atlas.widgets.len()),
            ) else {
                log::warn!(
                    "no record {} to place region {}",
                    band.record + i,
                    region + 1
                );
                return None;
            };
            if !spans(&widget, box_) {
                log::warn!(
                    "record {} at {:?} does not cover region {}'s box {box_:?}",
                    band.record + i,
                    widget.dst,
                    region + 1
                );
                return None;
            }
            placed.push((region, widget));
        }
    }

    for (region, widget) in placed {
        atlas.widgets[region] = widget;
    }
    atlas.offset = base;
    Some(base)
}

/// Whether a record covers the hit region it is drawn over: the same top edge,
/// and no narrower at either end.
///
/// Nothing is asked of the height, because a region's bounding box is the
/// extent of that region's **pixels** and those need not stay inside the row
/// they belong to: `SAVELOAD`'s ten rows are 31 tall in `SysMenuSDHQ.dll` and
/// their boxes run 199, because the map carries each row's index over the rows
/// below it as well. The width is the half a band covers, which is exactly what
/// this is here to catch — a record that covers half a row is not the row.
pub fn spans(record: &Widget, region: &Rect) -> bool {
    record.dst.y == region.y
        && record.dst.x <= region.x
        && record.dst.x + record.dst.width >= region.x + region.width
}

/// The layout every stored rect is authored in, so a run that leaves it is not
/// one of these tables. See the module doc's note on native resolution.
const LAYOUT: (u32, u32) = (800, 450);

/// Finds a table of `count` records by the **shape** of the records themselves,
/// for a table no hit map can anchor.
///
/// [`table_at`] needs a box to match a record against. A screen whose widgets
/// are rectangles rather than map regions has none — Shiny Days' Option pages
/// ship a hit map covering only the frame — so the run has to be recognised by
/// how it is built instead: a row of equally spaced buttons of one size, a
/// column of tracks at one pitch, pairs sharing a baseline.
///
/// **This search is ours, not theirs.** The shipped code reaches every one of
/// these tables by hardcoded address and never matches anything; nothing in the
/// data marks a table's start. So the result is only believed when it is
/// unambiguous, and two rules do the discriminating:
///
/// * every record of the run must be a rect inside the [`LAYOUT`] the tables
///   are authored in, which is what rejects a window straddling two unrelated
///   tables, and
/// * exactly one position in the image may match. A run that is itself preceded
///   by a matching run is dropped first, because these tables are two runs of
///   the same geometry at different `src_y` and it is the first that is the
///   table; anything still ambiguous after that returns `None` rather than a
///   guess.
pub fn table_by_shape(
    dll: &[u8],
    count: usize,
    shape: impl Fn(&[Widget]) -> bool,
) -> Option<usize> {
    if count == 0 {
        return None;
    }
    let span = count.checked_mul(RECORD)?;
    let mut run: Vec<Widget> = Vec::with_capacity(count);
    let mut hits: Vec<usize> = Vec::new();
    for base in 0..=dll.len().checked_sub(span)? {
        run.clear();
        if (0..count).any(|i| match record(dll, base + i * RECORD) {
            Some(w) if in_layout(&w.dst) => {
                run.push(w);
                false
            }
            _ => true,
        }) {
            continue;
        }
        if shape(&run) {
            hits.push(base);
        }
    }
    let mut first = hits
        .iter()
        .copied()
        .filter(|base| base.checked_sub(span).is_none_or(|p| !hits.contains(&p)));
    match (first.next(), first.next()) {
        (Some(base), None) => Some(base),
        _ => None,
    }
}

/// Whether a rect lies inside the layout the tables are authored in.
fn in_layout(rect: &Rect) -> bool {
    rect.x + rect.width <= LAYOUT.0 && rect.y + rect.height <= LAYOUT.1
}

/// The containment test both page tables are scanned with.
///
/// `FUN_1000bb10` and `FUN_1002cf00` — the Option and Replay hit tests that run
/// when the screen's hit map misses — compare a point against a record the same
/// way: `rec.x < x <= rec.x + rec.w`, and the same in `y`. Half-open at the low
/// edge and closed at the high one, which is the shipped comparison rather than
/// a tidied version of it.
pub fn contains(rect: &Rect, x: u32, y: u32) -> bool {
    x > rect.x && x <= rect.x + rect.width && y > rect.y && y <= rect.y + rect.height
}

/// One record of a table whose base offset is already known.
pub fn record_at(dll: &[u8], base: usize, index: usize) -> Option<Widget> {
    record(dll, base.checked_add(index.checked_mul(RECORD)?)?)
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

    /// A row is one record behind two hit regions covering half of it each, so
    /// the record is wider than either box and may be shorter than it: the hit
    /// map's bounding box is the extent of the region's pixels and `SAVELOAD`'s
    /// run past the row they belong to. What must not pass is the half-row —
    /// the record of another screen's table that splits the row where this one
    /// only splits the hit map.
    #[test]
    fn a_record_covers_a_band_when_it_spans_it_however_tall_the_box_is() {
        let row = Widget {
            dst: Rect {
                x: 0,
                y: 96,
                width: 799,
                height: 31,
            },
            src_x: 1,
            src_y: 122,
        };
        let left = boxes(&[[0, 96, 329, 199]]);
        let right = boxes(&[[328, 96, 472, 199]]);
        let panel = Widget {
            dst: Rect {
                x: 328,
                y: 96,
                width: 472,
                height: 97,
            },
            ..row
        };
        assert!(spans(&row, &left[0]));
        assert!(spans(&panel, &right[0]));

        // The half-row from the other screen's table covers the left band and
        // nothing else, which is what catches an anchor that landed on it.
        let half = Widget {
            dst: left[0],
            ..row
        };
        assert!(!spans(&half, &right[0]));
        // A record on the wrong row is not this row's however wide it is.
        assert!(!spans(&row, &boxes(&[[0, 129, 329, 31]])[0]));
    }

    /// `table_at` scans for whichever anchor reproduces its record exactly, so a
    /// set whose first box is a pixel out still finds the table.
    #[test]
    fn an_anchor_set_need_not_lead_with_its_exact_box() {
        let mut dll = vec![0xaau8; 24];
        dll.extend(rec([10.0, 20.0, 30.0, 40.0, 0.0, 0.0]));
        dll.extend(rec([50.0, 20.0, 30.0, 40.0, 0.0, 0.0]));
        let want = boxes(&[[10, 20, 29, 40], [50, 20, 30, 40]]);
        let anchors = [(1, want[0]), (2, want[1])];
        assert_eq!(table_at(&dll, &anchors), Some(0));
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

    /// Shiny Days' `TITLE` regions all sit a pixel left of the records they
    /// select, and three of the five a pixel up as well, so an exact anchor
    /// finds nothing anywhere in the image. The slop pass is what reaches the
    /// table; these are that screen's five boxes and records.
    #[test]
    fn a_hit_map_a_pixel_out_still_reaches_its_table() {
        let mut dll = vec![0xaau8; 64];
        for (y, src_y) in [
            (233.0, 1.0),
            (268.0, 30.0),
            (301.0, 59.0),
            (337.0, 88.0),
            (373.0, 117.0),
        ] {
            dll.extend(rec([636.0, y, 165.0, 28.0, 1.0, src_y]));
        }
        dll.extend(vec![0u8; 32]);
        let a = find(
            &dll,
            &boxes(&[
                [635, 232, 165, 28],
                [635, 267, 165, 28],
                [635, 301, 165, 28],
                [635, 336, 165, 28],
                [635, 372, 165, 28],
            ]),
            (512, 256),
        )
        .unwrap();
        assert_eq!(a.offset, 64);
        // The table is what drawing uses, not the box it was reached through.
        assert_eq!(a.widgets[0].dst.x, 636);
        assert_eq!(a.widgets[4].src_y, 117);
        // Nothing was exact, which is the whole point of the screen.
        assert_eq!(a.matched, 0);
    }

    /// Two pixels is not a pixel: the slop is a tolerance for a hand-drawn hit
    /// map, not a licence to match whatever is nearby.
    #[test]
    fn a_hit_map_further_out_than_the_slop_finds_nothing() {
        let mut dll = vec![0xaau8; 64];
        for y in [233.0f32, 268.0, 301.0] {
            dll.extend(rec([636.0, y, 165.0, 28.0, 1.0, y]));
        }
        assert!(find(
            &dll,
            &boxes(&[
                [633, 232, 165, 28],
                [633, 267, 165, 28],
                [633, 301, 165, 28]
            ]),
            (512, 256),
        )
        .is_err());
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
        // Regions 1 and 2 reproduce no record anywhere, so the
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

    /// A pair of records on one baseline, for the shape tests below.
    fn pair(y: f32) -> Vec<u8> {
        let mut v = rec([10.0, y, 30.0, 20.0, 1.0, 1.0]);
        v.extend(rec([50.0, y, 30.0, 20.0, 32.0, 1.0]));
        v
    }

    /// Stands in for a real shape: a left-to-right pair of 30x20 buttons on one
    /// baseline. The real shapes are seven and fourteen records and constrain
    /// size, pitch and sheet packing; a toy predicate has to say enough to tell
    /// a run start from a window straddling two of them, which is why this one
    /// pins the size as well as the line.
    fn on_one_line(run: &[Widget]) -> bool {
        run.iter()
            .all(|w| (w.dst.y, w.dst.width, w.dst.height) == (run[0].dst.y, 30, 20))
            && run[0].dst.x < run[1].dst.x
    }

    #[test]
    fn a_shape_matched_twice_over_is_refused_rather_than_guessed() {
        // Two runs that are not adjacent: nothing says which is the table, so
        // the honest answer is that it was not found.
        let mut dll = pair(40.0);
        dll.extend(vec![0u8; 64]);
        dll.extend(pair(40.0));
        assert_eq!(table_by_shape(&dll, 2, on_one_line), None);
    }

    #[test]
    fn the_first_of_two_adjacent_runs_is_the_table() {
        // These tables are the same geometry twice at different `src_y`, and
        // it is the first run that the code indexes.
        let mut dll = vec![0u8; 8];
        dll.extend(pair(40.0));
        dll.extend(pair(40.0));
        assert_eq!(table_by_shape(&dll, 2, on_one_line), Some(8));
    }

    #[test]
    fn a_run_that_leaves_the_layout_is_not_one_of_these_tables() {
        // `SysMenuSD.dll` really does hold a window like this — a straddle of
        // two unrelated tables whose rects run off the bottom of the 800x450
        // the records are authored in. Without the bound it matches.
        let mut dll = rec([10.0, 453.0, 30.0, 288.0, 1.0, 1.0]);
        dll.extend(rec([50.0, 453.0, 30.0, 288.0, 32.0, 1.0]));
        assert_eq!(table_by_shape(&dll, 2, on_one_line), None);
    }
}
