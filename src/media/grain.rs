//! A very light grain, laid over the frame after it has been scaled.
//!
//! # Why there is grain at all
//!
//! `deblock` and `gradfun` in the pre-scale chain ([`super::filter`]) take out
//! most of what the encoder left, but not all of it: a band the debander judged
//! too wide to be an artifact stays, and the upscale adds contouring of its own
//! by interpolating between levels that were already quantised. Both are
//! *flat areas with a visible step between them*, and a step is only visible
//! because it is clean. A dither under a level of amplitude breaks the step
//! into noise the eye integrates back to the same average, which is the whole
//! trick, and it is why every encoder and every renderer that fills a gradient
//! dithers it.
//!
//! # Why it is ours and not libavfilter's
//!
//! `noise` is the obvious answer and is what the post-scale chain would hold.
//! It is a poor fit here, for three measured reasons — all of them about the
//! frame at this point being **packed RGBA**, because that is what the scaler
//! produced and what the window wants:
//!
//! * libavfilter will not run `noise` on packed RGBA, so it inserts a
//!   conversion to planar `gbrap` and another back: two passes over eight
//!   megabytes a frame at 1080p, on top of the filter.
//! * Its per-component patterns are independent even given one seed, so what
//!   lands on the picture is coloured noise rather than the neutral grain a
//!   dither wants. `alls` is worse: in `gbrap` the fourth component is the
//!   alpha plane, and noising it makes the frame slightly translucent — a
//!   render with `alls=3` comes back with alpha of both 254 and 255.
//! * A frame is 41 milliseconds and the round trip is not free.
//!
//! What is wanted is one offset per pixel, the same on all three colours,
//! nothing on alpha. That is this module, and it is a table lookup and an add.
//!
//! The chain in `[Video] FiltersAfterScale` is still there for anything else
//! that belongs after the scale; this is only the default, and
//! `[Video] Grain = 0` turns it off.

/// Edge of the tile, in pixels. A power of two, so the wrap is a mask.
const TILE: usize = 256;

/// The fewest rows worth giving a thread of its own.
const BAND: usize = 64;

/// The most a grain can move a channel, so a settings file cannot ask for
/// something that is no longer a dither.
pub const MAX: u8 = 16;

/// A tile of per-pixel offsets, and how far it is shifted per frame.
///
/// The tile is built once. It is 64KB, which stays in L2 while a frame is
/// written, and at 256 pixels the repeat is invisible under an amplitude this
/// small — what would give it away is the pattern sitting still, and it does
/// not: every frame shifts it somewhere else, which is also what makes the
/// grain temporal rather than a film of dirt on the screen.
pub struct Grain {
    amount: u8,
    tile: Vec<i8>,
    /// [`Grain::tile`] rotated for the frame being written and expanded to one
    /// offset per byte, and how far it is rotated. Kept between frames, so the
    /// rotation is one pass over 256KB rather than a wrap computed per pixel.
    rotated: Vec<i8>,
    rotated_by: Option<usize>,
}

impl Grain {
    /// Builds the tile for an amplitude of `amount` levels, `0` for no grain.
    ///
    /// Amplitudes above [`MAX`] are clamped to it.
    pub fn new(amount: u8) -> Grain {
        let amount = amount.min(MAX);
        if amount == 0 {
            return Grain {
                amount,
                tile: Vec::new(),
                rotated: Vec::new(),
                rotated_by: None,
            };
        }
        let span = u32::from(amount) * 2 + 1;
        let mut state = 0x2545_f491_4f6c_dd1d_u64;
        let mut next = move || {
            // xorshift64*, which is a few instructions and more than random
            // enough for a dither.
            state ^= state >> 12;
            state ^= state << 25;
            state ^= state >> 27;
            state.wrapping_mul(0x2545_f491_4f6c_dd1d)
        };
        let tile = (0..TILE * TILE)
            .map(|_| {
                let v = (next() >> 33) as u32 % span;
                (v as i32 - i32::from(amount)) as i8
            })
            .collect();
        Grain {
            amount,
            tile,
            rotated: Vec::with_capacity(TILE * TILE * 4),
            rotated_by: None,
        }
    }

    /// Whether this grain does anything, so a caller can skip the pass.
    pub fn is_visible(&self) -> bool {
        self.amount > 0 && !self.tile.is_empty()
    }

    /// Lays the grain over one packed RGBA frame of `width` pixels a row.
    ///
    /// `frame` is the frame's index in the clip, and is what moves the tile:
    /// the same frame of the same clip always comes out the same, so a paused
    /// picture is a still picture and a seek back shows what it showed before.
    /// Alpha is not touched.
    ///
    /// # Speed
    ///
    /// This runs over every byte of a frame — two megapixels at 1080p, which
    /// is the size this engine is expected to run at — inside the 41
    /// milliseconds the frame has, so the shape of the inner loop is the whole
    /// of the design — and the shape that matters is **flat bytes**. Both
    /// formulations that treat a pixel as a pixel — a modulo and a
    /// `(v as i32 + o).clamp(0, 255)` per channel, or a rotated tile and three
    /// saturating adds inside a `[u8; 4]` — measured about 12ms a frame at
    /// 1080p, which is a quarter of the frame. Expanding the offsets to one per
    /// byte, alpha's being zero, leaves the loop a single saturating add down
    /// two contiguous runs, which vectorises: the same work measures about
    /// 2.5ms.
    ///
    /// [`u8::saturating_add_signed`] is the other half of it. It is one
    /// instruction, and it is exactly the clamp that is wanted at both ends.
    ///
    /// The rest is threads. A band of rows is written by one worker and read by
    /// nobody, so they need nothing from each other: 2.5ms becomes 1.25 at
    /// 1080p, and 8ms becomes 1.7 at 4K, which nothing here is aiming at but
    /// which is where a pass over every byte stops being free.
    pub fn apply(&mut self, rgba: &mut [u8], width: usize, frame: u64) {
        if !self.is_visible() || width == 0 {
            return;
        }
        // Two odd multipliers, so consecutive frames land nowhere near each
        // other and the sequence does not repeat inside a clip.
        let shift_x = (frame.wrapping_mul(0x9e37_79b9) % TILE as u64) as usize;
        let shift_y = (frame.wrapping_mul(0x85eb_ca6b) % TILE as u64) as usize;
        self.rotate(shift_x);
        let rotated = &self.rotated;
        let stride = width * 4;
        // A band of rows is written by one thread and read by nobody, so the
        // whole of the synchronisation is handing them out.
        let rows = rgba.len() / stride.max(1);
        let workers = std::thread::available_parallelism()
            .map_or(1, std::num::NonZeroUsize::get)
            .min(rows.div_ceil(BAND).max(1));
        let band = rows.div_ceil(workers).max(1) * stride;
        if workers <= 1 {
            Grain::band(rgba, stride, rotated, shift_y);
            return;
        }
        std::thread::scope(|scope| {
            for (index, chunk) in rgba.chunks_mut(band).enumerate() {
                let first = index * (band / stride.max(1));
                scope.spawn(move || Grain::band(chunk, stride, rotated, shift_y + first));
            }
        });
    }

    /// One band of rows: the pass itself, and the only place bytes are written.
    ///
    /// `shift_y` is the tile row the band's first row takes, so a band knows
    /// where it is without being told which frame it is part of.
    fn band(rgba: &mut [u8], stride: usize, rotated: &[i8], shift_y: usize) {
        for (y, row) in rgba.chunks_exact_mut(stride).enumerate() {
            let line = ((y + shift_y) % TILE) * TILE * 4;
            let offsets = &rotated[line..line + TILE * 4];
            // A row is as many whole passes over the tile row as it takes,
            // plus whatever is left. Both sides are flat bytes and nothing
            // wraps inside a pass, so the loop is a straight saturating add
            // down two contiguous runs — which is what vectorises.
            for span in row.chunks_mut(TILE * 4) {
                for (byte, offset) in span.iter_mut().zip(offsets) {
                    *byte = byte.saturating_add_signed(*offset);
                }
            }
        }
    }

    /// Rotates the tile left by `shift` columns, which is how the pattern moves
    /// horizontally without the inner loop having to wrap.
    fn rotate(&mut self, shift: usize) {
        if self.rotated_by == Some(shift) && self.rotated.len() == self.tile.len() * 4 {
            return;
        }
        self.rotated.clear();
        for line in self.tile.as_chunks::<TILE>().0 {
            // Expanded to one offset per *byte* of a pixel, with a zero in the
            // alpha slot: that is what lets the pass above run down the row as
            // flat bytes rather than picking the three colours out of each
            // pixel, and alpha comes out untouched by arithmetic rather than
            // by being skipped.
            for offset in line[shift..].iter().chain(&line[..shift]) {
                self.rotated
                    .extend_from_slice(&[*offset, *offset, *offset, 0]);
            }
        }
        self.rotated_by = Some(shift);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The grain is neutral and does not touch alpha: one offset per pixel,
    /// the same on all three colours, which is what makes it a dither rather
    /// than coloured noise.
    #[test]
    fn one_offset_per_pixel_and_nothing_on_alpha() {
        let mut grain = Grain::new(3);
        let flat = [128u8, 128, 128, 255];
        let mut frame: Vec<u8> = flat.iter().copied().cycle().take(64 * 8 * 4).collect();
        grain.apply(&mut frame, 64, 7);
        for px in frame.as_chunks::<4>().0 {
            assert_eq!(px[3], 255, "alpha was moved");
            assert_eq!(px[0], px[1], "the grain is not neutral");
            assert_eq!(px[1], px[2], "the grain is not neutral");
            assert!(
                px[0].abs_diff(128) <= 3,
                "{} is outside the amplitude",
                px[0]
            );
        }
    }

    /// It is a dither, so it has to average out: over a flat field the mean
    /// stays where it was, and every one of the seven offsets is used.
    #[test]
    fn the_offsets_are_centred_on_no_change() {
        let mut grain = Grain::new(3);
        let mut frame: Vec<u8> = [128u8, 128, 128, 255]
            .iter()
            .copied()
            .cycle()
            .take(256 * 256 * 4)
            .collect();
        grain.apply(&mut frame, 256, 0);
        let total: i64 = frame
            .as_chunks::<4>()
            .0
            .iter()
            .map(|p| i64::from(p[0]))
            .sum();
        let mean = total as f64 / (256.0 * 256.0);
        assert!((mean - 128.0).abs() < 0.05, "the mean moved to {mean}");
        let used: std::collections::BTreeSet<u8> =
            frame.as_chunks::<4>().0.iter().map(|p| p[0]).collect();
        assert_eq!(used.len(), 7, "not every offset in -3..=3 was used");
    }

    /// A frame index moves the tile, so the pattern is not a fixed film of
    /// dirt — and the same index brings the same frame back, so a pause holds
    /// a still picture.
    #[test]
    fn the_pattern_moves_with_the_frame_but_repeats_for_one() {
        let mut grain = Grain::new(2);
        let flat: Vec<u8> = [100u8, 100, 100, 255]
            .iter()
            .copied()
            .cycle()
            .take(64 * 64 * 4)
            .collect();
        let mut at = |frame: u64| {
            let mut f = flat.clone();
            grain.apply(&mut f, 64, frame);
            f
        };
        assert_ne!(at(0), at(1), "consecutive frames got the same pattern");
        assert_eq!(at(9), at(9), "the same frame came out twice over");
    }

    /// The bands are threads, and a thread has to know which rows it was given:
    /// the tile repeats every 256 rows, so a frame twice that tall has to come
    /// out the same twice over however the rows were handed out.
    #[test]
    fn the_bands_agree_about_where_they_are() {
        let mut grain = Grain::new(3);
        let (w, h) = (32usize, TILE * 2);
        let mut frame: Vec<u8> = [90u8, 90, 90, 255]
            .iter()
            .copied()
            .cycle()
            .take(w * h * 4)
            .collect();
        grain.apply(&mut frame, w, 5);
        let row = |y: usize| &frame[y * w * 4..(y + 1) * w * 4];
        for y in 0..TILE {
            assert_eq!(row(y), row(y + TILE), "row {y} and row {} differ", y + TILE);
        }
        // The tile repeating every 256 rows is not enough on its own: a band
        // that started every one of its own bands from the top would repeat
        // every band instead, and 256 is a multiple of that. So the pattern
        // must *not* repeat at the band boundary.
        assert!(h.div_ceil(BAND) > 1, "the pass was not threaded at all");
        assert_ne!(
            row(0),
            row(BAND),
            "the pattern repeats every band: the bands all started from the top"
        );
    }

    /// Zero is off, and it is off without touching a byte.
    #[test]
    fn no_amplitude_is_no_pass() {
        let mut grain = Grain::new(0);
        assert!(!grain.is_visible());
        let mut frame = vec![7u8; 16 * 4];
        grain.apply(&mut frame, 16, 3);
        assert!(frame.iter().all(|b| *b == 7));
    }
}
