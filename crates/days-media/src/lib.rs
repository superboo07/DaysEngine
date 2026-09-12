//! Video and audio decoding for DaysEngine, over the **system** ffmpeg.
//!
//! The game's media is unusual in one helpful way: **the movies carry no audio
//! stream**. Every sound is a separate Ogg Vorbis file triggered by its own
//! script statement. So there is no muxed A/V sync problem — video and audio are
//! independent streams, both slaved to the 24 fps script clock.
//!
//! * Video is WMV3 (VC-1 Main), 800x452, 24 fps. Decoded lazily, frame by frame.
//! * Audio is Vorbis. Decoded eagerly to interleaved stereo f32 at the mixer
//!   rate, because clips are short and the mixer wants random access.
//!
//! This is the only crate in the workspace that uses `unsafe`. It is confined to
//! FFI calls; everything the rest of the engine touches is safe.

#![deny(unsafe_op_in_unsafe_fn)]

use rusty_ffmpeg::ffi;
use std::ffi::c_int;

mod io;

pub mod audio;
pub mod video;

pub use audio::{decode_audio, AudioBuffer};
pub use video::{VideoDecoder, VideoFrame};

/// Mixer sample rate, from `Ini/DX8SOUND.INI` (`SamplePerSec`).
pub const SAMPLE_RATE: u32 = 44_100;
/// Mixer channel count, from `Ini/DX8SOUND.INI` (`Channels`).
pub const CHANNELS: u32 = 2;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{what} failed: {}", describe(*.code))]
    Ffmpeg { what: &'static str, code: c_int },
    #[error("no {0} stream in this file")]
    NoStream(&'static str),
    #[error("no decoder available for codec {0}")]
    NoDecoder(String),
    #[error("out of memory allocating {0}")]
    Alloc(&'static str),
}

impl Error {
    fn check(what: &'static str, code: c_int) -> Result<c_int, Error> {
        if code < 0 {
            Err(Error::Ffmpeg { what, code })
        } else {
            Ok(code)
        }
    }
}

/// Renders an ffmpeg error code as its human-readable message.
fn describe(code: c_int) -> String {
    let mut buf = [0i8; ffi::AV_ERROR_MAX_STRING_SIZE as usize];
    // SAFETY: buf is correctly sized and av_strerror never writes past it.
    let ok = unsafe { ffi::av_strerror(code, buf.as_mut_ptr().cast(), buf.len()) } == 0;
    if !ok {
        return format!("error {code}");
    }
    // SAFETY: av_strerror wrote a NUL-terminated string on success.
    let s = unsafe { std::ffi::CStr::from_ptr(buf.as_ptr().cast()) };
    s.to_string_lossy().into_owned()
}

/// Version string of the ffmpeg we linked against. For diagnostics.
pub fn ffmpeg_version() -> String {
    // SAFETY: av_version_info returns a static NUL-terminated string.
    unsafe {
        std::ffi::CStr::from_ptr(ffi::av_version_info())
            .to_string_lossy()
            .into_owned()
    }
}
