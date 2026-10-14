//! Ogg Vorbis decoding to the game's mixer format.
//!
//! The mixer wants random access into a decoded clip — a loop wraps, a rate
//! resamples, a seek starts a sound part-way — so every clip is decoded to
//! samples rather than streamed through an ffmpeg context per playing sound.
//!
//! **Not all of it at once, though.** The assumption this module used to rest
//! on, that "voice lines and SFX are a few seconds each", is false of the
//! retail data: several `[PlaySe]` clips are ambient beds, `SE00-00-A00-003`
//! and `-004` run 167 seconds, and decoding one whole took 160-190ms on a
//! Snapdragon 8 Gen 2 — on the thread that draws, so the movie under it lost
//! frames every time one started. [`decode_audio_progressive`] decodes the
//! first [`HEAD_SECONDS`] there and then, which is a few milliseconds, and
//! hands the rest to a thread of its own. [`decode_audio`] is still the whole
//! clip at once, for the callers that need all of it before they start.

use super::io::MemoryIo;
use super::{Error, CHANNELS, SAMPLE_RATE};
use rusty_ffmpeg::ffi;
use std::ffi::c_int;
use std::sync::{Arc, OnceLock, Weak};

/// How much of a clip [`decode_audio_progressive`] decodes before it returns.
///
/// What it has to outlast is the worker decoding everything after it. That
/// measured about 1.1ms per second of audio on a Snapdragon 8 Gen 2 — a
/// 167-second bed in 190ms — so four seconds of head is some twenty times what
/// the longest clip in the game needs, and costs about 5ms to decode.
pub const HEAD_SECONDS: usize = 4;

/// Decoded audio, interleaved stereo `f32` at [`SAMPLE_RATE`].
///
/// In two parts: the **head**, decoded before the buffer existed, and the
/// **rest**, which a worker may still be decoding — see
/// [`decode_audio_progressive`]. The rest is written once and then never
/// changes, so the audio callback reads both without a lock. Until it is
/// written, [`AudioBuffer::frames`] is the head's length and
/// [`AudioBuffer::is_complete`] is false, and a reader that reaches the end
/// of the head waits rather than taking it for the end of the clip.
pub struct AudioBuffer {
    /// Interleaved L,R,L,R... in `-1.0..=1.0`.
    head: Vec<f32>,
    rest: OnceLock<Vec<f32>>,
}

impl AudioBuffer {
    pub const CHANNELS: usize = CHANNELS as usize;

    /// A clip decoded whole: interleaved L,R,L,R... in `-1.0..=1.0`.
    pub fn complete(samples: Vec<f32>) -> AudioBuffer {
        AudioBuffer {
            head: samples,
            rest: OnceLock::from(Vec::new()),
        }
    }

    /// A clip whose head is here and whose rest is not yet, for a test of
    /// what reads one. The rest arrives with [`Self::arrive`].
    #[cfg(test)]
    pub(crate) fn pending(head: Vec<f32>) -> AudioBuffer {
        AudioBuffer {
            head,
            rest: OnceLock::new(),
        }
    }

    /// The rest of a [`Self::pending`] clip, as the worker would set it.
    #[cfg(test)]
    pub(crate) fn arrive(&self, rest: Vec<f32>) {
        self.rest.set(rest).expect("the rest arrives once");
    }

    pub fn silent() -> AudioBuffer {
        AudioBuffer::complete(Vec::new())
    }

    /// Number of sample frames decoded so far (one frame = one sample per
    /// channel). The whole clip once [`Self::is_complete`].
    pub fn frames(&self) -> usize {
        (self.head.len() + self.rest.get().map_or(0, Vec::len)) / Self::CHANNELS
    }

    /// Whether every frame of the clip is here.
    pub fn is_complete(&self) -> bool {
        self.rest.get().is_some()
    }

    /// Blocks until [`Self::is_complete`], for a caller that needs the whole
    /// clip before it can start — lip sync reads a voice line end to end.
    pub fn wait(&self) {
        self.rest.wait();
    }

    /// One interleaved sample, `frame * CHANNELS + channel`. Must be below
    /// `frames() * CHANNELS`.
    pub fn sample(&self, index: usize) -> f32 {
        match index.checked_sub(self.head.len()) {
            None => self.head[index],
            Some(at) => self.rest.get().map_or(0.0, |rest| rest[at]),
        }
    }

    /// The interleaved samples from frame `at` to the end of whichever part
    /// holds it: contiguous, so a caller can sum straight down it. Empty at
    /// or past [`Self::frames`].
    pub fn run(&self, at: usize) -> &[f32] {
        let index = at * Self::CHANNELS;
        match index.checked_sub(self.head.len()) {
            None => &self.head[index..],
            Some(at) => self
                .rest
                .get()
                .and_then(|rest| rest.get(at..))
                .unwrap_or(&[]),
        }
    }

    /// Every sample decoded so far, in order.
    pub fn samples(&self) -> impl Iterator<Item = f32> + '_ {
        self.head
            .iter()
            .chain(self.rest.get().into_iter().flatten())
            .copied()
    }

    pub fn duration_seconds(&self) -> f64 {
        self.frames() as f64 / f64::from(SAMPLE_RATE)
    }

    pub fn is_empty(&self) -> bool {
        self.frames() == 0
    }
}

impl std::fmt::Debug for AudioBuffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioBuffer")
            .field("frames", &self.frames())
            .field("seconds", &self.duration_seconds())
            .field("complete", &self.is_complete())
            .finish()
    }
}

/// Decodes a complete audio file, resampling to the mixer's format.
pub fn decode_audio(data: Vec<u8>) -> Result<AudioBuffer, Error> {
    let mut decoder = Decoder::open(data)?;
    let mut samples = Vec::new();
    decoder.decode(&mut samples, usize::MAX)?;
    Ok(AudioBuffer::complete(samples))
}

/// Decodes the first [`HEAD_SECONDS`] of a clip now and the rest on a thread
/// of its own, returning as soon as the head is ready.
///
/// A clip that ends inside its head comes back complete and starts no thread,
/// which is every short sound. The worker holds the buffer weakly and stops
/// when nothing else does — a script that ends mid-decode does not leave it
/// running — and a clip that fails part-way is kept as far as it decoded, so
/// a sound that is waiting on it ends rather than waiting for ever.
pub fn decode_audio_progressive(data: Vec<u8>) -> Result<Arc<AudioBuffer>, Error> {
    let mut decoder = Decoder::open(data)?;
    let mut head = Vec::new();
    let ended = decoder.decode(&mut head, HEAD_SECONDS * SAMPLE_RATE as usize)?;
    if ended {
        return Ok(Arc::new(AudioBuffer::complete(head)));
    }
    let buffer = Arc::new(AudioBuffer {
        head,
        rest: OnceLock::new(),
    });
    let weak = Arc::downgrade(&buffer);
    let spawned = std::thread::Builder::new()
        .name("audio-decode".into())
        .spawn(move || finish(decoder, weak));
    if let Err(err) = spawned {
        // No thread to be had: the sound is its head and no more, which is a
        // short sound rather than a stuck one.
        log::warn!("no thread to decode the rest of a sound on: {err}");
        let _ = buffer.rest.set(Vec::new());
    }
    Ok(buffer)
}

/// The worker: everything after the head, in slices so it can notice that
/// nobody wants the clip any more.
fn finish(mut decoder: Decoder, buffer: Weak<AudioBuffer>) {
    // A second of audio at a time; a check of the weak count between them is
    // nothing next to decoding one.
    let slice = SAMPLE_RATE as usize;
    let mut rest = Vec::new();
    loop {
        if buffer.strong_count() == 0 {
            return;
        }
        match decoder.decode(&mut rest, slice) {
            Ok(false) => continue,
            Ok(true) => break,
            Err(err) => {
                log::warn!("decoding the rest of a sound: {err}; it ends where it got to");
                break;
            }
        }
    }
    if let Some(buffer) = buffer.upgrade() {
        let _ = buffer.rest.set(rest);
    }
}

/// An open clip, part-way or not at all decoded.
struct Decoder {
    state: AudioState,
    /// The audio stream's index, against packets of other streams.
    index: c_int,
    /// Whether the demuxer has run out and the decoder been flushed.
    flushed: bool,
    // Declared last so it is dropped last: the format context reads through it.
    _io: Box<MemoryIo>,
}

// Every pointer in `state` is owned by this decoder alone, and the one thread
// that holds it is the only one that touches them.
unsafe impl Send for Decoder {}

impl Decoder {
    fn open(data: Vec<u8>) -> Result<Decoder, Error> {
        let io = MemoryIo::new(data);
        let mut state = AudioState::default();

        // SAFETY: `state` frees everything it owns on drop, including on the
        // early returns below, and `io` outlives the format context because
        // the decoder drops it last.
        unsafe {
            let format = ffi::avformat_alloc_context();
            if format.is_null() {
                return Err(Error::Alloc("AVFormatContext"));
            }
            (*format).pb = io.context();
            (*format).flags |= ffi::AVFMT_FLAG_CUSTOM_IO as c_int;

            let mut format_ptr = format;
            let code = ffi::avformat_open_input(
                &mut format_ptr,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null_mut(),
            );
            if code < 0 {
                return Err(Error::Ffmpeg {
                    what: "avformat_open_input",
                    code,
                });
            }
            state.format = format_ptr;

            Error::check(
                "avformat_find_stream_info",
                ffi::avformat_find_stream_info(state.format, std::ptr::null_mut()),
            )?;

            let index = ffi::av_find_best_stream(
                state.format,
                ffi::AVMEDIA_TYPE_AUDIO,
                -1,
                -1,
                std::ptr::null_mut(),
                0,
            );
            if index < 0 {
                return Err(Error::NoStream("audio"));
            }

            let stream = *(*state.format).streams.offset(index as isize);
            let params = (*stream).codecpar;
            let decoder = ffi::avcodec_find_decoder((*params).codec_id);
            if decoder.is_null() {
                return Err(Error::NoDecoder(format!("{:?}", (*params).codec_id)));
            }

            state.codec = ffi::avcodec_alloc_context3(decoder);
            if state.codec.is_null() {
                return Err(Error::Alloc("AVCodecContext"));
            }
            Error::check(
                "avcodec_parameters_to_context",
                ffi::avcodec_parameters_to_context(state.codec, params),
            )?;
            Error::check(
                "avcodec_open2",
                ffi::avcodec_open2(state.codec, decoder, std::ptr::null_mut()),
            )?;

            // Target layout: interleaved stereo f32 at the mixer rate.
            let mut out_layout: ffi::AVChannelLayout = std::mem::zeroed();
            ffi::av_channel_layout_default(&mut out_layout, CHANNELS as c_int);

            let mut resampler: *mut ffi::SwrContext = std::ptr::null_mut();
            Error::check(
                "swr_alloc_set_opts2",
                ffi::swr_alloc_set_opts2(
                    &mut resampler,
                    &out_layout,
                    ffi::AV_SAMPLE_FMT_FLT,
                    SAMPLE_RATE as c_int,
                    &(*state.codec).ch_layout,
                    (*state.codec).sample_fmt,
                    (*state.codec).sample_rate,
                    0,
                    std::ptr::null_mut(),
                ),
            )?;
            state.resampler = resampler;
            Error::check("swr_init", ffi::swr_init(state.resampler))?;

            state.packet = ffi::av_packet_alloc();
            state.frame = ffi::av_frame_alloc();
            if state.packet.is_null() || state.frame.is_null() {
                return Err(Error::Alloc("AVPacket/AVFrame"));
            }

            Ok(Decoder {
                state,
                index,
                flushed: false,
                _io: io,
            })
        }
    }

    /// Appends decoded frames to `out` until at least `frames` more are there
    /// or the clip ends, answering whether it ended. Stops on a packet
    /// boundary, so it may go a packet past `frames`.
    fn decode(&mut self, out: &mut Vec<f32>, frames: usize) -> Result<bool, Error> {
        if self.flushed {
            return Ok(true);
        }
        let goal = out
            .len()
            .saturating_add(frames.saturating_mul(AudioBuffer::CHANNELS));
        let state = &self.state;
        // SAFETY: `state` was fully initialised by `open` and is only used
        // here, by whichever thread holds the decoder.
        unsafe {
            while out.len() < goal {
                let code = ffi::av_read_frame(state.format, state.packet);
                if code == ffi::AVERROR_EOF {
                    // Flush the decoder, then the resampler's own tail.
                    ffi::avcodec_send_packet(state.codec, std::ptr::null());
                    drain(state, out)?;
                    flush_resampler(state, out)?;
                    self.flushed = true;
                    return Ok(true);
                }
                Error::check("av_read_frame", code)?;

                if (*state.packet).stream_index != self.index {
                    ffi::av_packet_unref(state.packet);
                    continue;
                }
                let send = ffi::avcodec_send_packet(state.codec, state.packet);
                ffi::av_packet_unref(state.packet);
                Error::check("avcodec_send_packet", send)?;
                drain(state, out)?;
            }
        }
        Ok(false)
    }
}

/// Pulls every frame the decoder currently has and resamples it.
///
/// # Safety
/// `state` must be fully initialised.
unsafe fn drain(state: &AudioState, out: &mut Vec<f32>) -> Result<(), Error> {
    loop {
        let code = unsafe { ffi::avcodec_receive_frame(state.codec, state.frame) };
        if code == ffi::AVERROR_EOF || code == ffi::AVERROR(ffi::EAGAIN) {
            return Ok(());
        }
        Error::check("avcodec_receive_frame", code)?;
        unsafe {
            resample(
                state,
                (*state.frame).extended_data as *const *const u8,
                (*state.frame).nb_samples,
                out,
            )?;
            ffi::av_frame_unref(state.frame);
        }
    }
}

/// Pushes buffered samples out of the resampler at end of stream.
///
/// # Safety
/// `state` must be fully initialised.
unsafe fn flush_resampler(state: &AudioState, out: &mut Vec<f32>) -> Result<(), Error> {
    unsafe { resample(state, std::ptr::null(), 0, out) }
}

/// Converts `in_samples` frames of input to the mixer format and appends them.
///
/// # Safety
/// `input` must point at `in_samples` frames in the decoder's format, or be null
/// to flush.
unsafe fn resample(
    state: &AudioState,
    input: *const *const u8,
    in_samples: c_int,
    out: &mut Vec<f32>,
) -> Result<(), Error> {
    // Worst case output size for this input, accounting for samples the
    // resampler is still holding.
    let capacity = unsafe { ffi::swr_get_out_samples(state.resampler, in_samples) };
    let capacity = Error::check("swr_get_out_samples", capacity)? as usize;
    if capacity == 0 {
        return Ok(());
    }

    let base = out.len();
    out.resize(base + capacity * AudioBuffer::CHANNELS, 0.0);

    let written = unsafe {
        // One plane: the output format is interleaved, so swr_convert wants an
        // array of exactly one pointer.
        let dst: [*mut u8; 1] = [out.as_mut_ptr().add(base) as *mut u8];
        ffi::swr_convert(
            state.resampler,
            dst.as_ptr(),
            capacity as c_int,
            input,
            in_samples,
        )
    };
    let written = Error::check("swr_convert", written)? as usize;
    out.truncate(base + written * AudioBuffer::CHANNELS);
    Ok(())
}

/// Owns the ffmpeg state for one decode, freeing it on drop.
#[derive(Default)]
struct AudioState {
    format: *mut ffi::AVFormatContext,
    codec: *mut ffi::AVCodecContext,
    resampler: *mut ffi::SwrContext,
    packet: *mut ffi::AVPacket,
    frame: *mut ffi::AVFrame,
}

impl Drop for AudioState {
    fn drop(&mut self) {
        // SAFETY: ffmpeg's free functions accept null and no-op.
        unsafe {
            if !self.frame.is_null() {
                ffi::av_frame_free(&mut self.frame);
            }
            if !self.packet.is_null() {
                ffi::av_packet_free(&mut self.packet);
            }
            if !self.resampler.is_null() {
                ffi::swr_free(&mut self.resampler);
            }
            if !self.codec.is_null() {
                ffi::avcodec_free_context(&mut self.codec);
            }
            if !self.format.is_null() {
                ffi::avformat_close_input(&mut self.format);
            }
        }
    }
}
