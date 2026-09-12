//! The audio mixer.
//!
//! `DX8SOUND.INI` declares the output format as 44.1 kHz, 16-bit, stereo, and
//! everything is decoded to that up front, so mixing is a straight sum. The
//! channel layout follows the script commands rather than anything in the
//! engine's own design:
//!
//! * one **BGM** channel, driven by `[PlayBgm]` / `[EndBGM]`
//! * five **SE** slots, addressed by number in `[PlaySe]`
//! * one **voice** channel, driven by `[PlayVoice]`
//!
//! Scripts hand `[PlaySe]` slot 5 a `Voice...` path fairly often — the game
//! reuses the SE mixer for lines that are not lip-synced — so the SE slots are
//! not special-cased for sound effects.

use days_media::AudioBuffer;
use std::sync::{Arc, Mutex};

/// How many `[PlaySe]` slots exist. Scripts address them as 1..=5.
pub const SE_SLOTS: usize = 5;

/// One playing sound.
struct Voice {
    buffer: Arc<AudioBuffer>,
    /// Position in sample frames.
    position: usize,
    /// Buffer to switch to when this one ends, used for BGM intro -> loop.
    then: Option<Arc<AudioBuffer>>,
    /// Restart at the end instead of stopping.
    repeat: bool,
    volume: f32,
}

impl Voice {
    fn once(buffer: Arc<AudioBuffer>, volume: f32) -> Voice {
        Voice {
            buffer,
            position: 0,
            then: None,
            repeat: false,
            volume,
        }
    }

    /// Adds this voice's next `frames` sample frames into `out`, returning false
    /// once the voice has finished.
    fn mix_into(&mut self, out: &mut [f32]) -> bool {
        let channels = AudioBuffer::CHANNELS;
        let mut written = 0usize;
        let wanted = out.len() / channels;

        while written < wanted {
            let available = self.buffer.frames().saturating_sub(self.position);
            if available == 0 {
                // End of this buffer: move to the follow-on, loop, or stop.
                if let Some(next) = self.then.take() {
                    self.buffer = next;
                    self.position = 0;
                    // The follow-on half of a BGM track is the looping half.
                    self.repeat = true;
                    continue;
                }
                if self.repeat && self.buffer.frames() > 0 {
                    self.position = 0;
                    continue;
                }
                return false;
            }

            let take = available.min(wanted - written);
            let src =
                &self.buffer.samples[self.position * channels..(self.position + take) * channels];
            let dst = &mut out[written * channels..(written + take) * channels];
            for (d, s) in dst.iter_mut().zip(src) {
                *d += s * self.volume;
            }
            self.position += take;
            written += take;
        }
        true
    }
}

/// Mixer state, shared between the game thread and the audio callback.
#[derive(Default)]
pub struct MixerState {
    bgm: Option<Voice>,
    se: [Option<Voice>; SE_SLOTS],
    voice: Option<Voice>,
    master: Option<f32>,
}

impl MixerState {
    fn master(&self) -> f32 {
        self.master.unwrap_or(1.0)
    }

    /// Sums every active channel into `out`, which arrives zeroed.
    fn render(&mut self, out: &mut [f32]) {
        out.fill(0.0);

        if let Some(v) = &mut self.bgm {
            if !v.mix_into(out) {
                self.bgm = None;
            }
        }
        for slot in &mut self.se {
            if let Some(v) = slot {
                if !v.mix_into(out) {
                    *slot = None;
                }
            }
        }
        if let Some(v) = &mut self.voice {
            if !v.mix_into(out) {
                self.voice = None;
            }
        }

        let master = self.master();
        if master != 1.0 {
            for s in out.iter_mut() {
                *s *= master;
            }
        }
        // Several channels can peak together; clamp rather than wrap.
        for s in out.iter_mut() {
            *s = s.clamp(-1.0, 1.0);
        }
    }
}

/// Handle used by the game thread to start and stop sounds.
#[derive(Clone, Default)]
pub struct Mixer {
    state: Arc<Mutex<MixerState>>,
}

impl Mixer {
    pub fn new() -> Mixer {
        Mixer::default()
    }

    fn with<R>(&self, f: impl FnOnce(&mut MixerState) -> R) -> R {
        let mut guard = self.state.lock().expect("mixer mutex poisoned");
        f(&mut guard)
    }

    /// Starts background music: `intro` once if present, then `looped` forever.
    pub fn play_bgm(&self, intro: Option<Arc<AudioBuffer>>, looped: Arc<AudioBuffer>) {
        self.with(|s| {
            s.bgm = Some(match intro {
                Some(intro) => Voice {
                    buffer: intro,
                    position: 0,
                    then: Some(looped),
                    repeat: false,
                    volume: 1.0,
                },
                None => Voice {
                    repeat: true,
                    ..Voice::once(looped, 1.0)
                },
            });
        });
    }

    pub fn stop_bgm(&self) {
        self.with(|s| s.bgm = None);
    }

    /// Plays a sound on one of the numbered slots. `slot` is 1-based, as scripts
    /// write it; anything out of range is dropped with a warning rather than
    /// panicking, since it would come from script data.
    pub fn play_se(&self, slot: u8, buffer: Arc<AudioBuffer>) {
        let Some(index) = (slot as usize).checked_sub(1).filter(|i| *i < SE_SLOTS) else {
            log::warn!("[PlaySe] slot {slot} is out of range 1..={SE_SLOTS}; ignoring");
            return;
        };
        self.with(|s| s.se[index] = Some(Voice::once(buffer, 1.0)));
    }

    pub fn play_voice(&self, buffer: Arc<AudioBuffer>) {
        self.with(|s| s.voice = Some(Voice::once(buffer, 1.0)));
    }

    /// Silences everything. Used when the timeline jumps.
    pub fn stop_all(&self) {
        self.with(|s| {
            s.bgm = None;
            s.voice = None;
            s.se = Default::default();
        });
    }

    pub fn set_master_volume(&self, volume: f32) {
        self.with(|s| s.master = Some(volume.clamp(0.0, 1.0)));
    }

    /// Fills `out` with the next block of mixed audio.
    pub fn render(&self, out: &mut [f32]) {
        self.with(|s| s.render(out));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buffer(frames: usize, value: f32) -> Arc<AudioBuffer> {
        Arc::new(AudioBuffer {
            samples: vec![value; frames * AudioBuffer::CHANNELS],
        })
    }

    #[test]
    fn a_one_shot_stops_and_leaves_silence() {
        let m = Mixer::new();
        m.play_se(1, buffer(2, 0.5));
        let mut out = vec![0.0; 8];
        m.render(&mut out);
        assert_eq!(out, vec![0.5, 0.5, 0.5, 0.5, 0.0, 0.0, 0.0, 0.0]);
    }

    /// BGM plays its intro once and then loops the second half forever.
    #[test]
    fn bgm_runs_the_intro_then_repeats_the_loop() {
        let m = Mixer::new();
        m.play_bgm(Some(buffer(1, 0.25)), buffer(2, 0.5));
        let mut out = vec![0.0; 2 * 7];
        m.render(&mut out);
        let frames: Vec<f32> = out.iter().step_by(2).copied().collect();
        assert_eq!(frames, vec![0.25, 0.5, 0.5, 0.5, 0.5, 0.5, 0.5]);
    }

    /// A loop-only track (nine of the game's BGM entries ship without an intro)
    /// repeats from the first sample.
    #[test]
    fn loop_only_bgm_repeats_immediately() {
        let m = Mixer::new();
        m.play_bgm(None, buffer(2, 1.0));
        let mut out = vec![0.0; 2 * 5];
        m.render(&mut out);
        assert!(out.iter().all(|&s| s == 1.0));
    }

    #[test]
    fn channels_sum_and_clamp() {
        let m = Mixer::new();
        m.play_se(1, buffer(4, 0.7));
        m.play_se(2, buffer(4, 0.7));
        m.play_voice(buffer(4, 0.7));
        let mut out = vec![0.0; 4];
        m.render(&mut out);
        assert!(out.iter().all(|&s| s == 1.0), "{out:?}");
    }

    #[test]
    fn out_of_range_se_slots_are_ignored() {
        let m = Mixer::new();
        m.play_se(0, buffer(4, 1.0));
        m.play_se(9, buffer(4, 1.0));
        let mut out = vec![0.0; 4];
        m.render(&mut out);
        assert!(out.iter().all(|&s| s == 0.0));
    }

    #[test]
    fn master_volume_scales_the_mix() {
        let m = Mixer::new();
        m.set_master_volume(0.5);
        m.play_se(1, buffer(2, 0.8));
        let mut out = vec![0.0; 4];
        m.render(&mut out);
        assert!(out.iter().all(|&s| (s - 0.4).abs() < 1e-6), "{out:?}");
    }
}
