//! Resampling, for getting the game's art onto a window that is not its size.
//!
//! # Why this is not the original's scaler
//!
//! Everything else in this engine is a recovered behaviour. This is not: it is
//! a deliberate, documented departure. The original hands its surfaces to
//! Direct3D and takes whatever the driver's bilinear filter gives, which at
//! 800x450 stretched onto a modern panel is a soft, slightly aliased picture
//! that varies with the driver. There is nothing to recover here that would be
//! worth reproducing, and reproducing a driver's filter faithfully is not
//! possible anyway.
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
//! reused: each output pixel is a short gather over the source, four taps when
//! upscaling and more when the picture is being shrunk. Horizontal first into a
//! scratch buffer of `dst_w x src_h`, then vertical, which is the cheaper order
//! whenever the output is wider than it is taller relative to the source.
//!
//! Downscaling widens the kernel in source space by the scale factor, so
//! shrinking averages over every source pixel that lands in the footprint
//! rather than point-sampling four of them and aliasing.

/// One output pixel's taps along one axis.
struct Taps {
    /// Index of the first source pixel this output pixel reads.
    start: usize,
    /// Weights, summing to 1.
    weights: Vec<f32>,
}

/// A cached set of resampling weights for one source and destination size.
#[derive(Default)]
pub struct Scaler {
    src: (usize, usize),
    dst: (usize, usize),
    horizontal: Vec<Taps>,
    vertical: Vec<Taps>,
    /// Scratch for the horizontal pass: `dst_w x src_h` in RGBA f32.
    scratch: Vec<f32>,
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
            Taps {
                start: start.max(0) as usize,
                weights,
            }
        })
        .collect()
}

impl Scaler {
    /// Builds the weights for one source and destination size.
    pub fn new(src: (usize, usize), dst: (usize, usize)) -> Scaler {
        Scaler {
            src,
            dst,
            horizontal: axis(src.0, dst.0),
            vertical: axis(src.1, dst.1),
            scratch: vec![0.0; dst.0 * src.1 * 4],
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
    /// Returns `None` when there is nothing to do — the sizes already match, or
    /// either is empty — so the caller can upload the source as it stands
    /// rather than pay for a pass that would only soften it.
    pub fn resample(
        &mut self,
        rgba: &[u8],
        src: (usize, usize),
        dst: (usize, usize),
    ) -> Option<Vec<u8>> {
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

        // Horizontal, into the scratch as f32 so the vertical pass does not
        // round twice.
        for y in 0..src.1 {
            let row = y * src.0 * 4;
            let out_row = y * dst.0 * 4;
            for (x, taps) in self.horizontal.iter().enumerate() {
                let mut acc = [0.0f32; 4];
                for (i, w) in taps.weights.iter().enumerate() {
                    let sx = (taps.start + i).min(src.0 - 1);
                    let px = row + sx * 4;
                    for (c, a) in acc.iter_mut().enumerate() {
                        *a += f32::from(rgba[px + c]) * w;
                    }
                }
                let at = out_row + x * 4;
                self.scratch[at..at + 4].copy_from_slice(&acc);
            }
        }

        // Vertical, straight out to bytes.
        let mut out = vec![0u8; dst.0 * dst.1 * 4];
        for (y, taps) in self.vertical.iter().enumerate() {
            let out_row = y * dst.0 * 4;
            for x in 0..dst.0 {
                let mut acc = [0.0f32; 4];
                for (i, w) in taps.weights.iter().enumerate() {
                    let sy = (taps.start + i).min(src.1 - 1);
                    let at = (sy * dst.0 + x) * 4;
                    for (c, a) in acc.iter_mut().enumerate() {
                        *a += self.scratch[at + c] * w;
                    }
                }
                let at = out_row + x * 4;
                for (c, a) in acc.iter().enumerate() {
                    // The kernel is non-negative so this cannot overshoot, but
                    // rounding still has to land inside the byte range.
                    out[at + c] = a.round().clamp(0.0, 255.0) as u8;
                }
            }
        }
        Some(out)
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
