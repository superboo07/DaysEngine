//! Lip sync: mouth overlays patched into a still background.
//!
//! When a `[PlayVoice]` statement names a speaker tag, the engine looks beside
//! the current background for three small PNGs and flaps the character's mouth
//! with them for as long as the clip is audible. The art lives in the `EventNN`
//! packs next to the background frame it belongs to:
//!
//! ```text
//! Event00/00-00/00-00-A02/00-00-A02-001B.PNG        the background
//! Event00/00-00/00-00-A02/00-00-A02-001BMAK.A.PNG   Makoto's mouth, closed
//! Event00/00-00/00-00-A02/00-00-A02-001BMAK.B.PNG
//! Event00/00-00/00-00-A02/00-00-A02-001BMAK.C.PNG
//! ```
//!
//! Everything here was recovered from `SCHOOLDAYS HQ.exe`. The background is a
//! `FILMOBJ::ImageChar` (vftable `0x004d535c`, constructed at `FUN_00443900`),
//! and the pieces are:
//!
//! * `FUN_00438de0` — the statement dispatcher. Its `[PlayVoice]` arm hands the
//!   current `BGS<n>` object the statement's speaker tag through vtable slot
//!   `+0x90`; its `[CreateBG]` arm walks the live voice list and hands the new
//!   background every tag already speaking, which is why a background change
//!   mid-line picks the mouth up again.
//! * `FUN_004453e0` (slot `+0x90`) — loads the three images for one tag. The
//!   path is the background's own path with the tag appended, then `.A`, `.B`
//!   or `.C`, then `.png` (`FUN_00444f70`, literals at `0x004d5268`,
//!   `0x004d5270`, `0x004d5278`). All three must load or the tag goes into a
//!   per-background reject set and that speaker simply never flaps here.
//! * `FUN_00445240` — derives the patch rectangle from the first image's alpha.
//! * `FUN_00444b80` — copies that rectangle straight into the background's
//!   surface. It is a `memcpy`, not an alpha blend; the shipped overlays have
//!   binary alpha and a solid opaque rectangle, so the two agree.
//! * `FUN_00444cf0` — the per-frame cadence, called from the object's update
//!   (`FUN_00443e30`).
//!
//! The tag is appended to the path exactly as the script spells it (lowercase);
//! the packs store the names uppercase, and pack lookups are case-insensitive.
//!
//! Only still backgrounds are handled. `FILMOBJ::MovieChar` carries the same
//! ten lip-sync slots at `+0xe4` (`FUN_0044a2c0`), but no `MovieNN` pack in the
//! retail install contains a single `.A`/`.B`/`.C` overlay, so nothing can
//! drive it.

use crate::install::vfs::Vfs;
use crate::media::AudioBuffer;
use anyhow::{Context, Result};
use days_script::Frame;

/// Canvas the retail engine scans for the patch rectangle, from the loop bounds
/// in `FUN_00445240`. Overlays that are not this size are rejected rather than
/// scanned with the wrong stride.
pub const CANVAS: (usize, usize) = (800, 452);

/// Filename pieces for the three mouth images, in the order `FUN_00444f70`
/// indexes them.
const SUFFIXES: [&str; 3] = [".A", ".B", ".C"];

/// One speaker's three mouth images for one background, cropped to the patch
/// rectangle.
pub struct Mouth {
    /// Patch rectangle on the 800x452 background.
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
    /// `width * height * 4` RGBA bytes each, in `.A`, `.B`, `.C` order.
    pub images: [Vec<u8>; 3],
}

impl Mouth {
    /// Loads the mouth set for `tag` beside the background at `background`.
    ///
    /// Returns `Ok(None)` when the set is not complete, which is the ordinary
    /// case: most backgrounds carry no overlays at all, and `FUN_004453e0`
    /// discards a partial set the same way.
    pub fn load(vfs: &Vfs, background: &str, tag: &str) -> Result<Option<Mouth>> {
        let stem = format!("{background}{tag}");
        let mut full = Vec::with_capacity(3);
        for suffix in SUFFIXES {
            let path = format!("{stem}{suffix}");
            let Some(image) = read_overlay(vfs, &path)? else {
                return Ok(None);
            };
            full.push(image);
        }

        let Some((x, y, width, height)) = patch_rect(&full[0]) else {
            log::warn!("{stem}.A is fully transparent; no mouth to patch");
            return Ok(None);
        };

        let images = full
            .iter()
            .map(|image| crop(image, x, y, width, height))
            .collect::<Vec<_>>();
        let [a, b, c] = <[Vec<u8>; 3]>::try_from(images).expect("three suffixes give three images");
        Ok(Some(Mouth {
            x,
            y,
            width,
            height,
            images: [a, b, c],
        }))
    }

    /// The rectangle to draw for mouth image `index`.
    pub fn image(&self, index: usize) -> &[u8] {
        &self.images[index % 3]
    }
}

/// Reads one overlay PNG as full-canvas RGBA, or `None` if it is not there.
fn read_overlay(vfs: &Vfs, path: &str) -> Result<Option<Vec<u8>>> {
    let Ok(bytes) = vfs.read_path_as(path, "png") else {
        return Ok(None);
    };
    let mut reader = png::Decoder::new(std::io::Cursor::new(bytes))
        .read_info()
        .with_context(|| format!("reading PNG header of {path}"))?;
    let mut buf = vec![0u8; reader.output_buffer_size().unwrap_or(0)];
    let info = reader
        .next_frame(&mut buf)
        .with_context(|| format!("decoding {path}"))?;
    if info.color_type != png::ColorType::Rgba {
        anyhow::bail!("overlay {path} is {:?}, not RGBA", info.color_type);
    }
    if (info.width as usize, info.height as usize) != CANVAS {
        anyhow::bail!(
            "overlay {path} is {}x{}, not {}x{}",
            info.width,
            info.height,
            CANVAS.0,
            CANVAS.1
        );
    }
    buf.truncate(info.buffer_size());
    Ok(Some(buf))
}

/// Finds the rectangle the engine patches, from `FUN_00445240`.
///
/// It is not a bounding box of every opaque pixel. The scan takes the first
/// opaque pixel in raster order as the corner, counts the opaque pixels on that
/// row for the width, and counts how far column `x` stays opaque for the
/// height — then stops looking. That is only the same thing because the shipped
/// overlays are a solid opaque rectangle on a fully transparent canvas, which
/// is also what makes the engine's straight `memcpy` of the region correct.
fn patch_rect(rgba: &[u8]) -> Option<(usize, usize, usize, usize)> {
    let (canvas_w, canvas_h) = CANVAS;
    let alpha = |x: usize, y: usize| rgba[(y * canvas_w + x) * 4 + 3];

    let (mut x, mut y) = (0, 0);
    let (mut width, mut height) = (0, 0);
    let (mut found_corner, mut row_done, mut column_started) = (false, false, false);

    for row in 0..canvas_h {
        if !row_done {
            for col in 0..canvas_w {
                if alpha(col, row) == 0 {
                    if found_corner {
                        row_done = true;
                    }
                } else {
                    if !found_corner {
                        (x, y) = (col, row);
                        found_corner = true;
                    }
                    width += 1;
                }
            }
        }
        if alpha(x, row) == 0 {
            if column_started {
                break;
            }
        } else {
            height += 1;
            column_started = true;
        }
    }

    found_corner.then_some((x, y, width, height))
}

/// Cuts the patch rectangle out of a full-canvas overlay.
fn crop(rgba: &[u8], x: usize, y: usize, width: usize, height: usize) -> Vec<u8> {
    let stride = CANVAS.0 * 4;
    let mut out = Vec::with_capacity(width * height * 4);
    for row in y..y + height {
        let start = row * stride + x * 4;
        out.extend_from_slice(&rgba[start..start + width * 4]);
    }
    out
}

/// Per-frame "is this clip making a sound" flags for one voice line.
///
/// The retail engine builds this while the clip decodes (`FUN_0041ba90`): it
/// appends one flag per tick, taking a single 16-bit sample and calling it
/// silent when it lies in `-59..=60`. The flag is then looked up by the elapsed
/// frame count, converted to 100ns units with `FUN_00428140` — whose divisor
/// `DAT_0050c468` is the engine-wide 24 fps, written once at `0x0044a623` and
/// also what the `MM:SS:FF` parser multiplies by.
///
/// **Which sample the retail build tests is not reproducible.** The index it
/// computes is relative to the decoder's current packet
/// (`packet_bytes * n / 23 - consumed_samples`), so it depends on how the
/// shipped Ogg reader happens to chunk the stream. We sample at the frame's own
/// position instead, and keep the threshold, which is what decides the result
/// for anything but a sample or two around an edge.
pub struct Envelope {
    voiced: Vec<bool>,
}

/// Silence window on a 16-bit sample, from the comparisons in `FUN_0041ba90`.
const SILENT_MIN: i32 = -59;
const SILENT_MAX: i32 = 60;

impl Envelope {
    /// Samples `audio` once per 24 fps frame.
    pub fn from_audio(audio: &AudioBuffer) -> Envelope {
        let rate = f64::from(crate::media::SAMPLE_RATE);
        let total = audio.frames();
        let ticks = (audio.duration_seconds() * f64::from(FPS)).ceil() as usize;
        let voiced = (0..ticks)
            .map(|n| {
                let at = (n as f64 * rate / f64::from(FPS)).round() as usize;
                if at >= total {
                    return false;
                }
                let sample = audio.samples[at * AudioBuffer::CHANNELS];
                let pcm = (sample.clamp(-1.0, 1.0) * 32768.0) as i32;
                !(SILENT_MIN..=SILENT_MAX).contains(&pcm)
            })
            .collect();
        Envelope { voiced }
    }

    /// Whether the clip is audible `elapsed` frames after it started.
    ///
    /// Past the end of the clip this is false, matching the retail lookup:
    /// `FUN_0041a830` walks the flag list for an exact timestamp match and
    /// returns zero when it runs off the end.
    pub fn voiced(&self, elapsed: u32) -> bool {
        self.voiced.get(elapsed as usize).copied().unwrap_or(false)
    }
}

/// The engine's frame rate, and the rate the envelope is sampled at.
pub const FPS: u32 = 24;

/// Which mouth image is showing at `at` for a line that started at `start`.
///
/// From `FUN_00444cf0`. A phase counter steps on every frame whose number is
/// divisible by three — so the mouth changes at 8 Hz, phase-locked to the
/// engine clock rather than to the line — and the image is that counter modulo
/// three. The counter only steps while the clip is audible, with one exception:
/// once it has left image `.A` it keeps stepping through silence until it comes
/// back round to `.A`, so the mouth always closes rather than freezing open.
pub fn image_index(envelope: &Envelope, start: Frame, at: Frame) -> usize {
    let mut phase = 0usize;
    for frame in start.0..=at.0 {
        if frame.is_multiple_of(3) && (envelope.voiced(frame - start.0) || !phase.is_multiple_of(3))
        {
            phase += 1;
        }
    }
    phase % 3
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a canvas with one solid opaque rectangle on it.
    fn canvas_with(x: usize, y: usize, w: usize, h: usize) -> Vec<u8> {
        let (cw, ch) = CANVAS;
        let mut rgba = vec![0u8; cw * ch * 4];
        for row in y..y + h {
            for col in x..x + w {
                rgba[(row * cw + col) * 4 + 3] = 255;
            }
        }
        rgba
    }

    #[test]
    fn patch_rect_finds_a_solid_rectangle() {
        assert_eq!(
            patch_rect(&canvas_with(392, 160, 48, 43)),
            Some((392, 160, 48, 43))
        );
    }

    #[test]
    fn a_fully_transparent_overlay_has_no_patch_rectangle() {
        assert_eq!(patch_rect(&vec![0u8; CANVAS.0 * CANVAS.1 * 4]), None);
    }

    #[test]
    fn crop_takes_only_the_rectangle() {
        let out = crop(&canvas_with(10, 5, 3, 2), 10, 5, 3, 2);
        assert_eq!(out.len(), 3 * 2 * 4);
        assert!(out.as_chunks::<4>().0.iter().all(|px| px[3] == 255));
    }

    fn envelope(flags: &[bool]) -> Envelope {
        Envelope {
            voiced: flags.to_vec(),
        }
    }

    /// The image steps once every three frames while the clip is audible.
    #[test]
    fn the_mouth_cycles_every_third_frame() {
        let env = envelope(&[true; 24]);
        let got: Vec<_> = (0..12)
            .map(|f| image_index(&env, Frame(0), Frame(f)))
            .collect();
        assert_eq!(got, [1, 1, 1, 2, 2, 2, 0, 0, 0, 1, 1, 1]);
    }

    /// Silence at the start leaves the mouth closed rather than flapping.
    #[test]
    fn silence_holds_the_mouth_shut() {
        let env = envelope(&[false; 24]);
        assert!((0..12).all(|f| image_index(&env, Frame(0), Frame(f)) == 0));
    }

    /// Once open, the cycle runs to the end even if the clip goes quiet, so the
    /// mouth closes instead of freezing part-way.
    #[test]
    fn an_open_mouth_finishes_its_cycle_through_silence() {
        let mut flags = [false; 24];
        flags[0] = true;
        let env = envelope(&flags);
        // Frame 0 opens it; frames 3 and 6 carry it round to closed, and it
        // then stays closed.
        assert_eq!(image_index(&env, Frame(0), Frame(0)), 1);
        assert_eq!(image_index(&env, Frame(0), Frame(3)), 2);
        assert_eq!(image_index(&env, Frame(0), Frame(6)), 0);
        assert_eq!(image_index(&env, Frame(0), Frame(9)), 0);
    }

    /// Past the end of the clip there are no flags, which reads as silence.
    #[test]
    fn a_finished_clip_reads_as_silent() {
        let env = envelope(&[true, true]);
        assert!(!env.voiced(5));
    }

    /// The phase is locked to the engine clock, not to the line, so a line that
    /// starts off the beat waits for the next multiple of three.
    #[test]
    fn the_cycle_is_locked_to_the_engine_clock() {
        let env = envelope(&[true; 24]);
        assert_eq!(image_index(&env, Frame(1), Frame(1)), 0);
        assert_eq!(image_index(&env, Frame(1), Frame(2)), 0);
        assert_eq!(image_index(&env, Frame(1), Frame(3)), 1);
    }

    #[test]
    fn the_silence_window_is_the_engines() {
        let mut audio = AudioBuffer::silent();
        // Three 24 fps frames: silent, just inside the window, just outside.
        let per_frame = crate::media::SAMPLE_RATE as usize / FPS as usize;
        audio.samples = vec![0.0; per_frame * 3 * AudioBuffer::CHANNELS];
        let set = |audio: &mut AudioBuffer, n: usize, pcm: i32| {
            let at =
                (n as f64 * f64::from(crate::media::SAMPLE_RATE) / f64::from(FPS)).round() as usize;
            audio.samples[at * AudioBuffer::CHANNELS] = pcm as f32 / 32768.0;
        };
        set(&mut audio, 0, 0);
        set(&mut audio, 1, SILENT_MAX);
        set(&mut audio, 2, SILENT_MAX + 1);
        let env = Envelope::from_audio(&audio);
        assert!(!env.voiced(0));
        assert!(!env.voiced(1));
        assert!(env.voiced(2));
    }
}
