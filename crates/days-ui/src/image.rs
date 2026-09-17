//! A plain RGBA image, PNG decode, and the three blits screens compose with: an
//! alpha blit, a nearest-neighbour stretch and an averaging downscale.
//!
//! Deliberately small, and deliberately not a filter library. Scaling art from
//! the native 800x450 layout up to a display size is filtered work — the
//! original sets `D3DTEXF_LINEAR` on every sampler stage in `FUN_0044a3d0` —
//! and the engine does it with the resampler in `daysengine::playback::scale`,
//! which is where a filter belongs. What is left here is placement: putting a
//! rectangle of one image onto another with the right alpha.

use crate::Error;
use std::sync::atomic::{AtomicU64, Ordering};

/// An 8-bit RGBA image.
///
/// Every image also carries a [`version`](Image::version): a number that is
/// new on each one built and changes whenever its pixels do. It exists so a
/// caller that turns images into something expensive -- a GPU texture -- can
/// tell whether the one it is holding is still the one in front of it, without
/// comparing eight megabytes to find out. See [`crate::Layer`]'s users.
#[derive(Debug)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    version: u64,
}

/// Hands out [`Image::version`]s. Wrapping is 2^64 images into a run.
static VERSIONS: AtomicU64 = AtomicU64::new(1);

fn next_version() -> u64 {
    VERSIONS.fetch_add(1, Ordering::Relaxed)
}

impl Clone for Image {
    /// A clone is a **different image**: it can be written to without the
    /// original changing, so it gets a version of its own.
    fn clone(&self) -> Image {
        Image {
            width: self.width,
            height: self.height,
            rgba: self.rgba.clone(),
            version: next_version(),
        }
    }
}

impl Image {
    /// A transparent image.
    pub fn empty(width: u32, height: u32) -> Image {
        Image::from_rgba(width, height, vec![0; width as usize * height as usize * 4])
    }

    /// An image over pixels that are already RGBA.
    pub fn from_rgba(width: u32, height: u32, rgba: Vec<u8>) -> Image {
        Image {
            width,
            height,
            rgba,
            version: next_version(),
        }
    }

    /// This image's identity, for a cache that holds something built from it.
    ///
    /// No two live images share one, and it changes whenever the pixels are
    /// written to, so a cache keyed on it can never serve a stale copy. It says
    /// nothing about *contents*: two images of the same thing have different
    /// versions, which costs a rebuild and never a wrong picture.
    pub fn version(&self) -> u64 {
        self.version
    }

    /// An opaque black image, the background a screen composites onto.
    pub fn black(width: u32, height: u32) -> Image {
        let mut img = Image::empty(width, height);
        for px in img.rgba.as_chunks_mut::<4>().0 {
            *px = [0, 0, 0, 255];
        }
        img
    }

    /// Decodes a PNG, widening greyscale and RGB to RGBA.
    pub fn decode_png(bytes: &[u8]) -> Result<Image, Error> {
        let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
        let mut reader = decoder.read_info()?;
        let mut buf = vec![0; reader.output_buffer_size().unwrap_or(0)];
        let info = reader.next_frame(&mut buf)?;
        let n = info.buffer_size();
        let rgba = match info.color_type {
            png::ColorType::Rgba => buf[..n].to_vec(),
            png::ColorType::Rgb => buf[..n]
                .as_chunks::<3>()
                .0
                .iter()
                .flat_map(|p| [p[0], p[1], p[2], 255])
                .collect(),
            png::ColorType::Grayscale => buf[..n].iter().flat_map(|g| [*g, *g, *g, 255]).collect(),
            png::ColorType::GrayscaleAlpha => buf[..n]
                .as_chunks::<2>()
                .0
                .iter()
                .flat_map(|p| [p[0], p[0], p[0], p[1]])
                .collect(),
            png::ColorType::Indexed => return Err(Error::UnsupportedPng),
        };
        Ok(Image::from_rgba(info.width, info.height, rgba))
    }

    /// Multiplies `alpha` through every pixel's own.
    ///
    /// The headless counterpart of an SDL alpha modulation: a layer that is
    /// drawn through a fade has one texture-wide alpha on the GPU, and this is
    /// the same thing done to the pixels for the path that has no texture.
    pub fn modulate(&mut self, alpha: u8) {
        if alpha == 255 {
            return;
        }
        self.version = next_version();
        let alpha = u32::from(alpha);
        for px in self.rgba.as_chunks_mut::<4>().0 {
            px[3] = (u32::from(px[3]) * alpha / 255) as u8;
        }
    }

    /// One pixel, or `None` outside the image.
    pub fn pixel(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let i = (y as usize * self.width as usize + x as usize) * 4;
        self.rgba.get(i..i + 4).map(|p| [p[0], p[1], p[2], p[3]])
    }

    /// Alpha-blends a rectangle of `src` onto `self`, averaging over the source
    /// area instead of point sampling it.
    ///
    /// [`Image::blit_scaled`] takes one source pixel per destination pixel,
    /// which is wrong for text that was rasterised at twice its final size:
    /// taking one source pixel in four turns a glyph stroke into a row of
    /// specks. This averages the source cell each destination pixel covers,
    /// weighting colour by alpha so a half-covered edge does not drag the
    /// glyph's colour towards its transparent surroundings.
    pub fn blit_downscaled(
        &mut self,
        src: &Image,
        src_rect: (u32, u32, u32, u32),
        dst: (i64, i64, u32, u32),
    ) {
        let (dx, dy, dw, dh) = dst;
        let cell = src.downscaled(src_rect, (dw, dh));
        self.blit_scaled(&cell, (0, 0, cell.width, cell.height), (dx, dy, dw, dh));
    }

    /// `src_rect` averaged down into an image of `size`.
    ///
    /// The averaging half of [`Image::blit_downscaled`], on its own, because a
    /// caller that draws through something other than a blit needs the pixels
    /// rather than the blend -- see `daysengine::ui::screen::Layer`. Drawing
    /// this 1:1 is what `blit_downscaled` is: an averaged cell composites
    /// exactly as the one source pixel it stands for would.
    pub fn downscaled(&self, src_rect: (u32, u32, u32, u32), size: (u32, u32)) -> Image {
        let (sx, sy, sw, sh) = src_rect;
        let (dw, dh) = size;
        let mut out = Image::empty(dw, dh);
        if sw == 0 || sh == 0 || dw == 0 || dh == 0 {
            return out;
        }

        // The run of source bytes each destination column averages over, in
        // the source row's own coordinates and worked out once for the whole
        // image -- the same footprint serves every row. Inline it cost four
        // 64-bit divisions per destination pixel.
        let span = |at: u32, n: u32, out: u32, from: u32, limit: u32| {
            let lo = u64::from(at) * u64::from(n) / u64::from(out);
            let hi = ((u64::from(at) + 1) * u64::from(n) / u64::from(out)).max(lo + 1);
            let end = u64::from(n)
                .min(hi)
                .min(u64::from(limit.saturating_sub(from)));
            (lo.min(end) as usize, end as usize)
        };
        let cols: Vec<(usize, usize)> = (0..dw)
            .map(|col| {
                let (lo, hi) = span(col, sw, dw, sx, self.width);
                ((sx as usize + lo) * 4, (sx as usize + hi) * 4)
            })
            .collect();

        let src_w = self.width as usize;
        for (row, line) in out.rgba.chunks_exact_mut(dw as usize * 4).enumerate() {
            let (y0, y1) = span(row as u32, sh, dh, sy, self.height);
            for (px, (from, to)) in line.as_chunks_mut::<4>().0.iter_mut().zip(&cols) {
                let mut cells = 0u32;
                let mut alpha = 0u32;
                let mut colour = [0u32; 3];
                for y in y0..y1 {
                    let base = (sy as usize + y) * src_w * 4;
                    let row = &self.rgba[base.min(self.rgba.len())..];
                    let run = &row[(*from).min(row.len())..(*to).min(row.len())];
                    for p in run.as_chunks::<4>().0 {
                        cells += 1;
                        alpha += u32::from(p[3]);
                        for (sum, c) in colour.iter_mut().zip(p) {
                            *sum += u32::from(*c) * u32::from(p[3]);
                        }
                    }
                }
                if cells == 0 || alpha == 0 {
                    continue;
                }
                let a = alpha / cells;
                if a == 0 {
                    continue;
                }
                // `sum / alpha` is the average colour of the covered area with
                // the alpha weighting taken back out, which is the colour the
                // cell composites with.
                *px = [
                    (colour[0] / alpha) as u8,
                    (colour[1] / alpha) as u8,
                    (colour[2] / alpha) as u8,
                    a as u8,
                ];
            }
        }
        out
    }

    /// Alpha-blends a rectangle of `src` onto `self`, stretching it to `dst`.
    ///
    /// `src_rect` is `(x, y, width, height)` in `src`; `dst` is the destination
    /// rectangle in `self`. Both are clipped rather than checked, so a screen
    /// with a widget hanging off the edge draws what fits.
    ///
    /// The stretch is nearest-neighbour, so callers that are changing an
    /// image's size resample it first and hand this one a rectangle it can copy
    /// across — see `daysengine::ui::screen`.
    pub fn blit_scaled(
        &mut self,
        src: &Image,
        src_rect: (u32, u32, u32, u32),
        dst: (i64, i64, u32, u32),
    ) {
        let (sx, sy, sw, sh) = src_rect;
        let (dx0, dy0, dw, dh) = dst;
        if sw == 0 || sh == 0 || dw == 0 || dh == 0 {
            return;
        }

        self.version = next_version();

        // Clipped once, into a range of destination rows and columns. Testing
        // each pixel against all four edges as it was written meant a
        // full-screen blit did four comparisons per pixel to discover that
        // every one of them was inside.
        let (col_from, col_to) = clip(dx0, dw, self.width);
        let (row_from, row_to) = clip(dy0, dh, self.height);
        if col_from == col_to || row_from == row_to {
            return;
        }

        // Which source column each destination column samples, worked out once
        // for the whole blit. The map is the same on every row, and computing
        // it inline cost two 64-bit divisions per pixel -- on a 1:1 blit, which
        // is what a menu screen's base art is, that was the bulk of the work
        // for a map that is the identity.
        let mut cols: Vec<u32> = Vec::with_capacity((col_to - col_from) as usize);
        for col in col_from..col_to {
            let s_x = u64::from(sx) + u64::from(col) * u64::from(sw) / u64::from(dw);
            // The map only rises, so the first column past the source edge is
            // the end of what this blit draws.
            if s_x >= u64::from(src.width) {
                break;
            }
            cols.push(s_x as u32);
        }
        let Some(first) = cols.first().copied() else {
            return;
        };

        let dst_w = self.width as usize;
        let src_w = src.width as usize;
        for row in row_from..row_to {
            let s_y = u64::from(sy) + u64::from(row) * u64::from(sh) / u64::from(dh);
            if s_y >= u64::from(src.height) {
                break;
            }
            let dy = (dy0 + i64::from(row)) as usize;
            let s_row = s_y as usize * src_w * 4;
            let sr = &src.rgba[s_row.min(src.rgba.len())..(s_row + src_w * 4).min(src.rgba.len())];
            let d_row = dy * dst_w * 4 + (dx0 + i64::from(col_from)) as usize * 4;
            let dr = &mut self.rgba[d_row..dy * dst_w * 4 + dst_w * 4];

            if sw == dw {
                // Unstretched: the source columns are consecutive, so the two
                // runs are walked side by side and the column map is not read
                // at all. This is the menus' own case.
                let run = &sr[(first as usize * 4).min(sr.len())..];
                for (d, s) in dr
                    .as_chunks_mut::<4>()
                    .0
                    .iter_mut()
                    .zip(run.as_chunks::<4>().0)
                    .take(cols.len())
                {
                    blend(d, *s);
                }
            } else {
                for (d, s_x) in dr.as_chunks_mut::<4>().0.iter_mut().zip(&cols) {
                    let o = *s_x as usize * 4;
                    let Some(s) = sr.get(o..o + 4) else {
                        continue;
                    };
                    blend(d, [s[0], s[1], s[2], s[3]]);
                }
            }
        }
    }
}

/// The range of destination rows (or columns) a run of `len` placed at `at`
/// actually lands on, clipped to `limit`.
fn clip(at: i64, len: u32, limit: u32) -> (u32, u32) {
    let from = (-at).clamp(0, i64::from(len)) as u32;
    let to = (i64::from(limit) - at).clamp(0, i64::from(len)) as u32;
    (from, to.max(from))
}

/// Source-over of one pixel.
///
/// The arithmetic is the general case -- compositing the destination's alpha as
/// well as its colour, because the in-game overlays are composited onto a
/// transparent layer and handed to the caller to blend over the frame. What is
/// taken first are the two cases that need no division by a variable, and
/// between them they are almost every pixel a screen composites:
///
/// * a fully opaque source pixel **is** the result (`out_a` works out to 255
///   and each channel to the source's own), so it is a copy; and
/// * over an opaque destination the divisor is 255 whatever the source alpha
///   is, and a division by a constant is a multiply.
///
/// Both are the same values the general form below produces, not an
/// approximation of them.
#[inline(always)]
fn blend(d: &mut [u8; 4], p: [u8; 4]) {
    let a = u32::from(p[3]);
    if a == 0 {
        return;
    }
    if a == 255 {
        *d = p;
        return;
    }
    let ia = 255 - a;
    let da = u32::from(d[3]);
    if da == 255 {
        for (channel, src) in d[..3].iter_mut().zip(p) {
            *channel = ((u32::from(src) * a + u32::from(*channel) * ia) / 255) as u8;
        }
        return;
    }
    let out_a = a + da * ia / 255;
    if out_a == 0 {
        return;
    }
    for (channel, src) in d[..3].iter_mut().zip(p) {
        let c = u32::from(src) * a * 255 + u32::from(*channel) * da * ia;
        *channel = (c / (out_a * 255)) as u8;
    }
    d[3] = out_a as u8;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: u32, h: u32, px: [u8; 4]) -> Image {
        let mut img = Image::empty(w, h);
        for p in img.rgba.as_chunks_mut::<4>().0 {
            *p = px;
        }
        img
    }

    /// One lit source pixel in four averages to a quarter coverage, where point
    /// sampling would either keep it whole or lose it entirely. This is the
    /// difference between a readable glyph stroke and a row of specks.
    #[test]
    fn a_downscale_averages_the_source_cell_it_covers() {
        let mut src = Image::empty(2, 2);
        src.rgba[0..4].copy_from_slice(&[255, 255, 255, 255]);
        let mut dst = Image::black(1, 1);
        dst.blit_downscaled(&src, (0, 0, 2, 2), (0, 0, 1, 1));
        // A quarter of 255 is 63, and over black that is the colour too.
        assert_eq!(dst.pixel(0, 0), Some([63, 63, 63, 255]));

        let mut point = Image::black(1, 1);
        point.blit_scaled(&src, (0, 0, 2, 2), (0, 0, 1, 1));
        assert_eq!(point.pixel(0, 0), Some([255, 255, 255, 255]));
    }

    /// Colour is weighted by alpha, so a transparent neighbour does not drag a
    /// glyph's edge towards whatever colour its unused pixels happen to carry.
    #[test]
    fn a_downscale_does_not_average_in_transparent_colour() {
        let mut src = Image::empty(2, 1);
        src.rgba[0..4].copy_from_slice(&[255, 0, 0, 255]);
        src.rgba[4..8].copy_from_slice(&[0, 0, 255, 0]);
        let mut dst = Image::empty(1, 1);
        dst.blit_downscaled(&src, (0, 0, 2, 1), (0, 0, 1, 1));
        let px = dst.pixel(0, 0).expect("in range");
        assert_eq!(px[3], 127);
        assert_eq!([px[0], px[1], px[2]], [255, 0, 0]);
    }

    #[test]
    fn a_fully_transparent_source_leaves_a_downscale_alone() {
        let src = solid(4, 4, [255, 0, 0, 0]);
        let mut dst = Image::black(2, 2);
        dst.blit_downscaled(&src, (0, 0, 4, 4), (0, 0, 2, 2));
        assert!(dst
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .all(|p| *p == [0, 0, 0, 255]));
    }

    /// A destination hanging off the edge draws what fits, the same as the
    /// point-sampled blit.
    #[test]
    fn a_downscale_clips_rather_than_panicking() {
        let src = solid(8, 8, [255, 255, 255, 255]);
        let mut dst = Image::black(4, 4);
        dst.blit_downscaled(&src, (0, 0, 8, 8), (-2, -2, 4, 4));
        assert_eq!(dst.pixel(0, 0), Some([255, 255, 255, 255]));
        assert_eq!(dst.pixel(3, 3), Some([0, 0, 0, 255]));
    }

    #[test]
    fn blitting_onto_a_transparent_layer_keeps_the_source_alpha() {
        // An overlay is composited onto nothing and blended over the frame by
        // the caller, so a half-transparent source pixel has to stay
        // half-transparent rather than becoming opaque over black.
        let src = solid(1, 1, [200, 100, 50, 128]);
        let mut dst = Image::empty(1, 1);
        dst.blit_scaled(&src, (0, 0, 1, 1), (0, 0, 1, 1));
        let px = dst.pixel(0, 0).expect("in range");
        assert_eq!(px[3], 128);
        assert_eq!([px[0], px[1], px[2]], [200, 100, 50]);
    }

    #[test]
    fn blitting_onto_opaque_black_is_unchanged_by_the_alpha_compositing() {
        // The full-screen path starts from opaque black, where source-over
        // reduces to the straight lerp it always was.
        let src = solid(1, 1, [200, 100, 50, 128]);
        let mut dst = Image::black(1, 1);
        dst.blit_scaled(&src, (0, 0, 1, 1), (0, 0, 1, 1));
        let px = dst.pixel(0, 0).expect("in range");
        assert_eq!(px[3], 255);
        assert_eq!(px[0], (200 * 128 / 255) as u8);
    }

    #[test]
    fn a_scaled_blit_fills_the_destination_rectangle() {
        let src = solid(2, 2, [255, 0, 0, 255]);
        let mut dst = Image::black(8, 8);
        dst.blit_scaled(&src, (0, 0, 2, 2), (2, 2, 4, 4));
        assert_eq!(dst.pixel(3, 3), Some([255, 0, 0, 255]));
        assert_eq!(dst.pixel(1, 1), Some([0, 0, 0, 255]));
        assert_eq!(dst.pixel(6, 6), Some([0, 0, 0, 255]));
    }

    #[test]
    fn a_transparent_source_leaves_the_destination_alone() {
        let src = solid(2, 2, [255, 0, 0, 0]);
        let mut dst = Image::black(4, 4);
        dst.blit_scaled(&src, (0, 0, 2, 2), (0, 0, 4, 4));
        assert!(dst
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .all(|p| *p == [0, 0, 0, 255]));
    }

    #[test]
    fn a_blit_off_the_edge_clips_instead_of_panicking() {
        let src = solid(4, 4, [1, 2, 3, 255]);
        let mut dst = Image::black(4, 4);
        dst.blit_scaled(&src, (0, 0, 4, 4), (-2, -2, 4, 4));
        assert_eq!(dst.pixel(0, 0), Some([1, 2, 3, 255]));
        dst.blit_scaled(&src, (0, 0, 4, 4), (3, 3, 4, 4));
        assert_eq!(dst.pixel(3, 3), Some([1, 2, 3, 255]));
    }

    /// [`blend`]'s two shortcuts are the general formula's own answers, not an
    /// approximation of it, and this is what says so: the general form is
    /// written out once here and every alpha pair is checked against it.
    #[test]
    fn the_divisionless_blends_are_the_general_one() {
        fn general(d: [u8; 4], p: [u8; 4]) -> [u8; 4] {
            let a = u32::from(p[3]);
            if a == 0 {
                return d;
            }
            let ia = 255 - a;
            let da = u32::from(d[3]);
            let out_a = a + da * ia / 255;
            if out_a == 0 {
                return d;
            }
            let mut out = d;
            for (channel, src) in out[..3].iter_mut().zip(p) {
                let c = u32::from(src) * a * 255 + u32::from(*channel) * da * ia;
                *channel = (c / (out_a * 255)) as u8;
            }
            out[3] = out_a as u8;
            out
        }

        for a in 0..=255u8 {
            for da in 0..=255u8 {
                for shade in [0u8, 1, 17, 128, 199, 254, 255] {
                    let src = [shade, 255 - shade, 64, a];
                    let dst = [200, 7, shade, da];
                    let mut got = dst;
                    blend(&mut got, src);
                    assert_eq!(got, general(dst, src), "src {src:?} over dst {dst:?}");
                }
            }
        }
    }
}
