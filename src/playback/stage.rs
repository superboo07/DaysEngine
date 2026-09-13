//! Timeline playback: what is on screen and what is audible at a given frame.
//!
//! The engine holds one clock over the whole script. Each statement carries a
//! `[start, end)` window, so [`Stage::seek_to`] fires everything whose start it
//! has just crossed and then reports the visual state for that frame.
//!
//! Assets are loaded when their statement fires rather than up front: a script
//! can reference a hundred movies and thirty minutes of voice, and the game only
//! ever needs the next few seconds.

use crate::install::vfs::Vfs;
use crate::media::{AudioBuffer, VideoDecoder, VideoFrame};
use crate::playback::lipsync::{self, Envelope, Mouth};
use anyhow::{Context, Result};
use days_script::{Command, Fade, Frame, Script};
use std::collections::{BTreeMap, HashMap};
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
    /// Which picture [`Self::movie`] is: the clip's path and the frame's
    /// 0-based index in it.
    ///
    /// The playback loop runs far faster than 24 fps — it has to, to stay
    /// responsive to the pointer — so it asks for the visual many times per
    /// movie frame and would resample the same picture over and over. Two
    /// visuals carrying the same pair here are the same picture, and the second
    /// one needs no work at all.
    pub movie_id: Option<(&'a str, u64)>,
    /// The still background to show, if one is set and no movie covers it.
    pub still: Option<&'a Still>,
    /// Speaker and line of the active `[PrintText]`.
    pub text: Option<(&'a str, &'a str)>,
    /// The active `[SetSELECT]`, with the window it owns.
    ///
    /// The window is part of it because the choice box is driven by it: the
    /// original raises the box at `start` and decides for the player once
    /// `frame + 1` reaches `end`, so a caller given only the labels could not
    /// reproduce the timeout. See [`crate::ui::select`].
    pub select: Option<SelectWindow<'a>>,
    /// Fade overlay: colour and opacity in `0.0..=1.0`.
    pub fade: Option<([u8; 3], f32)>,
    /// Mouth patches to draw over the still, each with the index of the image
    /// showing this frame. Empty unless a tagged voice line is speaking over a
    /// background that ships overlays for it.
    pub mouths: Vec<(&'a Mouth, usize)>,
}

/// The active `[SetSELECT]`: its labels and the frames it runs between.
#[derive(Debug, Clone, Copy)]
pub struct SelectWindow<'a> {
    pub a: &'a str,
    pub b: Option<&'a str>,
    pub start: Frame,
    pub end: Frame,
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
    /// Voice lines that could drive a mouth, keyed by speaker tag. The retail
    /// engine keeps ten slots on the background object and matches an incoming
    /// line against them by tag (`FUN_004448f0`), so a tag has one line at a
    /// time.
    voices: BTreeMap<String, VoiceLine>,
    /// The player's `MenVoice` option. `FUN_0044e800` drops a clip whose
    /// `[PlayVoice]` male-voice flag is set when this is off; the option's
    /// default is on.
    men_voice: bool,
    /// Mouth art for the current background, by tag. A `None` is a tag this
    /// background has no complete set for — the engine caches that rejection
    /// per background too, and re-checking it every frame would mean three
    /// failed pack lookups per speaker per frame.
    mouths: BTreeMap<String, Option<Mouth>>,
    /// The filter movie frames are scaled with, from `DaysEngine.ini`.
    video_scaler: crate::media::VideoScaler,
    /// The size movie frames are wanted at, or `None` for the clip's own.
    ///
    /// The window is almost never 800x452, and a frame has to be scaled to it
    /// somewhere. Here is the cheapest place: libswscale is already converting
    /// every frame out of the codec's colour space, so the scale is folded into
    /// a pass the frame was making anyway. See
    /// [`crate::media::VideoDecoder::set_output_size`].
    video_size: Option<(u32, u32)>,
}

/// A voice line that may be flapping a mouth.
struct VoiceLine {
    start: Frame,
    end: Frame,
    envelope: Envelope,
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
            men_voice: true,
            voices: BTreeMap::new(),
            mouths: BTreeMap::new(),
            video_scaler: crate::media::VideoScaler::default(),
            video_size: None,
        }
    }

    /// Asks for movie frames at `width` x `height`, applying it to whatever is
    /// playing and to every clip this stage opens afterwards.
    ///
    /// A size the decoder will not take is reported and otherwise ignored: a
    /// movie at the wrong size is still a movie, and losing playback over it
    /// would be worse.
    pub fn set_video_size(&mut self, width: u32, height: u32) {
        if self.video_size == Some((width, height)) {
            return;
        }
        self.video_size = Some((width, height));
        if let Some(movie) = &mut self.movie {
            if let Err(err) = movie.decoder.set_output_size(width, height) {
                log::warn!("{}: cannot decode at {width}x{height}: {err}", movie.path);
            }
        }
    }

    /// Chooses the filter movie frames are scaled with, applying it to whatever
    /// is playing and to every clip this stage opens afterwards.
    pub fn set_video_scaler(&mut self, scaler: crate::media::VideoScaler) {
        if self.video_scaler == scaler {
            return;
        }
        self.video_scaler = scaler;
        if let Some(movie) = &mut self.movie {
            if let Err(err) = movie.decoder.set_scaler(scaler) {
                log::warn!("{}: cannot scale with {scaler:?}: {err}", movie.path);
            }
        }
    }

    /// Puts this stage's video settings on a freshly opened clip.
    fn size_video(&self, decoder: &mut VideoDecoder, path: &str) {
        if let Err(err) = decoder.set_scaler(self.video_scaler) {
            log::warn!("{path}: cannot scale with {:?}: {err}", self.video_scaler);
        }
        let Some((width, height)) = self.video_size else {
            return;
        };
        if let Err(err) = decoder.set_output_size(width, height) {
            log::warn!("{path}: cannot decode at {width}x{height}: {err}");
        }
    }

    /// Sets the player's `MenVoice` option, which gates male voice clips.
    pub fn set_men_voice(&mut self, on: bool) {
        self.men_voice = on;
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
        self.voices.clear();
        self.mouths.clear();
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
                let mut decoder =
                    VideoDecoder::open(bytes).with_context(|| format!("opening movie {path}"))?;
                self.size_video(&mut decoder, path);
                self.movie = Some(Movie {
                    decoder,
                    start,
                    next_index: 0,
                    current: None,
                    path: path.clone(),
                });
                // A movie covers the whole frame, so the still under it is
                // done.
                self.drop_background();
            }
            Command::CreateBg { kind, path } => {
                if kind != "BGS" {
                    log::warn!("[CreateBG] unknown kind {kind:?} for {path}");
                }
                self.still = Some(load_still(vfs, path)?);
                self.movie = None;
                // The overlays belong to the background, so a new one starts
                // with none loaded — and then picks up every line still
                // speaking, which is what lets a mouth carry across a
                // background change mid-sentence (`FUN_00438de0`'s
                // `[CreateBG]` arm).
                self.mouths.clear();
                let speaking: Vec<String> = self
                    .voices
                    .iter()
                    .filter(|(_, v)| start >= v.start && start < v.end)
                    .map(|(tag, _)| tag.clone())
                    .collect();
                for tag in speaking {
                    self.load_mouth(vfs, &tag);
                }
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
            Command::PlayVoice {
                path,
                men_voice,
                tag,
            } => {
                let buffer = self.audio_by_path(vfs, path)?;
                // `FUN_0044e800` asks the menu DLL's `GetMenVoice` export and
                // returns without playing when the option is off. The clip is
                // dropped outright, so there is no mouth to drive either.
                if *men_voice && !self.men_voice {
                    return Ok(());
                }
                if !tag.is_empty() {
                    self.voices.insert(
                        tag.clone(),
                        VoiceLine {
                            start,
                            end,
                            envelope: Envelope::from_audio(&buffer),
                        },
                    );
                    self.load_mouth(vfs, tag);
                }
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
                let mut decoder = VideoDecoder::open(bytes)?;
                self.size_video(&mut decoder, path);
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

    /// Drops the background and everything loaded onto it.
    ///
    /// The mouth overlays are part of the background object, not a layer over
    /// it: `FUN_004453e0` fills the `FILMOBJ::ImageChar`'s own slots, so
    /// destroying the background destroys them. `FILMOBJ::MovieChar` carries
    /// ten slots of its own at `+0xe4` and nothing in the retail install fills
    /// those — no `MovieNN` pack holds a single `.A`/`.B`/`.C` overlay — so a
    /// movie has no mouths, rather than inheriting the last background's.
    ///
    /// Keeping them is what stamped the previous background's mouth over the
    /// movie, in the same place on every frame of it. 483 of the shipped
    /// scripts start a movie while a tagged line is still speaking, so it was
    /// not a corner.
    fn drop_background(&mut self) {
        self.still = None;
        self.mouths.clear();
    }

    /// Loads the mouth set for `tag` on the current background, once.
    fn load_mouth(&mut self, vfs: &Vfs, tag: &str) {
        let Some(still) = &self.still else { return };
        if self.mouths.contains_key(tag) {
            return;
        }
        let mouth = match Mouth::load(vfs, &still.path, tag) {
            Ok(mouth) => mouth,
            Err(err) => {
                log::warn!("loading {tag} mouth for {}: {err:#}", still.path);
                None
            }
        };
        self.mouths.insert(tag.to_string(), mouth);
    }

    fn audio(
        &mut self,
        vfs: &Vfs,
        handle: crate::install::vfs::Handle,
    ) -> Result<Arc<AudioBuffer>> {
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

        let movie = self
            .movie
            .as_ref()
            .and_then(|m| Some((m, m.current.as_ref()?)));
        let mut visual = Visual {
            movie: movie.map(|(_, frame)| frame),
            movie_id: movie.map(|(clip, frame)| (clip.path.as_str(), frame.index)),
            still: self.still.as_ref(),
            fade: self.fade.as_ref().map(|f| (f.colour, f.opacity(at))),
            ..Default::default()
        };

        visual.mouths = self
            .voices
            .iter()
            .filter(|(_, v)| at >= v.start && at < v.end)
            .filter_map(|(tag, v)| {
                let mouth = self.mouths.get(tag)?.as_ref()?;
                Some((mouth, lipsync::image_index(&v.envelope, v.start, at)))
            })
            .collect();

        for event in self.script.events.iter().filter(|e| e.is_active_at(at)) {
            match &event.command {
                Command::PrintText { speaker, text } => {
                    visual.text = Some((speaker.as_str(), text.as_str()))
                }
                Command::SetSelect { a, b } => {
                    visual.select = Some(SelectWindow {
                        a: a.as_str(),
                        b: b.as_deref(),
                        start: event.start,
                        end: event.end,
                    })
                }
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

    /// The mouth overlays belong to the background object, so losing the
    /// background loses them. This is the step `[PlayMovie]` takes; a movie
    /// that inherited them drew the last background's mouth over itself.
    #[test]
    fn dropping_the_background_drops_its_mouths() {
        let mut stage = Stage::new(Script::default());
        stage.still = Some(Still {
            path: "Event00/x".into(),
            width: 800,
            height: 452,
            rgba: vec![0u8; 800 * 452 * 4],
        });
        stage.mouths.insert("mak".into(), None);
        stage.mouths.insert("sek".into(), None);

        stage.drop_background();

        assert!(stage.still.is_none());
        assert!(
            stage.mouths.is_empty(),
            "a movie must not inherit the background's mouths"
        );
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
