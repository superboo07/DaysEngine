//! Software compositor for a single frame.
//!
//! The SDL path draws through the GPU, which is not something we can inspect
//! from a test or a build machine. This produces the same frame on the CPU so
//! playback can be checked frame by frame without a display — and so a
//! regression in timing, fades or text layout shows up as an image diff rather
//! than as "it looked wrong when I ran it".

use crate::playback::stage::Visual;
use crate::playback::text;
use days_font::Font;

/// Composites one frame to RGBA at `width` x `height`.
pub fn frame_rgba(visual: &Visual<'_>, font: &Font, width: usize, height: usize) -> Vec<u8> {
    frame_rgba_with(visual, font, width, height, false, false)
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
        blit(
            &mut out,
            width,
            height,
            &Surface {
                pixels: &still.rgba,
                width: still.width as usize,
                height: still.height as usize,
            },
            0,
            0,
        );
    }

    // Mouth patches go over the background and under everything else, because
    // the engine writes them straight into the background's own surface
    // (`FUN_00444b80`) before the frame is composited.
    for (mouth, index) in &visual.mouths {
        blit(
            &mut out,
            width,
            height,
            &Surface {
                pixels: mouth.image(*index),
                width: mouth.width,
                height: mouth.height,
            },
            mouth.x,
            mouth.y,
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

    // Dialogue, broken into lines the way `FUN_0043f600` breaks them and
    // stacked at the recovered pitch. A 62-column line comes to 992 units at
    // the English pitch of 16, so at the half scale everything here is drawn
    // at it lands inside 500 of the stage's 800 pixels — which is what stops
    // long lines running off the edge.
    if let Some((speaker, line)) = visual.text {
        let display = if speaker.is_empty() {
            line.to_string()
        } else {
            format!("{speaker}: {line}")
        };
        let lines = text::wrap(&display, english);
        let pitch = text::LINE_PITCH / 2;
        let block = pitch * lines.len();
        for (n, one) in lines.iter().enumerate() {
            let image = text::render_line(font, one, [255, 255, 255], english);
            let scaled = downscale_half(&image.rgba, image.width, image.height);
            let (sw, sh) = (image.width / 2, image.height / 2);
            let _ = sh;
            let y = height.saturating_sub(block + 8) + n * pitch;
            blit(
                &mut out,
                width,
                height,
                &Surface {
                    pixels: &scaled,
                    width: sw,
                    height: image.height / 2,
                },
                8,
                y,
            );
        }
    }

    // The choice box, when one is up. Its labels are placed by fractions of
    // the frame rather than in pixels: the hit maps are shipped only at 1024x576
    // and 1280x720, so the split -- x for two boxes side by side, y for two
    // stacked -- is what carries over to any size. Which axis is the install's
    // own `[SelectType]` and `[UseEnglish]` answer; see `crate::ui::select`.
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
            let scaled = downscale_half(&image.rgba, image.width, image.height);
            let (sw, sh) = (image.width / 2, image.height / 2);
            let x = ((width as f32 * cx) as usize).saturating_sub(sw / 2);
            let y = ((height as f32 * cy) as usize).saturating_sub(sh / 2);
            blit(
                &mut out,
                width,
                height,
                &Surface {
                    pixels: &scaled,
                    width: sw,
                    height: sh,
                },
                x,
                y,
            );
        }
    }

    out
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

/// Box-filters an RGBA image to half size.
fn downscale_half(src: &[u8], w: usize, h: usize) -> Vec<u8> {
    let (nw, nh) = (w / 2, h / 2);
    let mut out = vec![0u8; nw * nh * 4];
    for y in 0..nh {
        for x in 0..nw {
            let mut acc = [0u32; 4];
            for dy in 0..2 {
                for dx in 0..2 {
                    let s = ((y * 2 + dy) * w + (x * 2 + dx)) * 4;
                    for i in 0..4 {
                        acc[i] += u32::from(src[s + i]);
                    }
                }
            }
            let d = (y * nw + x) * 4;
            for i in 0..4 {
                out[d + i] = (acc[i] / 4) as u8;
            }
        }
    }
    out
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
