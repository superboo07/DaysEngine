//! A plain RGBA image, PNG decode, and the two operations screens need:
//! nearest-neighbour scaling and an alpha blit.
//!
//! Deliberately small. The UI never needs filtering — every scale factor the
//! game uses (1.0, 1.28, 1.6) is applied to art authored at 800x450, and the
//! original scales it on the GPU with point sampling — and pulling in an
//! imaging crate for a blit and a stretch would not pay for itself.

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

    fn pixel(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let i = (y as usize * self.width as usize + x as usize) * 4;
        self.rgba.get(i..i + 4).map(|p| [p[0], p[1], p[2], p[3]])
    }

    /// Alpha-blends a rectangle of `src` onto `self`, stretching it to `dst`.
    ///
    /// `src_rect` is `(x, y, width, height)` in `src`; `dst` is the destination
    /// rectangle in `self`. Both are clipped rather than checked, so a screen
    /// with a widget hanging off the edge draws what fits.
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
