//! Timeline playback: what is on screen and what is audible at a given frame.
//!
//! The engine holds one clock over the whole script. Each statement carries a
//! `[start, end)` window, so [`Stage::seek_to`] fires everything whose start it
//! has just crossed and then reports the visual state for that frame.
//!
//! Assets are loaded when their statement fires rather than up front: a script
//! can reference a hundred movies and thirty minutes of voice, and the game only
//! ever needs the next few seconds.

use crate::media::{AudioBuffer, VideoDecoder, VideoFrame};
use crate::vfs::Vfs;
use anyhow::{Context, Result};
use days_script::{Command, Fade, Frame, Script};
use std::collections::HashMap;
use std::sync::Arc;

use crate::Mixer;

/// A movie being played, with its position on the script timeline.
struct Movie {
    decoder: VideoDecoder,
    /// Script frame at which this movie's frame 0 shows.
    start: Frame,
    /// Index the decoder will return next.
    next_index: u64,
    /// Most recently decoded frame, held so a stalled or finished movie keeps
    /// showing its last image rather than flashing.
    current: Option<VideoFrame>,
    path: String,
}

/// A still background image, decoded from PNG.
pub struct Still {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub path: String,
}

/// What should be drawn for the current frame.
#[derive(Default)]
pub struct Visual<'a> {
    /// The movie frame to show, if a movie is playing.
    pub movie: Option<&'a VideoFrame>,
    /// The still background to show, if one is set and no movie covers it.
    pub still: Option<&'a Still>,
    /// Speaker and line of the active `[PrintText]`.
    pub text: Option<(&'a str, &'a str)>,
    /// Choice labels of the active `[SetSELECT]`.
    pub choices: Option<(&'a str, Option<&'a str>)>,
    /// Fade overlay: colour and opacity in `0.0..=1.0`.
    pub fade: Option<([u8; 3], f32)>,
}

/// Tracks a fade in progress.
struct FadeState {
    colour: [u8; 3],
    start: Frame,
    end: Frame,
    direction: Fade,
}

impl FadeState {
    /// Opacity of the overlay at `at`.
    ///
    /// `IN` means "fade from the colour into the scene", so the overlay starts
    /// opaque and clears; `OUT` is the reverse. Past the window the fade holds
    /// its final value rather than snapping back, because scripts rely on a
    /// `BlackFade OUT` leaving the screen black until the next statement paints.
    fn opacity(&self, at: Frame) -> f32 {
        let span = self.end.0.saturating_sub(self.start.0);
        let progress = if span == 0 {
            1.0
        } else {
            ((at.0.saturating_sub(self.start.0)) as f32 / span as f32).clamp(0.0, 1.0)
        };
        match self.direction {
            Fade::In => 1.0 - progress,
            Fade::Out => progress,
        }
    }
}

/// One script, playing.
pub struct Stage {
    script: Script,
    /// Frame most recently played. Events are fired for `(played, target]`.
    played: Option<Frame>,
    movie: Option<Movie>,
    still: Option<Still>,
    fade: Option<FadeState>,
    /// Decoded audio kept alive so repeated plays do not re-decode. Voice lines
    /// are one-shot but SE and BGM repeat constantly within a scene.
    audio_cache: HashMap<String, Arc<AudioBuffer>>,
}

impl Stage {
    pub fn new(script: Script) -> Stage {
        Stage {
            script,
            played: None,
            movie: None,
            still: None,
            fade: None,
            audio_cache: HashMap::new(),
        }
    }

    pub fn script(&self) -> &Script {
        &self.script
    }

    pub fn length(&self) -> Frame {
        self.script.length
    }

    /// True once the clock has passed the end of the script.
    pub fn finished(&self, at: Frame) -> bool {
        at >= self.script.length
    }

    /// Advances to `target`, firing every statement that starts in between.
    ///
    /// Only forward motion is supported; a backward target restarts from the
    /// beginning, which is what the skip/replay controls need.
    pub fn seek_to(&mut self, target: Frame, vfs: &Vfs, mixer: &Mixer) -> Result<()> {
        if self.played.is_some_and(|p| target < p) {
            self.reset(mixer);
        }

        let after = self.played.unwrap_or(Frame::ZERO);
        let first = self.played.is_none();

        // Collected first so the borrow of `self.script` ends before dispatch.
        let due: Vec<_> = self
            .script
            .events
            .iter()
            .filter(|e| {
                if first {
                    e.start <= target
                } else {
                    e.start > after && e.start <= target
                }
            })
            .cloned()
            .collect();

        for event in due {
            if let Err(err) = self.dispatch(&event.command, event.start, event.end, vfs, mixer) {
                // A missing asset must not end playback: 166 voice references in
                // the retail scripts point at clips that were never recorded.
                log::warn!("{}: {err:#}", self.script.name);
            }
        }

        self.played = Some(target);
        Ok(())
    }

    fn reset(&mut self, mixer: &Mixer) {
        mixer.stop_all();
        self.movie = None;
        self.still = None;
        self.fade = None;
        self.played = None;
    }

    fn dispatch(
        &mut self,
        command: &Command,
        start: Frame,
        end: Frame,
        vfs: &Vfs,
        mixer: &Mixer,
    ) -> Result<()> {
        match command {
            Command::PlayMovie { path, looping } => {
                if *looping {
                    // No retail statement sets this; flag it if one ever does.
                    log::warn!("[PlayMovie] {path} requests looping, which is unimplemented");
                }
                let bytes = vfs
                    .read_path_as(path, "wmv")
                    .with_context(|| format!("loading movie {path}"))?;
                let decoder =
                    VideoDecoder::open(bytes).with_context(|| format!("opening movie {path}"))?;
                self.movie = Some(Movie {
                    decoder,
                    start,
                    next_index: 0,
                    current: None,
                    path: path.clone(),
                });
                // A movie covers the whole frame, so any still under it is done.
                self.still = None;
            }
            Command::CreateBg { kind, path } => {
                if kind != "BGS" {
                    log::warn!("[CreateBG] unknown kind {kind:?} for {path}");
                }
                self.still = Some(load_still(vfs, path)?);
                self.movie = None;
            }
            Command::PlayBgm { path } | Command::EndBgm { path } => {
                let bgm = vfs
                    .resolve_bgm(path)
                    .with_context(|| format!("resolving BGM {path}"))?;
                let intro = match bgm.intro {
                    Some(h) if Some(h) != Some(bgm.looped) => Some(self.audio(vfs, h)?),
                    _ => None,
                };
                let looped = self.audio(vfs, bgm.looped)?;
                mixer.play_bgm(intro, looped);
            }
            Command::PlaySe { slot, path } => {
                let buffer = self.audio_by_path(vfs, path)?;
                mixer.play_se(*slot, buffer);
            }
            Command::PlayVoice { path, .. } => {
                let buffer = self.audio_by_path(vfs, path)?;
                mixer.play_voice(buffer);
            }
            Command::BlackFade(direction) => {
                self.fade = Some(FadeState {
                    colour: [0, 0, 0],
                    start,
                    end,
                    direction: *direction,
                });
            }
            Command::WhiteFade(direction) => {
                self.fade = Some(FadeState {
                    colour: [255, 255, 255],
                    start,
                    end,
                    direction: *direction,
                });
            }
            Command::EndRoll { path } => {
                let bytes = vfs.read_path_as(path, "wmv")?;
                let decoder = VideoDecoder::open(bytes)?;
                self.movie = Some(Movie {
                    decoder,
                    start,
                    next_index: 0,
                    current: None,
                    path: path.clone(),
                });
            }
            // Text and choices are read out of the script by `visual_at` rather
            // than latched here, so seeking lands mid-line correctly.
            Command::PrintText { .. } | Command::SetSelect { .. } => {}
            // Peripheral control; no hardware, nothing to do.
            Command::MoveSom { .. } => {}
            Command::SkipFrame | Command::Next => {}
        }
        Ok(())
    }

    fn audio(&mut self, vfs: &Vfs, handle: crate::vfs::Handle) -> Result<Arc<AudioBuffer>> {
        let key = vfs.entry(handle).name.clone();
        if let Some(cached) = self.audio_cache.get(&key) {
            return Ok(cached.clone());
        }
        let bytes = vfs.read(handle)?;
        let buffer =
            Arc::new(crate::media::decode_audio(bytes).with_context(|| format!("decoding {key}"))?);
        self.audio_cache.insert(key, buffer.clone());
        Ok(buffer)
    }

    fn audio_by_path(&mut self, vfs: &Vfs, path: &str) -> Result<Arc<AudioBuffer>> {
        let handle = vfs
            .resolve_as(path, "ogg")
            .with_context(|| format!("resolving audio {path}"))?;
        self.audio(vfs, handle)
    }

    /// Decodes forward so the movie shows the right frame for `at`, then reports
    /// everything that should be drawn.
    pub fn visual_at(&mut self, at: Frame) -> Visual<'_> {
        if let Some(movie) = &mut self.movie {
            let wanted = u64::from(at.0.saturating_sub(movie.start.0));
            while movie.next_index <= wanted {
                match movie.decoder.next_frame() {
                    Ok(Some(frame)) => {
                        movie.next_index = frame.index + 1;
                        movie.current = Some(frame);
                    }
                    Ok(None) => break, // Clip exhausted; hold the last frame.
                    Err(err) => {
                        log::warn!("decoding {}: {err}", movie.path);
                        break;
                    }
                }
            }
        }

        let mut visual = Visual {
            movie: self.movie.as_ref().and_then(|m| m.current.as_ref()),
            still: self.still.as_ref(),
            fade: self.fade.as_ref().map(|f| (f.colour, f.opacity(at))),
            ..Default::default()
        };

        for event in self.script.events.iter().filter(|e| e.is_active_at(at)) {
            match &event.command {
                Command::PrintText { speaker, text } => {
                    visual.text = Some((speaker.as_str(), text.as_str()))
                }
                Command::SetSelect { a, b } => visual.choices = Some((a.as_str(), b.as_deref())),
                _ => {}
            }
        }
        visual
    }
}

/// Decodes a background PNG to RGBA.
fn load_still(vfs: &Vfs, path: &str) -> Result<Still> {
    let bytes = vfs
        .read_path_as(path, "png")
        .with_context(|| format!("loading background {path}"))?;
    let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    let mut reader = decoder
        .read_info()
        .with_context(|| format!("reading PNG header of {path}"))?;
    let mut buf = vec![0u8; reader.output_buffer_size().unwrap_or(0)];
    let info = reader
        .next_frame(&mut buf)
        .with_context(|| format!("decoding {path}"))?;

    // Backgrounds are colour type 6 (RGBA) in the retail packs, but expand
    // anything else rather than assuming.
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf[..info.buffer_size()].to_vec(),
        png::ColorType::Rgb => buf[..info.buffer_size()]
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[1], p[2], 0xff])
            .collect(),
        other => anyhow::bail!("{path} has unsupported PNG colour type {other:?}"),
    };

    Ok(Still {
        width: info.width,
        height: info.height,
        rgba,
        path: path.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use days_script::Script;

    #[test]
    fn fade_in_clears_the_overlay_and_fade_out_fills_it() {
        let f = FadeState {
            colour: [0, 0, 0],
            start: Frame(0),
            end: Frame(10),
            direction: Fade::In,
        };
        assert_eq!(f.opacity(Frame(0)), 1.0);
        assert_eq!(f.opacity(Frame(5)), 0.5);
        assert_eq!(f.opacity(Frame(10)), 0.0);
        // Holds past the end rather than snapping back.
        assert_eq!(f.opacity(Frame(50)), 0.0);

        let f = FadeState {
            direction: Fade::Out,
            ..f
        };
        assert_eq!(f.opacity(Frame(0)), 0.0);
        assert_eq!(f.opacity(Frame(10)), 1.0);
        assert_eq!(f.opacity(Frame(50)), 1.0);
    }

    /// A zero-length fade is instant, not a divide by zero.
    #[test]
    fn zero_length_fade_is_instant() {
        let f = FadeState {
            colour: [0, 0, 0],
            start: Frame(7),
            end: Frame(7),
            direction: Fade::Out,
        };
        assert_eq!(f.opacity(Frame(7)), 1.0);
    }

    #[test]
    fn text_is_reported_only_inside_its_window() {
        let script = Script::parse_str(
            "t",
            "[SkipFRAME]=00:10:00;\n\
             [PrintText]=00:01:00\tMakoto\tHello.\t00:02:00;\n\
             [Next]=00:10:00;\n",
        )
        .unwrap();
        let mut stage = Stage::new(script);
        assert!(stage
            .visual_at(Frame::parse("00:00:12").unwrap())
            .text
            .is_none());
        assert_eq!(
            stage.visual_at(Frame::parse("00:01:12").unwrap()).text,
            Some(("Makoto", "Hello."))
        );
        assert!(stage
            .visual_at(Frame::parse("00:02:00").unwrap())
            .text
            .is_none());
    }
}
