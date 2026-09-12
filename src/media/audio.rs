//! Ogg Vorbis decoding to the game's mixer format.
//!
//! Clips are decoded eagerly and completely. Voice lines and SFX are a few
//! seconds each and the longest BGM loops are a couple of MB decoded, which is
//! nothing next to the 12 GB of assets already on disk — and it means the mixer
//! gets random access without an ffmpeg context per playing sound.

use super::io::MemoryIo;
use super::{Error, CHANNELS, SAMPLE_RATE};
use rusty_ffmpeg::ffi;
use std::ffi::c_int;

/// Fully decoded audio, interleaved stereo `f32` at [`SAMPLE_RATE`].
#[derive(Clone)]
pub struct AudioBuffer {
    /// Interleaved L,R,L,R... in `-1.0..=1.0`.
    pub samples: Vec<f32>,
}

impl AudioBuffer {
    pub const CHANNELS: usize = CHANNELS as usize;

    pub fn silent() -> AudioBuffer {
        AudioBuffer {
            samples: Vec::new(),
        }
    }

    /// Number of sample frames (one frame = one sample per channel).
    pub fn frames(&self) -> usize {
        self.samples.len() / Self::CHANNELS
    }

    pub fn duration_seconds(&self) -> f64 {
        self.frames() as f64 / f64::from(SAMPLE_RATE)
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }
}

impl std::fmt::Debug for AudioBuffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioBuffer")
            .field("frames", &self.frames())
            .field("seconds", &self.duration_seconds())
            .finish()
    }
}

/// Decodes a complete audio file, resampling to the mixer's format.
pub fn decode_audio(data: Vec<u8>) -> Result<AudioBuffer, Error> {
    let io = MemoryIo::new(data);
    let mut state = AudioState::default();

    // SAFETY: `state` frees everything it owns on drop, including on the early
    // returns below, and `io` outlives the format context.
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

        let mut samples: Vec<f32> = Vec::new();

        loop {
            let code = ffi::av_read_frame(state.format, state.packet);
            if code == ffi::AVERROR_EOF {
                break;
            }
            Error::check("av_read_frame", code)?;

            if (*state.packet).stream_index != index {
                ffi::av_packet_unref(state.packet);
                continue;
            }
            let send = ffi::avcodec_send_packet(state.codec, state.packet);
            ffi::av_packet_unref(state.packet);
            Error::check("avcodec_send_packet", send)?;
            drain(&state, &mut samples)?;
        }

        // Flush.
        ffi::avcodec_send_packet(state.codec, std::ptr::null());
        drain(&state, &mut samples)?;
        flush_resampler(&state, &mut samples)?;

        Ok(AudioBuffer { samples })
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
