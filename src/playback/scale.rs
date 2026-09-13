//! Resampling, for getting the game's art onto a window that is not its size.
//!
//! # Why this is not the original's scaler
//!
//! Everything else in this engine is a recovered behaviour. This is not: it is
//! a deliberate, documented departure. The original hands its surfaces to
//! Direct3D and takes whatever the driver's bilinear filter gives —
//! `FUN_0044a3d0` sets `D3DSAMP_MAGFILTER` and `D3DSAMP_MINFILTER` to
//! `D3DTEXF_LINEAR` on all eight sampler stages, with `D3DSAMP_ADDRESSU`/`V`
//! clamped — which at 800x450 stretched onto a modern panel is a soft, slightly
//! aliased picture that varies with the driver. There is nothing to recover
//! here that would be worth reproducing, and reproducing a driver's filter
//! faithfully is not possible anyway.
//!
//! What is recovered is that the picture is *filtered*, and that its edges are
//! clamped. This module is both of those; only the kernel is ours.
//!
//! So this module scales with a **cubic B-spline** instead. Nothing about the
//! game's timing, layout or art depends on the filter, so the choice changes
//! how the frame looks and nothing about how it behaves.
//!
//! # The filter
//!
//! The Mitchell-Netravali family, at `B = 1, C = 0`:
//!
//! ```text
//!            (12 - 9B - 6C)|x|^3 + (-18 + 12B + 6C)|x|^2 + (6 - 2B)
//! k(x) =     ------------------------------------------------------   |x| < 1
//!                                     6
//!
//!        (-B - 6C)|x|^3 + (6B + 30C)|x|^2 + (-12B - 48C)|x| + (8B + 24C)
//!        ---------------------------------------------------------------  1 <= |x| < 2
//!                                     6
//! ```
//!
//! which reduces to `(3|x|^3 - 6|x|^2 + 4) / 6` and
//! `(-|x|^3 + 6|x|^2 - 12|x| + 8) / 6`. It is the smoothest member of the
//! family: the kernel is everywhere non-negative, so the result cannot
//! overshoot and there is no ringing or haloing at all. The cost is softness —
//! a B-spline does not interpolate, it approximates, so even a 1:1 pass would
//! blur if it were allowed to run. [`Scaler::resample`] returns the source
//! untouched when nothing needs scaling, so that never happens.
//!
//! # How it runs
//!
//! Separably, and with the weights cached. A frame's scale factors only change
//! when the window does, so the per-axis weight tables are built once and then
//! reused: each output pixel is a short gather over the source, at most four
//! taps when upscaling and more when the picture is being shrunk. Horizontal
//! first into a scratch of f32, then vertical down it.
//!
//! Downscaling widens the kernel in source space by the scale factor, so
//! shrinking averages over every source pixel that lands in the footprint
//! rather than point-sampling four of them and aliasing.
//!
//! # Keeping up with the picture
//!
//! This runs on every movie frame, so it has a deadline. The engine's clock is
//! 24 fps — 41ms a frame for decode, mixing, this, the upload and the present —
//! and a frame that misses it judders. A full-screen frame is the hard case:
//! 800x452 onto a 4K panel is 8.3 million output pixels, each four taps in each
//! direction, which is a hundred and thirty million multiply-adds that have to
//! happen between one frame and the next.
//!
//! Four things are what make it fit. Only the last of them is about the filter.
//!
//! **Threads.** A band of output rows reads the source and writes its own rows
//! of the frame and touches nothing else, so bands go to a scoped thread each
//! and the picture is resampled on every core the machine has. This is the only
//! parallel code in the engine. It is parallel over the data rather than over
//! the work, so the whole of the synchronisation is handing out the next band —
//! and a resample small enough for one worker takes the loop inline instead,
//! because spawning a thread to give work to yourself is pure overhead.
//!
//! **Bands.** [`BAND`] output rows at a time through both passes, rather than
//! each pass over the whole frame. That is what gives the threads something
//! divisible, and it bounds the f32 scratch between the passes at a few hundred
//! kilobytes rather than the 27MB a full 4K frame would need. Neighbouring
//! bands overlap by the kernel's support and filter those few rows twice, which
//! is cheaper than any way of sharing them would be.
//!
//! **The buffers are kept.** [`Scaler`] owns the output, so a 33MB 4K frame is
//! not allocated and zeroed again every time, and [`Scaler::resample`] hands
//! back a slice of it rather than a `Vec`.
//!
//! **Four taps go straight through.** Trimming zero-weight taps makes every
//! interior pixel of an upscale exactly four, and [`column`] takes that case
//! with the four source values and the sum in registers — one load per tap and
//! a store, instead of an accumulator read and written once per tap. What a
//! resample is bound by is that traffic, not the arithmetic.
//!
//! Two of these were arrived at by measuring rather than by reasoning, and the
//! reasoning would have got them wrong: band-tiling on its own, without the
//! threads, measured no faster at all, and the single largest saving in the
//! whole module was the float-to-byte conversion at the end — see [`to_byte`].

/// One output pixel's taps along one axis.
struct Taps {
    /// Index of the first source pixel this output pixel reads. The whole run
    /// `start .. start + weights.len()` is inside the source.
    start: usize,
    /// Weights, summing to 1.
    weights: Vec<f32>,
}

/// How many output rows one band covers.
///
/// Small enough that a band's scratch — the source rows it reads, filtered
/// horizontally — stays in a core's own cache, and large enough that the rows
/// of overlap between neighbouring bands are a rounding error rather than real
/// duplicated work.
const BAND: usize = 32;

/// How many f32 of one output row the vertical pass accumulates at a time.
///
/// The accumulator is read and written once per tap, so it wants to be in L1:
/// a whole 4K row of it is 61KB and is not.
const STRIP: usize = 1024;

/// A cached set of resampling weights for one source and destination size,
/// with the buffers the passes need.
#[derive(Default)]
pub struct Scaler {
    src: (usize, usize),
    dst: (usize, usize),
    horizontal: Vec<Taps>,
    vertical: Vec<Taps>,
    /// One scratch per worker, each `band_rows` source rows of `dst_w` RGBA f32.
    scratch: Vec<f32>,
    /// The most source rows any one band reads, which is how tall a scratch is.
    band_rows: usize,
    /// The finished frame, kept between calls so a 4K one is not reallocated
    /// and rezeroed on every frame of every movie.
    out: Vec<u8>,
}

/// Turns one filtered f32 channel into the byte that goes on screen.
///
/// The clamp is a guard the B-spline does not need — its weights are
/// non-negative and sum to one, so a value here cannot leave `0.0..=255.0` by
/// more than float error — but it costs nothing, and it is what makes the line
/// below safe to write.
///
/// That line is the measured reason this is a function and not `as u8`. Adding
/// `1.5 * 2^23` to a value in `0..=255` forces the exponent to 23, where a
/// binary32's ULP is exactly 1, so the addition rounds to the nearest integer
/// and leaves it in the low bits of the mantissa; reading those bits back is
/// the conversion. It avoids the float-to-int instruction altogether, which is
/// what stops LLVM vectorising the loop: over a frame's worth of bytes this
/// measured 4ms against 17ms for `(x + 0.5) as u8`. `f32::round` — the obvious
/// way to write it — is worse still, a libm call per byte on a baseline x86-64
/// target, and it cost more than the filter did.
#[inline]
fn to_byte(v: f32) -> u8 {
    /// `1.5 * 2^23`, the smallest bias that puts every byte value in the low
    /// mantissa bits of a normal binary32.
    const BIAS: f32 = 12582912.0;
    (v.clamp(0.0, 255.0) + BIAS).to_bits() as u8
}

/// The cubic B-spline kernel, `B = 1, C = 0`.
fn kernel(x: f32) -> f32 {
    let x = x.abs();
    if x < 1.0 {
        (3.0 * x * x * x - 6.0 * x * x + 4.0) / 6.0
    } else if x < 2.0 {
        (-(x * x * x) + 6.0 * x * x - 12.0 * x + 8.0) / 6.0
    } else {
        0.0
    }
}

/// Builds one axis' worth of taps.
///
/// `support` is the kernel's radius in source pixels: 2 when upscaling, and
/// widened by the shrink factor when downscaling so the footprint covers every
/// source pixel that contributes.
fn axis(src: usize, dst: usize) -> Vec<Taps> {
    let ratio = src as f32 / dst as f32;
    let filter_scale = ratio.max(1.0);
    let support = 2.0 * filter_scale;
    (0..dst)
        .map(|out| {
            // The centre of this output pixel, in source coordinates.
            let centre = (out as f32 + 0.5) * ratio - 0.5;
            let first = (centre - support).ceil() as i64;
            let last = (centre + support).floor() as i64;
            let mut weights = Vec::with_capacity((last - first + 1).max(1) as usize);
            let mut total = 0.0f32;
            for i in first..=last {
                let w = kernel((i as f32 - centre) / filter_scale);
                weights.push(w);
                total += w;
            }
            // Clamp the footprint to the edges by folding the out-of-range
            // weight onto the nearest real pixel, which is what makes an edge
            // pixel keep its own colour instead of fading toward nothing.
            let mut start = first;
            while start < 0 && weights.len() > 1 {
                let w = weights.remove(0);
                weights[0] += w;
                start += 1;
            }
            let mut end = start + weights.len() as i64 - 1;
            while end > src as i64 - 1 && weights.len() > 1 {
                let w = weights.pop().unwrap_or(0.0);
                if let Some(back) = weights.last_mut() {
                    *back += w;
                }
                end -= 1;
            }
            if total != 0.0 {
                for w in &mut weights {
                    *w /= total;
                }
            }
            // A tap at exactly the kernel's radius has weight zero, which
            // happens whenever an output pixel's centre lands on a source
            // pixel's. Dropping those is not just saved work: it is what makes
            // every upscaled output pixel exactly four taps, which is the case
            // the passes have a fast path for.
            while weights.len() > 1 && weights[0] == 0.0 {
                weights.remove(0);
                start += 1;
            }
            while weights.len() > 1 && weights[weights.len() - 1] == 0.0 {
                weights.pop();
            }
            // The folding above already pulls the footprint inside the
            // image except in the degenerate one-tap case. Bringing it into
            // range here means the passes can index the row directly instead
            // of clamping on every tap.
            let start = start.clamp(0, (src as i64 - weights.len() as i64).max(0)) as usize;
            Taps { start, weights }
        })
        .collect()
}

/// The most source rows any one band reads.
fn band_rows(vertical: &[Taps]) -> usize {
    vertical
        .chunks(BAND)
        .map(|band| {
            let first = band[0].start;
            band.last()
                .map_or(0, |taps| taps.start + taps.weights.len() - first)
        })
        .max()
        .unwrap_or(0)
}

/// How many bands to filter at once: one per core, and never more than there
/// are bands to hand out.
fn workers(bands: usize) -> usize {
    std::thread::available_parallelism()
        .map_or(1, std::num::NonZeroUsize::get)
        .min(bands)
        .max(1)
}

impl Scaler {
    /// Builds the weights and buffers for one source and destination size.
    pub fn new(src: (usize, usize), dst: (usize, usize)) -> Scaler {
        let vertical = axis(src.1, dst.1);
        let rows = band_rows(&vertical);
        let workers = workers(vertical.len().div_ceil(BAND).max(1));
        Scaler {
            src,
            dst,
            horizontal: axis(src.0, dst.0),
            band_rows: rows,
            scratch: vec![0.0; workers * rows * dst.0 * 4],
            out: vec![0; dst.0 * dst.1 * 4],
            vertical,
        }
    }

    /// Rebuilds the weights if the sizes have changed, and answers whether this
    /// scaler is usable for them.
    pub fn fit(&mut self, src: (usize, usize), dst: (usize, usize)) {
        if self.src != src || self.dst != dst {
            *self = Scaler::new(src, dst);
        }
    }

    /// Resamples `rgba` from `src` to `dst`.
    ///
    /// The frame comes back as a slice of the scaler's own buffer, valid until
    /// the next call. Returns `None` when there is nothing to do — the sizes
    /// already match, or either is empty — so the caller can upload the source
    /// as it stands rather than pay for a pass that would only soften it.
    pub fn resample(
        &mut self,
        rgba: &[u8],
        src: (usize, usize),
        dst: (usize, usize),
    ) -> Option<&[u8]> {
        if src == dst || src.0 == 0 || src.1 == 0 || dst.0 == 0 || dst.1 == 0 {
            return None;
        }
        if rgba.len() < src.0 * src.1 * 4 {
            log::warn!(
                "a {}x{} frame arrived with {} bytes; not scaling it",
                src.0,
                src.1,
                rgba.len()
            );
            return None;
        }
        self.fit(src, dst);

        let Scaler {
            horizontal,
            vertical,
            scratch,
            band_rows,
            out,
            ..
        } = self;
        let stride = dst.0 * 4;
        let pad = *band_rows * stride;

        // One band of output rows is one unit of work: it reads the source, it
        // writes its own rows and nothing else's, and it borrows a scratch of
        // its own. Nothing is shared, so handing the next one out is the whole
        // of the synchronisation.
        let mut queue = vertical.chunks(BAND).zip(out.chunks_mut(stride * BAND));
        if scratch.len() <= pad {
            // One worker's worth: a UI sprite, or a machine with one core.
            // Spawning a thread to hand work to yourself is pure overhead.
            for (taps, out_band) in queue {
                band(rgba, src.0, stride, horizontal, taps, scratch, out_band);
            }
        } else {
            let queue = &std::sync::Mutex::new(&mut queue);
            let horizontal = &*horizontal;
            std::thread::scope(|scope| {
                for scratch in scratch.chunks_mut(pad) {
                    scope.spawn(move || loop {
                        // Locked only to take the next band, never while one is
                        // being filtered. A poisoned lock means another worker
                        // panicked; there is nothing useful to do but stop.
                        let Ok(mut queue) = queue.lock() else {
                            return;
                        };
                        let Some((taps, out_band)) = queue.next() else {
                            return;
                        };
                        drop(queue);
                        band(rgba, src.0, stride, horizontal, taps, scratch, out_band);
                    });
                }
            });
        }
        Some(&self.out[..stride * dst.1])
    }
}

/// Filters one band of output rows.
///
/// `taps` is the band's slice of the vertical weights and `out_band` its rows
/// of the frame; `scratch` is this worker's own, big enough for every source
/// row the band reads.
fn band(
    rgba: &[u8],
    src_w: usize,
    stride: usize,
    horizontal: &[Taps],
    taps: &[Taps],
    scratch: &mut [f32],
    out_band: &mut [u8],
) {
    // The span of source rows this band reads. Neighbouring bands overlap by
    // the kernel's support and filter those few rows twice, which is cheaper
    // than any way of sharing them would be.
    let first = taps[0].start;
    let Some(last) = taps.last().map(|t| t.start + t.weights.len()) else {
        return;
    };

    // Horizontal, into the scratch as f32 so the vertical pass does not round
    // twice. One output pixel gathers a short run of the source row, so the row
    // is sliced once and the taps read it in order.
    for (in_row, out_row) in rgba[first * src_w * 4..last * src_w * 4]
        .chunks_exact(src_w * 4)
        .zip(scratch.chunks_exact_mut(stride))
    {
        for (out, taps) in out_row.as_chunks_mut::<4>().0.iter_mut().zip(horizontal) {
            let mut acc = [0.0f32; 4];
            let run = &in_row[taps.start * 4..(taps.start + taps.weights.len()) * 4];
            for (px, w) in run.as_chunks::<4>().0.iter().zip(&taps.weights) {
                for (a, s) in acc.iter_mut().zip(px) {
                    *a += f32::from(*s) * w;
                }
            }
            out.copy_from_slice(&acc);
        }
    }

    // Vertical, straight out to bytes. A tap of an output row is a whole
    // scratch row, so this reads along rows rather than down a column.
    for (taps, out_row) in taps.iter().zip(out_band.chunks_exact_mut(stride)) {
        let at = (taps.start - first) * stride;
        let run = &scratch[at..at + taps.weights.len() * stride];
        column(run, stride, &taps.weights, out_row);
    }
}

/// One output row, from the scratch rows its taps name.
fn column(run: &[f32], stride: usize, weights: &[f32], out: &mut [u8]) {
    // Up to four taps is every pixel of an upscale, once zero weights are
    // trimmed, and it is worth taking straight: the source values and the
    // running sum all stay in registers, so an output byte costs one load per
    // tap and a store. The general case below accumulates into a buffer and so
    // reads and writes that buffer once per tap — and the traffic, not the
    // arithmetic, is what a resample is bound by.
    if run.len() == weights.len() * stride && stride >= out.len() {
        // Four is the interior of every upscale, and gets written out rather
        // than left to the generic form below, which measures 40% slower on it.
        if let ([w0, w1, w2, w3], [r0, r1, r2, r3]) = (weights, &rows::<4>(run, stride, out.len()))
        {
            for (i, o) in out.iter_mut().enumerate() {
                *o = to_byte(r0[i] * w0 + r1[i] * w1 + r2[i] * w2 + r3[i] * w3);
            }
            return;
        }
        match weights.len() {
            1 => return straight::<1>(run, stride, weights, out),
            2 => return straight::<2>(run, stride, weights, out),
            3 => return straight::<3>(run, stride, weights, out),
            _ => {}
        }
    }

    // Anything else: a downscale, whose footprint is as wide as the shrink
    // factor. A strip at a time, on the stack, because a 4K row of accumulator
    // does not fit in L1.
    for (strip, out_strip) in (0..).zip(out.chunks_mut(STRIP)) {
        let from = strip * STRIP;
        let mut acc = [0.0f32; STRIP];
        let acc = &mut acc[..out_strip.len()];
        for (row, w) in run.chunks_exact(stride).zip(weights) {
            let w = *w;
            for (a, s) in acc.iter_mut().zip(&row[from..from + out_strip.len()]) {
                *a += *s * w;
            }
        }
        for (o, a) in out_strip.iter_mut().zip(&*acc) {
            *o = to_byte(*a);
        }
    }
}

/// The `N` scratch rows of `run`, each sliced to `len` so indexing them carries
/// no bounds check into the filter loop.
fn rows<const N: usize>(run: &[f32], stride: usize, len: usize) -> [&[f32]; N] {
    let mut rows: [&[f32]; N] = [&[]; N];
    for (slot, row) in rows.iter_mut().zip(run.chunks_exact(stride)) {
        *slot = &row[..len];
    }
    rows
}

/// One output row from exactly `N` taps, with nothing spilled to memory.
///
/// `N` is a constant so the inner sum unrolls into registers, and every row is
/// sliced to the output's own length so the indexing carries no check into the
/// loop. Both of those matter: writing this with a bounds-checked `get` instead
/// stopped it vectorising and cost more than the whole fast path saves.
///
/// The caller guarantees `run` holds exactly `N` rows of `stride` and that
/// `stride` covers `out`.
fn straight<const N: usize>(run: &[f32], stride: usize, weights: &[f32], out: &mut [u8]) {
    let rows = rows::<N>(run, stride, out.len());
    let mut w = [0.0f32; N];
    w.copy_from_slice(&weights[..N]);
    for (i, o) in out.iter_mut().enumerate() {
        let mut v = 0.0f32;
        for (row, w) in rows.iter().zip(&w) {
            v += row[i] * *w;
        }
        *o = to_byte(v);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The kernel is a partition of unity: whatever the sub-pixel offset, the
    /// four taps sum to 1, so a flat field stays flat.
    #[test]
    fn the_kernel_sums_to_one_at_every_offset() {
        for step in 0..64 {
            let frac = step as f32 / 64.0;
            let sum: f32 = (-1..=2).map(|i| kernel(i as f32 - frac)).sum();
            assert!((sum - 1.0).abs() < 1e-5, "offset {frac} summed to {sum}");
        }
    }

    /// It is non-negative everywhere, which is the property that rules out
    /// ringing — the whole reason for choosing a B-spline over Catmull-Rom.
    #[test]
    fn the_kernel_never_goes_negative() {
        for step in -300..=300 {
            let x = step as f32 / 100.0;
            assert!(kernel(x) >= 0.0, "k({x}) = {}", kernel(x));
        }
    }

    /// `to_byte` is a bit trick, so it is checked against the arithmetic it
    /// stands for across the whole range, and at the ends where it could wrap.
    #[test]
    fn the_byte_conversion_rounds_and_clamps() {
        for v in 0..=255u32 {
            assert_eq!(to_byte(v as f32), v as u8, "{v} exactly");
            assert_eq!(to_byte(v as f32 + 0.25), v as u8, "{v} and a quarter");
        }
        assert_eq!(to_byte(-0.4), 0, "below the range clamps, it does not wrap");
        assert_eq!(to_byte(-1000.0), 0);
        assert_eq!(to_byte(255.7), 255, "above it clamps too");
        assert_eq!(to_byte(1e9), 255);
        assert_eq!(to_byte(0.5), 0, "a tie goes to even");
        assert_eq!(to_byte(1.5), 2);
    }

    /// Upscaling never needs more than four taps once the zero-weight ones at
    /// the kernel's radius are trimmed. [`column`] takes that path straight
    /// into registers, so this is the property the speed rests on — including
    /// at whole-number ratios, where the untrimmed footprint is five wide.
    #[test]
    fn an_upscale_never_needs_more_than_four_taps() {
        for (src, dst) in [(800, 1920), (452, 1085), (720, 2160), (7, 21), (100, 101)] {
            let taps = axis(src, dst);
            let widest = taps.iter().map(|t| t.weights.len()).max().unwrap_or(0);
            assert!(widest <= 4, "{src} -> {dst} needed {widest} taps");
            for t in &taps {
                assert!(t.weights.iter().all(|w| *w != 0.0), "a zero tap survived");
                assert!(
                    t.start + t.weights.len() <= src,
                    "{src} -> {dst} runs off the end"
                );
            }
        }
    }

    /// The frame is filtered in bands of [`BAND`] output rows, on as many
    /// threads as the machine has, and neighbouring bands share the rows their
    /// kernels overlap on. A band that read the wrong rows, or a worker that
    /// wrote into another's, would show up as a seam every 32 rows — so this
    /// upscales a vertical ramp tall enough to cross many band boundaries and
    /// insists it is still a ramp the whole way down.
    #[test]
    fn bands_join_without_a_seam() {
        let (w, h) = (8usize, 64usize);
        let mut src = vec![0u8; w * h * 4];
        for (y, row) in src.chunks_exact_mut(w * 4).enumerate() {
            let v = (y * 255 / (h - 1)) as u8;
            row.fill(v);
        }
        let (dw, dh) = (16usize, 500usize);
        let mut scaler = Scaler::default();
        let out = scaler
            .resample(&src, (w, h), (dw, dh))
            .expect("should scale");
        let at = |y: usize| out[y * dw * 4] as i32;
        for y in 1..dh {
            let step = at(y) - at(y - 1);
            assert!(
                (0..=3).contains(&step),
                "row {y} jumped by {step}: a band boundary at {}",
                y % BAND
            );
        }
        assert_eq!(at(0), 0, "the top stays the top");
        assert_eq!(at(dh - 1), 255, "and the bottom the bottom");
    }

    /// Nothing to do is nothing done: an unscaled frame is handed back
    /// untouched rather than softened by a 1:1 pass.
    #[test]
    fn a_matching_size_is_not_resampled() {
        let mut scaler = Scaler::default();
        let src = vec![0u8; 4 * 4 * 4];
        assert!(scaler.resample(&src, (4, 4), (4, 4)).is_none());
        assert!(scaler.resample(&src, (4, 4), (0, 4)).is_none());
    }

    /// A flat colour stays exactly that colour at any size, in every channel.
    /// This is what the weights summing to 1 buys, and it catches an edge
    /// clamp that loses weight off the sides.
    #[test]
    fn a_flat_field_survives_scaling_in_both_directions() {
        let mut scaler = Scaler::default();
        for (src, dst) in [((8, 8), (32, 20)), ((64, 40), (11, 7)), ((5, 9), (9, 5))] {
            let pixels = vec![0u8; src.0 * src.1 * 4]
                .chunks(4)
                .flat_map(|_| [17u8, 99, 200, 255])
                .collect::<Vec<u8>>();
            let out = scaler.resample(&pixels, src, dst).expect("should scale");
            assert_eq!(out.len(), dst.0 * dst.1 * 4);
            for (i, px) in out.as_chunks::<4>().0.iter().enumerate() {
                assert_eq!(*px, [17, 99, 200, 255], "{src:?} -> {dst:?} pixel {i}");
            }
        }
    }

    /// Upscaling keeps the picture's shape: a dark half stays dark, a light
    /// half stays light, and the seam is somewhere in the middle rather than
    /// smeared across the whole image.
    #[test]
    fn upscaling_keeps_the_picture_where_it_was() {
        let mut scaler = Scaler::default();
        let (w, h) = (8usize, 8usize);
        let mut src = vec![0u8; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                let v = if x < w / 2 { 0 } else { 255 };
                let at = (y * w + x) * 4;
                src[at..at + 4].copy_from_slice(&[v, v, v, 255]);
            }
        }
        let (dw, dh) = (32usize, 32usize);
        let out = scaler
            .resample(&src, (w, h), (dw, dh))
            .expect("should scale");
        let at = |x: usize, y: usize| out[(y * dw + x) * 4] as u32;
        assert_eq!(at(0, 16), 0, "the left edge stays black");
        assert_eq!(at(dw - 1, 16), 255, "the right edge stays white");
        assert!(at(2, 16) < 40, "the dark half is still dark");
        assert!(at(dw - 3, 16) > 215, "the light half is still light");
        // Monotone across the seam: no overshoot above 255 or below 0, which a
        // ringing filter would produce and this one cannot.
        for x in 1..dw {
            assert!(at(x, 16) >= at(x - 1, 16), "not monotone at x={x}");
        }
    }

    /// Downscaling widens the kernel, so shrinking averages rather than
    /// point-sampling: a one-pixel checkerboard collapses to its mean instead
    /// of aliasing into stripes.
    #[test]
    fn downscaling_averages_instead_of_aliasing() {
        let mut scaler = Scaler::default();
        let (w, h) = (64usize, 64usize);
        let mut src = vec![0u8; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                let v = if (x + y) % 2 == 0 { 0 } else { 255 };
                let at = (y * w + x) * 4;
                src[at..at + 4].copy_from_slice(&[v, v, v, 255]);
            }
        }
        let out = scaler.resample(&src, (w, h), (8, 8)).expect("should scale");
        for (i, px) in out.as_chunks::<4>().0.iter().enumerate() {
            assert!(
                (100..=155).contains(&px[0]),
                "pixel {i} came out {} rather than around the mean",
                px[0]
            );
        }
    }

    /// The weights are rebuilt when the window changes size and reused when it
    /// does not.
    #[test]
    fn the_weights_follow_the_sizes() {
        let mut scaler = Scaler::default();
        scaler.fit((10, 10), (20, 20));
        assert_eq!(scaler.horizontal.len(), 20);
        scaler.fit((10, 10), (37, 5));
        assert_eq!(scaler.horizontal.len(), 37);
        assert_eq!(scaler.vertical.len(), 5);
    }
}
