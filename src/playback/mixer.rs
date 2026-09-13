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
//!
//! Those are the script's channels, and they are the ones
//! [`Mixer::pause_script`] holds. The menus' own sounds go to a channel beside
//! them ([`Mixer::play_system_se`]), because in the original they are a
//! different object: the script's streams are `FILMOBJ::BgmSound`s owned by the
//! timeline, and a menu that is up over a paused script still clicks.

use crate::media::AudioBuffer;
use std::sync::{Arc, Mutex};

/// How many `[PlaySe]` slots exist. Scripts address them as 1..=5.
pub const SE_SLOTS: usize = 5;

/// The playback rate above which the original stops letting the audio be heard.
///
/// `FUN_004433d0` is the stream's rate setter, and before it forwards the rate
/// it compares it against the **double** at `0x004d5080` — `FCOMP double ptr`,
/// and the eight bytes there are `4.0`. Above that it latches `+0x40` and calls
/// the mute helper `FUN_00443650(this, 1)`; at or below it, it restores
/// whatever `_GetMute@0` says. So of the five rates in [`SPEEDS`] the first
/// three are audible and 12x and 24x are not.
///
/// Read as a 4-byte float those bytes are `0.0`, which would make every rate
/// mute. The operand width is the authority.
///
/// [`SPEEDS`]: crate::ui::bar::SPEEDS
pub const MUTE_ABOVE: f32 = 4.0;

/// One playing sound.
struct Voice {
    buffer: Arc<AudioBuffer>,
    /// Position in sample frames. Fractional because the playback rate the
    /// speed widgets select resamples every channel — see [`MixerState::rate`].
    position: f64,
    /// Buffer to switch to when this one ends, used for BGM intro -> loop.
    then: Option<Arc<AudioBuffer>>,
    /// Restart at the end instead of stopping.
    repeat: bool,
    volume: f32,
    /// Held where it is, neither heard nor advanced. See
    /// [`Mixer::pause_script`].
    paused: bool,
}

impl Voice {
    fn once(buffer: Arc<AudioBuffer>, volume: f32) -> Voice {
        Voice {
            buffer,
            position: 0.0,
            then: None,
            repeat: false,
            volume,
            paused: false,
        }
    }

    /// Advances to the next source frame, answering false once the voice has
    /// finished. The wrap keeps the fractional part, so a resampled loop does
    /// not drift or click at the seam.
    fn wrap(&mut self) -> bool {
        loop {
            let frames = self.buffer.frames();
            if self.position < frames as f64 {
                return true;
            }
            // End of this buffer: move to the follow-on, loop, or stop.
            if let Some(next) = self.then.take() {
                self.position -= frames as f64;
                self.buffer = next;
                // The follow-on half of a BGM track is the looping half.
                self.repeat = true;
                continue;
            }
            if self.repeat && frames > 0 {
                self.position -= frames as f64;
                continue;
            }
            return false;
        }
    }

    /// Adds this voice's next sample frames into `out`, returning false once
    /// the voice has finished.
    ///
    /// `rate` is the playback rate: 1.0 reads one source frame per output
    /// frame, 2.0 reads two, and so on. `silent` advances the position without
    /// writing anything, which is how a muted-but-running stream stays in step
    /// with the timeline.
    fn mix_into(&mut self, out: &mut [f32], rate: f64, silent: bool) -> bool {
        // A paused stream is not silence with the clock running: the original
        // stops the buffer where it stands and starts it again from there.
        if self.paused {
            return true;
        }
        let channels = AudioBuffer::CHANNELS;
        let wanted = out.len() / channels;

        // At 1x with a whole position this is a straight sum, which is both
        // faster and bit-for-bit what it was before rates existed.
        if rate == 1.0 && self.position.fract() == 0.0 {
            let mut written = 0usize;
            while written < wanted {
                if !self.wrap() {
                    return false;
                }
                let at = self.position as usize;
                let available = self.buffer.frames() - at;
                let take = available.min(wanted - written);
                if !silent {
                    let src = &self.buffer.samples[at * channels..(at + take) * channels];
                    let dst = &mut out[written * channels..(written + take) * channels];
                    for (d, s) in dst.iter_mut().zip(src) {
                        *d += s * self.volume;
                    }
                }
                self.position += take as f64;
                written += take;
            }
            return true;
        }

        for frame in 0..wanted {
            if !self.wrap() {
                return false;
            }
            if !silent {
                // Linear interpolation between the two source frames the
                // position falls between. The last frame interpolates against
                // itself rather than past the end of the buffer.
                let at = self.position as usize;
                let next = (at + 1).min(self.buffer.frames() - 1);
                let frac = (self.position - at as f64) as f32;
                for channel in 0..channels {
                    let a = self.buffer.samples[at * channels + channel];
                    let b = self.buffer.samples[next * channels + channel];
                    out[frame * channels + channel] += (a + (b - a) * frac) * self.volume;
                }
            }
            self.position += rate;
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
    /// The menus' own sound. Outside the script's channels, so pausing the
    /// script does not silence a menu over it and a script cannot cut a click
    /// short by reusing a slot.
    system: Option<Voice>,
    master: Option<f32>,
    /// The playback rate the control bar's speed widgets select, as a
    /// multiplier on the source. `None` is 1x.
    ///
    /// `FUN_00424f90` hands the rate it stores at `engine + 0x538` to the
    /// script's audio through `FUN_00429500` -> `FUN_004433d0`, which sets the
    /// stream's rate with `FUN_0041a050` — a bare rate message (`0x8005`) on
    /// the sound object, so the original resamples and the pitch rises with the
    /// speed. Nothing time-stretches.
    rate: Option<f32>,
}

impl MixerState {
    fn master(&self) -> f32 {
        self.master.unwrap_or(1.0)
    }

    fn rate(&self) -> f32 {
        self.rate.unwrap_or(1.0)
    }

    /// Every channel a script owns: the ones the pause holds.
    fn script_voices(&mut self) -> impl Iterator<Item = &mut Voice> {
        self.bgm
            .iter_mut()
            .chain(self.voice.iter_mut())
            .chain(self.se.iter_mut().flatten())
    }

    /// Sums every active channel into `out`, which arrives zeroed.
    fn render(&mut self, out: &mut [f32]) {
        out.fill(0.0);

        let rate = self.rate();
        // Past the threshold the original stops giving the player audio at all
        // and keeps the stream running underneath — see [`MUTE_ABOVE`].
        let silent = rate > MUTE_ABOVE;
        let rate = f64::from(rate);

        if let Some(v) = &mut self.bgm {
            if !v.mix_into(out, rate, silent) {
                self.bgm = None;
            }
        }
        for slot in &mut self.se {
            if let Some(v) = slot {
                if !v.mix_into(out, rate, silent) {
                    *slot = None;
                }
            }
        }
        if let Some(v) = &mut self.voice {
            if !v.mix_into(out, rate, silent) {
                self.voice = None;
            }
        }
        // The menus are not on the rate-adjusted stream and are never muted by
        // it: the speed widgets belong to the script.
        if let Some(v) = &mut self.system {
            if !v.mix_into(out, 1.0, false) {
                self.system = None;
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
                    position: 0.0,
                    then: Some(looped),
                    repeat: false,
                    volume: 1.0,
                    paused: false,
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

    /// Plays one of the menus' own sounds, beside the script's channels.
    pub fn play_system_se(&self, buffer: Arc<AudioBuffer>) {
        self.with(|s| s.system = Some(Voice::once(buffer, 1.0)));
    }

    /// Silences the script's channels. Used when the timeline jumps, and when
    /// playback is left for good.
    ///
    /// The menus' own channel is left alone: a click that was still ringing
    /// when the screen changed rings out, as it does in the original, where it
    /// belongs to the menu module and not to the timeline being torn down.
    pub fn stop_all(&self) {
        self.with(|s| {
            s.bgm = None;
            s.voice = None;
            s.se = Default::default();
        });
    }

    /// Holds every script channel where it stands.
    ///
    /// `FUN_00424910`, reached through `FUN_00424e20`, which everything that
    /// suspends playback calls first: host `+0xf4` (the bar's pause widget,
    /// `FUN_00424f40`), `+0xf8` (open a menu over the script, `FUN_0042a430`),
    /// `+0xfc` (skip) and `+0x100` (leave playback, `FUN_0042a500`). It folds
    /// the frames run so far into the clock's base and then pauses the engine's
    /// two script streams at `+0x304` and `+0x30c` and, through the timeline
    /// object at `+0x1e4` (`FUN_0043edd0`), its eight `FILMOBJ::BgmSound` slots
    /// at `+0x39c`, the one at `+0x3bc`, and the movie. Pausing is
    /// `FUN_004431e0` -> `FUN_0041a7a0`, which stops the buffer and latches
    /// `+0x28`; the position is kept.
    ///
    /// Nothing here is heard and nothing advances, so a script resumes on the
    /// frame it was interrupted on rather than a menu's worth of audio later.
    pub fn pause_script(&self) {
        self.with(|s| {
            for v in s.script_voices() {
                v.paused = true;
            }
        });
    }

    /// Starts the script's channels again where [`Mixer::pause_script`] left
    /// them.
    ///
    /// `FUN_00424a10`, through `FUN_00424eb0`. Its six call sites — Ghidra's
    /// reference index and a raw scan of `.text` for `E8` displacements
    /// agree — are all paths back into playback: `FUN_00425550` case 8 (a menu
    /// the control bar opened was closed), `FUN_00426620` cases 7 and 9,
    /// `FUN_00426bd0` case 6, `FUN_00425bf0` and `FUN_00424f40`. Leaving
    /// playback for the title reaches none of them, so the sound stays stopped
    /// and the title screen is silent but for its own `[TitleBGM]`.
    pub fn resume_script(&self) {
        self.with(|s| {
            for v in s.script_voices() {
                v.paused = false;
            }
        });
    }

    pub fn set_master_volume(&self, volume: f32) {
        self.with(|s| s.master = Some(volume.clamp(0.0, 1.0)));
    }

    /// Sets the playback rate every channel is resampled at.
    ///
    /// The engine calls this from the control bar's speed widgets, so the
    /// audio keeps up with the timeline instead of playing on at 1x under a
    /// picture running four times as fast. Above [`MUTE_ABOVE`] the streams
    /// keep running and stop being heard, which is what the original does.
    pub fn set_rate(&self, rate: f32) {
        let rate = if rate.is_finite() && rate > 0.0 {
            rate
        } else {
            1.0
        };
        self.with(|s| s.rate = Some(rate));
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

    /// A ramp, so a resampled read is distinguishable from a plain one.
    ///
    /// The step is 1/256 because `render` clamps the mix to +/-1: a ramp in
    /// whole numbers would come back flattened and every rate would look the
    /// same.
    fn ramp(frames: usize) -> Arc<AudioBuffer> {
        let channels = AudioBuffer::CHANNELS;
        Arc::new(AudioBuffer {
            samples: (0..frames * channels).map(|i| step(i / channels)).collect(),
        })
    }

    /// The ramp's value at source frame `n`.
    fn step(n: usize) -> f32 {
        n as f32 / 256.0
    }

    /// The left channel of each output frame.
    fn left(out: &[f32]) -> Vec<f32> {
        out.iter().step_by(AudioBuffer::CHANNELS).copied().collect()
    }

    /// 2x reads two source frames per output frame, so the ramp comes back
    /// stepping by two. This is the whole point: the audio keeps up with the
    /// picture instead of playing on at 1x underneath it.
    #[test]
    fn a_doubled_rate_reads_the_source_twice_as_fast() {
        let m = Mixer::new();
        m.set_rate(2.0);
        m.play_se(1, ramp(16));
        let mut out = vec![0.0; 4 * AudioBuffer::CHANNELS];
        m.render(&mut out);
        assert_eq!(left(&out), vec![step(0), step(2), step(4), step(6)]);
    }

    /// The pause is not silence with the clock running: the stream stands
    /// still and starts again on the frame it stopped on, because the original
    /// stops the buffer and keeps its position (`FUN_004431e0` / `FUN_00443340`
    /// around the `+0x28` latch).
    #[test]
    fn a_paused_stream_stands_still_and_goes_on_where_it_stopped() {
        let m = Mixer::new();
        m.play_se(1, ramp(16));
        let mut out = vec![0.0; 2 * AudioBuffer::CHANNELS];
        m.render(&mut out);
        assert_eq!(left(&out), vec![step(0), step(1)]);

        m.pause_script();
        m.render(&mut out);
        assert_eq!(left(&out), vec![0.0, 0.0]);

        m.resume_script();
        m.render(&mut out);
        assert_eq!(left(&out), vec![step(2), step(3)]);
    }

    /// The menus are not the script. Their sounds are separate objects in the
    /// original, so a screen over a paused script still clicks — and it clicks
    /// at 1x, whatever speed the control bar was left at, including the two
    /// rates that silence the script's own streams.
    #[test]
    fn the_menus_sound_over_a_paused_script() {
        let m = Mixer::new();
        m.set_rate(24.0);
        m.play_se(1, buffer(16, 0.5));
        m.pause_script();
        m.play_system_se(ramp(16));
        let mut out = vec![0.0; 2 * AudioBuffer::CHANNELS];
        m.render(&mut out);
        assert_eq!(left(&out), vec![step(0), step(1)]);
    }

    /// 4x is the last audible rate, and it is audible.
    #[test]
    fn four_times_is_still_heard() {
        let m = Mixer::new();
        m.set_rate(MUTE_ABOVE);
        m.play_se(1, ramp(32));
        let mut out = vec![0.0; 3 * AudioBuffer::CHANNELS];
        m.render(&mut out);
        assert_eq!(left(&out), vec![step(0), step(4), step(8)]);
    }

    /// Past the threshold the stream keeps running and stops being heard, so
    /// 12x and 24x are silent — `FUN_004433d0` mutes above the double 4.0.
    #[test]
    fn past_the_threshold_the_stream_runs_on_unheard() {
        for rate in [12.0f32, 24.0] {
            let m = Mixer::new();
            m.set_rate(rate);
            m.play_se(1, ramp(256));
            let mut out = vec![0.0; 4 * AudioBuffer::CHANNELS];
            m.render(&mut out);
            assert!(
                out.iter().all(|s| *s == 0.0),
                "{rate}x should be silent, got {out:?}"
            );
            // Still running underneath: dropping back to 1x picks up where the
            // muted stretch left off rather than at the start.
            m.set_rate(1.0);
            m.render(&mut out);
            assert_eq!(
                out[0],
                step(4 * rate as usize),
                "{rate}x resumed in the wrong place"
            );
        }
    }

    /// 1x is exactly what it was before rates existed — the fast path, and no
    /// interpolation of a signal that is not being resampled.
    #[test]
    fn one_times_is_unchanged() {
        let m = Mixer::new();
        m.play_se(1, ramp(4));
        let mut out = vec![0.0; 4 * AudioBuffer::CHANNELS];
        m.render(&mut out);
        assert_eq!(left(&out), vec![step(0), step(1), step(2), step(3)]);
    }

    /// A rate that could only come from a bug does not take the mixer with it.
    #[test]
    fn a_nonsense_rate_falls_back_to_1x() {
        for bad in [0.0f32, -2.0, f32::NAN, f32::INFINITY] {
            let m = Mixer::new();
            m.set_rate(bad);
            m.play_se(1, ramp(4));
            let mut out = vec![0.0; 2 * AudioBuffer::CHANNELS];
            m.render(&mut out);
            assert_eq!(left(&out), vec![step(0), step(1)], "rate {bad}");
        }
    }

    /// Looping BGM keeps the fractional position across the seam, so a
    /// resampled loop neither drifts nor lands on the same sample twice.
    #[test]
    fn a_resampled_loop_carries_its_fraction_over_the_seam() {
        let m = Mixer::new();
        m.set_rate(1.5);
        m.play_bgm(None, ramp(4));
        let mut out = vec![0.0; 5 * AudioBuffer::CHANNELS];
        m.render(&mut out);
        // 0, 1.5, 3.0, then the seam carries 4.5 - 4 = 0.5 over, then 2.0.
        let heard = left(&out);
        assert_eq!(heard[0], step(0));
        assert_eq!(heard[1], (step(1) + step(2)) / 2.0);
        assert_eq!(heard[2], step(3));
        assert_eq!(heard[3], (step(0) + step(1)) / 2.0);
        assert_eq!(heard[4], step(2));
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
