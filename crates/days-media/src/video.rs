//! WMV3 video decoding.

use crate::io::MemoryIo;
use crate::Error;
use rusty_ffmpeg::ffi;
use std::ffi::c_int;

/// One decoded frame, as tightly packed RGBA.
pub struct VideoFrame {
    pub width: u32,
    pub height: u32,
    /// `width * height * 4` bytes, row-major, no padding.
    pub rgba: Vec<u8>,
    /// 0-based position in the clip. This, not [`Self::timestamp`], is what the
    /// engine schedules against: the script clock counts frames at 24 fps and
    /// every movie in the game is constant frame rate.
    pub index: u64,
    /// Presentation time in seconds from the start of the clip.
    ///
    /// Taken from the container when it has one. The final flushed frame of an
    /// ASF clip carries no timestamp, so that case falls back to
    /// `index / frame_rate`.
    pub timestamp: f64,
}

/// A lazily-decoding video stream over an in-memory clip.
pub struct VideoDecoder {
    // Field order matters: `_io` must outlive `format`, and Rust drops in
    // declaration order, so `format` is declared first.
    format: *mut ffi::AVFormatContext,
    codec: *mut ffi::AVCodecContext,
    packet: *mut ffi::AVPacket,
    frame: *mut ffi::AVFrame,
    scaler: *mut ffi::SwsContext,
    _io: Box<MemoryIo>,
    stream_index: c_int,
    time_base: f64,
    frame_rate: f64,
    index: u64,
    width: u32,
    height: u32,
    /// Set once the demuxer is exhausted, so we drain the decoder exactly once.
    draining: bool,
    finished: bool,
}

// All pointers are owned solely by this struct.
unsafe impl Send for VideoDecoder {}

impl VideoDecoder {
    /// Opens a clip from its raw bytes (as read out of a `.GPK`).
    pub fn open(data: Vec<u8>) -> Result<VideoDecoder, Error> {
        let io = MemoryIo::new(data);

        // SAFETY: every pointer below is checked, and the whole constructor
        // unwinds through `Drop` on the partially built decoder only after the
        // struct is assembled, so early returns free what they allocated.
        unsafe {
            let format = ffi::avformat_alloc_context();
            if format.is_null() {
                return Err(Error::Alloc("AVFormatContext"));
            }
            (*format).pb = io.context();
            // Tell the demuxer not to go looking for a file on disk.
            (*format).flags |= ffi::AVFMT_FLAG_CUSTOM_IO as c_int;

            let mut format_ptr = format;
            let code = ffi::avformat_open_input(
                &mut format_ptr,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null_mut(),
            );
            if code < 0 {
                // avformat_open_input frees the context itself on failure.
                return Err(Error::Ffmpeg {
                    what: "avformat_open_input",
                    code,
                });
            }
            let format = format_ptr;

            let mut guard = Closing::new(format);
            Error::check(
                "avformat_find_stream_info",
                ffi::avformat_find_stream_info(format, std::ptr::null_mut()),
            )?;

            let stream_index = ffi::av_find_best_stream(
                format,
                ffi::AVMEDIA_TYPE_VIDEO,
                -1,
                -1,
                std::ptr::null_mut(),
                0,
            );
            if stream_index < 0 {
                return Err(Error::NoStream("video"));
            }

            let stream = *(*format).streams.offset(stream_index as isize);
            let params = (*stream).codecpar;
            let time_base = (*stream).time_base;
            let time_base = f64::from(time_base.num) / f64::from(time_base.den);

            // Used only to timestamp frames the container did not timestamp.
            let rate = |r: ffi::AVRational| {
                (r.num > 0 && r.den > 0).then(|| f64::from(r.num) / f64::from(r.den))
            };
            let frame_rate = rate((*stream).avg_frame_rate)
                .or_else(|| rate((*stream).r_frame_rate))
                .unwrap_or(24.0);

            let decoder = ffi::avcodec_find_decoder((*params).codec_id);
            if decoder.is_null() {
                return Err(Error::NoDecoder(format!("{:?}", (*params).codec_id)));
            }
            let codec = ffi::avcodec_alloc_context3(decoder);
            if codec.is_null() {
                return Err(Error::Alloc("AVCodecContext"));
            }
            guard.codec = codec;

            Error::check(
                "avcodec_parameters_to_context",
                ffi::avcodec_parameters_to_context(codec, params),
            )?;
            Error::check(
                "avcodec_open2",
                ffi::avcodec_open2(codec, decoder, std::ptr::null_mut()),
            )?;

            let width = (*codec).width.max(0) as u32;
            let height = (*codec).height.max(0) as u32;

            let packet = ffi::av_packet_alloc();
            let frame = ffi::av_frame_alloc();
            if packet.is_null() || frame.is_null() {
                return Err(Error::Alloc("AVPacket/AVFrame"));
            }
            guard.packet = packet;
            guard.frame = frame;

            // SWS_BILINEAR matches what the original D3D path would have done
            // when scaling; the conversion here is colour-space only (same size).
            let scaler = ffi::sws_getContext(
                width as c_int,
                height as c_int,
                (*codec).pix_fmt,
                width as c_int,
                height as c_int,
                ffi::AV_PIX_FMT_RGBA,
                ffi::SWS_BILINEAR as c_int,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            );
            if scaler.is_null() {
                return Err(Error::Alloc("SwsContext"));
            }

            guard.disarm();
            Ok(VideoDecoder {
                format,
                codec,
                packet,
                frame,
                scaler,
                _io: io,
                stream_index,
                time_base,
                frame_rate,
                index: 0,
                width,
                height,
                draining: false,
                finished: false,
            })
        }
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// Frames per second as the container declares it. Every retail movie is 24.
    pub fn frame_rate(&self) -> f64 {
        self.frame_rate
    }

    /// Decodes the next frame, or `None` at the end of the clip.
    pub fn next_frame(&mut self) -> Result<Option<VideoFrame>, Error> {
        if self.finished {
            return Ok(None);
        }
        loop {
            // SAFETY: all pointers are live for the lifetime of self.
            let code = unsafe { ffi::avcodec_receive_frame(self.codec, self.frame) };
            if code == 0 {
                return Ok(Some(self.convert_frame()?));
            }
            if code == ffi::AVERROR_EOF {
                self.finished = true;
                return Ok(None);
            }
            if code != ffi::AVERROR(ffi::EAGAIN) {
                return Err(Error::Ffmpeg {
                    what: "avcodec_receive_frame",
                    code,
                });
            }

            // Decoder wants more input.
            if self.draining {
                self.finished = true;
                return Ok(None);
            }
            self.feed()?;
        }
    }

    /// Pushes one packet from the demuxer into the decoder, or signals EOF.
    fn feed(&mut self) -> Result<(), Error> {
        loop {
            // SAFETY: format and packet are live.
            let code = unsafe { ffi::av_read_frame(self.format, self.packet) };
            if code == ffi::AVERROR_EOF {
                self.draining = true;
                // A null packet flushes the decoder's internal queue.
                // SAFETY: passing null is the documented flush protocol.
                unsafe { ffi::avcodec_send_packet(self.codec, std::ptr::null()) };
                return Ok(());
            }
            Error::check("av_read_frame", code)?;

            // SAFETY: packet is live and owned by us.
            let is_ours = unsafe { (*self.packet).stream_index } == self.stream_index;
            if !is_ours {
                unsafe { ffi::av_packet_unref(self.packet) };
                continue;
            }
            let send = unsafe { ffi::avcodec_send_packet(self.codec, self.packet) };
            unsafe { ffi::av_packet_unref(self.packet) };
            Error::check("avcodec_send_packet", send)?;
            return Ok(());
        }
    }

    /// Converts the decoder's current frame to packed RGBA.
    fn convert_frame(&mut self) -> Result<VideoFrame, Error> {
        let (w, h) = (self.width as usize, self.height as usize);
        let mut rgba = vec![0u8; w * h * 4];
        let stride = (w * 4) as c_int;

        // SAFETY: `rgba` is exactly height * stride bytes, which is what
        // sws_scale writes given a single plane and matching dimensions.
        let index = self.index;
        self.index += 1;

        let timestamp = unsafe {
            let dst_slices = [
                rgba.as_mut_ptr(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            ];
            let dst_strides = [stride, 0, 0, 0];
            ffi::sws_scale(
                self.scaler,
                (*self.frame).data.as_ptr() as *const *const u8,
                (*self.frame).linesize.as_ptr(),
                0,
                self.height as c_int,
                dst_slices.as_ptr(),
                dst_strides.as_ptr(),
            );

            let best = (*self.frame).best_effort_timestamp;
            let pts = if best == ffi::AV_NOPTS_VALUE {
                (*self.frame).pts
            } else {
                best
            };
            ffi::av_frame_unref(self.frame);
            if pts == ffi::AV_NOPTS_VALUE {
                index as f64 / self.frame_rate
            } else {
                pts as f64 * self.time_base
            }
        };

        Ok(VideoFrame {
            width: self.width,
            height: self.height,
            rgba,
            index,
            timestamp,
        })
    }
}

impl Drop for VideoDecoder {
    fn drop(&mut self) {
        // SAFETY: each pointer was allocated by the matching ffmpeg call and is
        // freed exactly once.
        unsafe {
            ffi::sws_freeContext(self.scaler);
            ffi::av_frame_free(&mut self.frame);
            ffi::av_packet_free(&mut self.packet);
            ffi::avcodec_free_context(&mut self.codec);
            ffi::avformat_close_input(&mut self.format);
        }
    }
}

/// Frees partially built ffmpeg state if a constructor bails out part-way.
struct Closing {
    format: *mut ffi::AVFormatContext,
    codec: *mut ffi::AVCodecContext,
    packet: *mut ffi::AVPacket,
    frame: *mut ffi::AVFrame,
}

impl Closing {
    fn disarm(&mut self) {
        self.format = std::ptr::null_mut();
        self.codec = std::ptr::null_mut();
        self.packet = std::ptr::null_mut();
        self.frame = std::ptr::null_mut();
    }
}

impl Drop for Closing {
    fn drop(&mut self) {
        // SAFETY: ffmpeg's free functions all accept null and no-op.
        unsafe {
            if !self.frame.is_null() {
                ffi::av_frame_free(&mut self.frame);
            }
            if !self.packet.is_null() {
                ffi::av_packet_free(&mut self.packet);
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

impl Closing {
    fn new(format: *mut ffi::AVFormatContext) -> Closing {
        Closing {
            format,
            codec: std::ptr::null_mut(),
            packet: std::ptr::null_mut(),
            frame: std::ptr::null_mut(),
        }
    }
}
