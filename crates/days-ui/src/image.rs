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

/// An 8-bit RGBA image.
#[derive(Debug, Clone)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Image {
    /// A transparent image.
    pub fn empty(width: u32, height: u32) -> Image {
        Image {
            width,
            height,
            rgba: vec![0; width as usize * height as usize * 4],
        }
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
        Ok(Image {
            width: info.width,
            height: info.height,
            rgba,
        })
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
        let (sx, sy, sw, sh) = src_rect;
        let (dx0, dy0, dw, dh) = dst;
        if sw == 0 || sh == 0 || dw == 0 || dh == 0 {
            return;
        }
        for row in 0..dh {
            let dy = dy0 + row as i64;
            if dy < 0 || dy >= self.height as i64 {
                continue;
            }
            let y0 = row as u64 * sh as u64 / dh as u64;
            let y1 = (((row as u64 + 1) * sh as u64) / dh as u64).max(y0 + 1);
            for col in 0..dw {
                let dx = dx0 + col as i64;
                if dx < 0 || dx >= self.width as i64 {
                    continue;
                }
                let x0 = col as u64 * sw as u64 / dw as u64;
                let x1 = (((col as u64 + 1) * sw as u64) / dw as u64).max(x0 + 1);

                let mut cells = 0u32;
                let mut alpha = 0u32;
                let mut colour = [0u32; 3];
                for y in y0..y1.min(sh as u64) {
                    for x in x0..x1.min(sw as u64) {
                        let Some(p) = src.pixel(sx + x as u32, sy + y as u32) else {
                            continue;
                        };
                        cells += 1;
                        alpha += u32::from(p[3]);
                        for (sum, c) in colour.iter_mut().zip(p) {
                            *sum += u32::from(c) * u32::from(p[3]);
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
                let ia = 255 - a;
                let d = (dy as usize * self.width as usize + dx as usize) * 4;
                let da = u32::from(self.rgba[d + 3]);
                let out_a = a + da * ia / 255;
                if out_a == 0 {
                    continue;
                }
                for (channel, sum) in self.rgba[d..d + 3].iter_mut().zip(colour) {
                    // `sum / alpha` is the average colour of the covered area,
                    // un-weighted again, then composited as usual.
                    let over = sum / alpha * a * 255;
                    let under = u32::from(*channel) * da * ia;
                    *channel = ((over + under) / (out_a * 255)) as u8;
                }
                self.rgba[d + 3] = out_a as u8;
            }
        }
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
        for row in 0..dh {
            let dy = dy0 + row as i64;
            if dy < 0 || dy >= self.height as i64 {
                continue;
            }
            // Point sampling: the source pixel whose cell covers this row.
            let s_y = sy + (row as u64 * sh as u64 / dh as u64) as u32;
            for col in 0..dw {
                let dx = dx0 + col as i64;
                if dx < 0 || dx >= self.width as i64 {
                    continue;
                }
                let s_x = sx + (col as u64 * sw as u64 / dw as u64) as u32;
                let Some(p) = src.pixel(s_x, s_y) else {
                    continue;
                };
                let a = u32::from(p[3]);
                if a == 0 {
                    continue;
                }
                let d = (dy as usize * self.width as usize + dx as usize) * 4;
                // Source-over, compositing the destination's alpha as well as
                // its colour. On the opaque black a full screen starts from
                // this reduces to `src * a + dst * (255 - a)`, but the in-game
                // overlays — the control bar, whose own art is RGBA — are
                // composited onto a transparent layer and handed to the caller
                // to blend over the frame, and there the destination alpha is
                // the whole point.
                let ia = 255 - a;
                let da = u32::from(self.rgba[d + 3]);
                let out_a = a + da * ia / 255;
                if out_a == 0 {
                    continue;
                }
                for (channel, src) in self.rgba[d..d + 3].iter_mut().zip(p) {
                    let c = u32::from(src) * a * 255 + u32::from(*channel) * da * ia;
                    *channel = (c / (out_a * 255)) as u8;
                }
                self.rgba[d + 3] = out_a as u8;
            }
        }
    }
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
}
