//! Software compositor for a single frame.
//!
//! The SDL path draws through the GPU, which is not something we can inspect
//! from a test or a build machine. This produces the same frame on the CPU so
//! playback can be checked frame by frame without a display — and so a
//! regression in timing, fades or text layout shows up as an image diff rather
//! than as "it looked wrong when I ran it".

use crate::playback::lipsync;
use crate::playback::stage::Visual;
use crate::playback::text;
use crate::ui::select;
use days_font::Font;

/// Composites one frame to RGBA at `width` x `height`.
pub fn frame_rgba(visual: &Visual<'_>, font: &Font, width: usize, height: usize) -> Vec<u8> {
    frame_rgba_with(visual, font, width, height, false, false, false)
}

/// As [`frame_rgba`], with the two answers only the install can give.
///
/// `stacked` is the choice box's axis and `english` is `FILMENGINE.INI`'s
/// `[UseEnglish]`, which decides both the character pitch and whether dialogue
/// wraps at all — see [`crate::playback::text`] and
/// [`crate::ui::select::Layout`]. `frame_rgba` cannot read either for itself
/// because it is handed no INI.
pub fn frame_rgba_with(
    visual: &Visual<'_>,
    font: &Font,
    width: usize,
    height: usize,
    stacked: bool,
    english: bool,
    left_arrangement: bool,
) -> Vec<u8> {
    let mut out = vec![0u8; width * height * 4];
    for px in out.as_chunks_mut::<4>().0 {
        *px = [0, 0, 0, 255];
    }

    if let Some(frame) = visual.movie {
        blit(
            &mut out,
            width,
            height,
            &Surface {
                pixels: &frame.rgba,
                width: frame.width as usize,
                height: frame.height as usize,
            },
            0,
            0,
        );
    } else if let Some(still) = visual.still {
        // The mouths go into the background's own surface first, exactly as
        // `FUN_00444b80` writes them into the original's, and the patched
        // surface is what gets composited. See
        // [`crate::playback::lipsync::compose_mouths`].
        let patched =
            lipsync::compose_mouths(&still.rgba, (still.width, still.height), &visual.mouths);
        blit(
            &mut out,
            width,
            height,
            &Surface {
                pixels: patched.as_deref().unwrap_or(&still.rgba),
                width: still.width as usize,
                height: still.height as usize,
            },
            0,
            0,
        );
    }

    if let Some((colour, opacity)) = visual.fade {
        let a = (opacity.clamp(0.0, 1.0) * 255.0) as u32;
        if a > 0 {
            for px in out.as_chunks_mut::<4>().0 {
                for i in 0..3 {
                    px[i] = ((u32::from(px[i]) * (255 - a) + u32::from(colour[i]) * a) / 255) as u8;
                }
            }
        }
    }

    // Dialogue: broken by `FUN_0043f600`'s rule, placed by `FUN_0044bf30`'s.
    // The speaker field is deliberately not drawn — the original never hands it
    // to the text layer, it goes to the backlog instead.
    if let Some((_speaker, line)) = visual.text {
        let lines = text::wrap(line, english);
        let geometry = text::Geometry::native(left_arrangement);
        for (one, at) in lines.iter().zip(text::place(&lines, english, geometry)) {
            let image = text::render_line(font, one, [255, 255, 255], english);
            blit_stretched(&mut out, width, height, &image, at);
        }
    }

    // The choice box, when one is up. Placed by fractions of the frame rather
    // than by `FUN_0044ced0`'s formula, which `crate::ui::select` records but
    // nothing uses yet: the hit maps are shipped only at 1024x576 and 1280x720,
    // so the split -- x for two boxes side by side, y for two stacked -- is what
    // carries over to any size. Which axis is the install's own `[SelectType]`
    // and `[UseEnglish]` answer.
    if let Some(window) = &visual.select {
        let labels: Vec<&str> = [Some(window.a), window.b]
            .into_iter()
            .flatten()
            .filter(|l| !l.eq_ignore_ascii_case("null"))
            .collect();
        for (index, label) in labels.iter().enumerate() {
            let (cx, cy) = match (labels.len(), stacked) {
                (1, _) => (0.5, 0.5),
                (_, true) => (0.5, 0.25 + 0.5 * index as f32),
                (_, false) => (0.25 + 0.5 * index as f32, 0.5),
            };
            let image = text::render_line(font, label, [255, 255, 255], english);
            // `FUN_0044ced0`'s `h = scale * 48.0`: a label keeps the font's
            // whole cell where a dialogue line is squashed to 42 of it, so the
            // factor into layout space is the geometry's own scale. See
            // `crate::ui::select::label_scale`.
            let k = select::label_scale(text::Geometry::native(left_arrangement));
            let (sw, sh) = (
                (image.width as f32 * k).round().max(1.0) as usize,
                (image.height as f32 * k).round().max(1.0) as usize,
            );
            let x = width as f32 * cx - sw as f32 / 2.0;
            let y = height as f32 * cy - sh as f32 / 2.0;
            blit_stretched(
                &mut out,
                width,
                height,
                &image,
                text::Placement {
                    x,
                    y,
                    width: sw as f32,
                    height: sh as f32,
                },
            );
        }
    }

    out
}

/// Stretches a rendered line into its placement, nearest-sampled.
///
/// The original does this on the GPU by handing the text texture a destination
/// rectangle; the source row is 48 units tall and the destination 42 before the
/// resolution scale, so a line is slightly squashed vertically, and that falls
/// out of using the recovered rectangle rather than being applied separately.
fn blit_stretched(
    dst: &mut [u8],
    dst_w: usize,
    dst_h: usize,
    src: &text::TextImage,
    at: text::Placement,
) {
    let w = at.width.round().max(1.0) as usize;
    let h = at.height.round().max(1.0) as usize;
    for row in 0..h {
        let dy = at.y.round() as i64 + row as i64;
        if dy < 0 || dy as usize >= dst_h {
            continue;
        }
        let sy = row * src.height / h;
        for col in 0..w {
            let dx = at.x.round() as i64 + col as i64;
            if dx < 0 || dx as usize >= dst_w {
                continue;
            }
            let sx = col * src.width / w;
            let s = (sy * src.width + sx) * 4;
            let Some(px) = src.rgba.get(s..s + 4) else {
                continue;
            };
            let a = u32::from(px[3]);
            if a == 0 {
                continue;
            }
            let d = (dy as usize * dst_w + dx as usize) * 4;
            for i in 0..3 {
                let under = u32::from(dst[d + i]);
                dst[d + i] = ((u32::from(px[i]) * a + under * (255 - a)) / 255) as u8;
            }
        }
    }
}

/// An RGBA image with its dimensions.
struct Surface<'a> {
    pixels: &'a [u8],
    width: usize,
    height: usize,
}

/// Alpha-blends `src` onto `dst` at `(x0, y0)`, clipping at the edges.
fn blit(dst: &mut [u8], dst_w: usize, dst_h: usize, src: &Surface<'_>, x0: usize, y0: usize) {
    let Surface {
        pixels: src,
        width: src_w,
        height: src_h,
    } = *src;
    for y in 0..src_h {
        let dy = y0 + y;
        if dy >= dst_h {
            break;
        }
        for x in 0..src_w {
            let dx = x0 + x;
            if dx >= dst_w {
                break;
            }
            let s = (y * src_w + x) * 4;
            let d = (dy * dst_w + dx) * 4;
            let Some(px) = src.get(s..s + 4) else {
                continue;
            };
            let a = u32::from(px[3]);
            if a == 0 {
                continue;
            }
            for i in 0..3 {
                dst[d + i] =
                    ((u32::from(px[i]) * a + u32::from(dst[d + i]) * (255 - a)) / 255) as u8;
            }
            dst[d + 3] = 255;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_visual_is_opaque_black() {
        let font = Font::parse(vec![0u8; days_font::TABLE_BYTES]).unwrap();
        let out = frame_rgba(&Visual::default(), &font, 4, 4);
        assert!(out.as_chunks::<4>().0.iter().all(|p| *p == [0, 0, 0, 255]));
    }

    #[test]
    fn a_full_white_fade_covers_everything() {
        let font = Font::parse(vec![0u8; days_font::TABLE_BYTES]).unwrap();
        let visual = Visual {
            fade: Some(([255, 255, 255], 1.0)),
            ..Default::default()
        };
        let out = frame_rgba(&visual, &font, 4, 4);
        assert!(out
            .as_chunks::<4>()
            .0
            .iter()
            .all(|p| *p == [255, 255, 255, 255]));
    }

    #[test]
    fn blit_clips_at_the_edges_rather_than_panicking() {
        let mut dst = vec![0u8; 4 * 4 * 4];
        let src = vec![255u8; 4 * 4 * 4];
        blit(
            &mut dst,
            4,
            4,
            &Surface {
                pixels: &src,
                width: 4,
                height: 4,
            },
            2,
            2,
        );
        assert_eq!(dst[(3 * 4 + 3) * 4], 255);
    }
}
