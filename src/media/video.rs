//! WMV3 video decoding.

use super::filter;
use super::grain;
use super::io::MemoryIo;
use super::Error;
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

/// How a movie frame is scaled to the window.
///
/// These are libswscale's own filters, named as `ffmpeg -sws_flags` names them.
/// The scaling happens inside the colour conversion every frame already goes
/// through — see [`VideoDecoder::set_output_size`] — so which one is chosen
/// costs nothing but the filter's own width.
///
/// Which one a player gets is `DaysEngine.ini`'s `[Video] Scaler`; see
/// [`crate::install::engine`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VideoScaler {
    /// Fastest, and visibly so on the way up.
    FastBilinear,
    /// What the original's Direct3D path gives (`FUN_0044a3d0` sets
    /// `D3DTEXF_LINEAR`), for a player who wants that rather than better.
    Bilinear,
    /// Sharper than bilinear, cheap enough for 4K.
    Bicubic,
    /// Sharper still, and rings a little.
    Lanczos,
    /// The default: a natural bicubic spline, sharp without lanczos' ringing.
    #[default]
    Spline,
    /// Softer than bicubic, with no ringing at all.
    Gaussian,
    /// No filtering. Blocky, and here because someone will want it.
    Neighbour,
    /// Averages the source area, which is the right answer going *down*.
    Area,
}

impl VideoScaler {
    /// The `SWS_*` flag this is.
    pub fn flag(self) -> i64 {
        let flag = match self {
            VideoScaler::FastBilinear => ffi::SWS_FAST_BILINEAR,
            VideoScaler::Bilinear => ffi::SWS_BILINEAR,
            VideoScaler::Bicubic => ffi::SWS_BICUBIC,
            VideoScaler::Lanczos => ffi::SWS_LANCZOS,
            VideoScaler::Spline => ffi::SWS_SPLINE,
            VideoScaler::Gaussian => ffi::SWS_GAUSS,
            VideoScaler::Neighbour => ffi::SWS_POINT,
            VideoScaler::Area => ffi::SWS_AREA,
        };
        i64::from(flag)
    }

    /// Parses a name as `ffmpeg -sws_flags` spells it, give or take the
    /// separators and the spelling of "neighbour".
    pub fn from_name(name: &str) -> Option<VideoScaler> {
        Some(
            match name.trim().to_ascii_lowercase().replace(['_', '-'], "") {
                n if n == "fastbilinear" => VideoScaler::FastBilinear,
                n if n == "bilinear" || n == "linear" => VideoScaler::Bilinear,
                n if n == "bicubic" || n == "cubic" => VideoScaler::Bicubic,
                n if n == "lanczos" => VideoScaler::Lanczos,
                n if n == "spline" => VideoScaler::Spline,
                n if n == "gauss" || n == "gaussian" => VideoScaler::Gaussian,
                n if n == "neighbour" || n == "neighbor" || n == "point" || n == "nearest" => {
                    VideoScaler::Neighbour
                }
                n if n == "area" => VideoScaler::Area,
                _ => return None,
            },
        )
    }

    /// Every name this accepts, for the message that lists them.
    pub const NAMES: &'static str =
        "fast_bilinear, bilinear, bicubic, lanczos, spline, gaussian, neighbour, area";
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
    /// The scaler's output, RGBA at [`VideoDecoder::output_size`]. Kept and
    /// rewritten in place rather than allocated per frame.
    scaled: *mut ffi::AVFrame,
    /// Which filter [`VideoDecoder::scaler`] was built with.
    filter: VideoScaler,
    /// The `[Video] Filters` chain, and the graph built from it.
    ///
    /// The graph is built from the first frame rather than from the codec
    /// context, because the format a decoder declares before it has decoded
    /// anything is a guess and the graph has to be configured with the real
    /// one. A chain that will not build is reported once and left empty, which
    /// is the unfiltered path.
    filters: String,
    graph: Option<filter::Graph>,
    graph_failed: bool,
    /// The `[Video] FiltersAfterScale` chain, and its graph.
    ///
    /// The same machinery on the other side of the scale, in RGBA at the
    /// window's size. Rebuilt when that size changes, because a graph is
    /// configured for one size and the window is resizable.
    post_filters: String,
    post_graph: Option<filter::Graph>,
    post_failed: bool,
    /// The dither laid over the finished RGBA frame. See [`grain`].
    grain: grain::Grain,
    _io: Box<MemoryIo>,
    stream_index: c_int,
    time_base: f64,
    frame_rate: f64,
    index: u64,
    width: u32,
    height: u32,
    /// The size frames are handed back at, which is the size the window wants
    /// them. Starts at the clip's own.
    output: (u32, u32),
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

            let scaler = new_scaler(VideoScaler::default(), ScaleBackend::Legacy)?;
            let scaled = new_rgba_frame((width, height))?;

            guard.disarm();
            Ok(VideoDecoder {
                format,
                codec,
                packet,
                frame,
                scaler,
                scaled,
                _io: io,
                stream_index,
                time_base,
                frame_rate,
                index: 0,
                width,
                height,
                output: (width, height),
                filter: VideoScaler::default(),
                filters: String::new(),
                graph: None,
                graph_failed: false,
                post_filters: String::new(),
                post_graph: None,
                post_failed: false,
                grain: grain::Grain::new(0),
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

    /// Seeks back to the first frame, for a clip that is played on a loop.
    ///
    /// The frame indices [`VideoFrame::index`] hands back start again from
    /// zero, so a caller that counts frames across a loop keeps its own base —
    /// which is what the original does too, adding the clip's whole length to
    /// every timestamp after a wrap.
    pub fn rewind(&mut self) -> Result<(), Error> {
        // SAFETY: format and codec are live for the lifetime of self.
        let code = unsafe {
            ffi::av_seek_frame(
                self.format,
                self.stream_index,
                0,
                ffi::AVSEEK_FLAG_BACKWARD as c_int,
            )
        };
        Error::check("av_seek_frame", code)?;
        // The decoder is holding frames from before the seek, and draining may
        // already have been signalled; both are undone here, or the first
        // frame after a rewind is the last frame before it.
        // SAFETY: codec is live.
        unsafe { ffi::avcodec_flush_buffers(self.codec) };
        self.draining = false;
        self.finished = false;
        self.index = 0;
        Ok(())
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

    /// The size frames currently come back at.
    pub fn output_size(&self) -> (u32, u32) {
        self.output
    }

    /// Asks for frames at `width` x `height` instead of the clip's own size.
    ///
    /// This is how a movie gets onto a window that is not 800x452: the scale is
    /// folded into the colour conversion every frame already goes through, in
    /// libswscale's hand-written SIMD, rather than done again afterwards. A
    /// full-screen frame on a 4K panel is the case that makes the difference —
    /// scaling it is otherwise the most expensive thing in a frame, and one a
    /// 24 fps clock has no room for.
    ///
    /// A zero in either dimension, or the size it is already at, is ignored.
    pub fn set_output_size(&mut self, width: u32, height: u32) -> Result<(), Error> {
        if (width, height) == self.output || width == 0 || height == 0 {
            return Ok(());
        }
        // SAFETY: the new frame is built before the old one is freed, so a
        // failure leaves the decoder producing the size it already was.
        let scaled = unsafe { new_rgba_frame((width, height))? };
        unsafe { ffi::av_frame_free(&mut self.scaled) };
        self.scaled = scaled;
        self.output = (width, height);
        // The post-scale graph was configured for the old size.
        self.post_graph = None;
        Ok(())
    }

    /// Chooses the filter frames are scaled with, from `DaysEngine.ini`.
    ///
    /// Rebuilding the context is the whole of it: libswscale takes the sizes
    /// from the frames it is handed, so this carries no size with it.
    pub fn set_scaler(&mut self, filter: VideoScaler) -> Result<(), Error> {
        if self.filter == filter {
            return Ok(());
        }
        // SAFETY: the new context is built before the old one is freed, so a
        // failure leaves the decoder with the filter it already had.
        let scaler = unsafe { new_scaler(filter, ScaleBackend::Legacy)? };
        unsafe { ffi::sws_freeContext(self.scaler) };
        self.scaler = scaler;
        self.filter = filter;
        Ok(())
    }

    /// Sets the libavfilter chain frames are put through before they are
    /// scaled, from `DaysEngine.ini`'s `[Video] Filters`.
    ///
    /// An empty chain is no graph at all. A chain that will not build is a
    /// warning and then no graph either — a movie that plays with the artifacts
    /// the encoder left in it is a movie, and a typo in a settings file is not
    /// a reason to stop the game. See [`filter`].
    pub fn set_filters(&mut self, chain: &str) {
        let chain = chain.trim();
        if chain == self.filters {
            return;
        }
        chain.clone_into(&mut self.filters);
        self.graph = None;
        self.graph_failed = false;
    }

    /// Sets the chain frames are put through *after* they are scaled, from
    /// `DaysEngine.ini`'s `[Video] FiltersAfterScale`.
    ///
    /// The two stages are for two different kinds of work. What repairs the
    /// encode — a debander, a deblocker — belongs before the scale, where the
    /// artifacts are still the size the encoder made them. What is added to the
    /// picture belongs after it: grain laid down before a 2.4x upscale is not
    /// grain by the time it is seen, it is 2.4x blobs, and the whole point of a
    /// dither is that it sits on the grid the eye is looking at.
    pub fn set_post_filters(&mut self, chain: &str) {
        let chain = chain.trim();
        if chain == self.post_filters {
            return;
        }
        chain.clone_into(&mut self.post_filters);
        self.post_graph = None;
        self.post_failed = false;
    }

    /// Sets the amplitude of the grain laid over the finished frame, in levels
    /// of 255, from `DaysEngine.ini`'s `[Video] Grain`. Zero is none.
    pub fn set_grain(&mut self, amount: u8) {
        self.grain = grain::Grain::new(amount);
    }

    /// The chains in force, before and after the scale, for a caller that wants
    /// to report them.
    pub fn filters(&self) -> (&str, &str) {
        (&self.filters, &self.post_filters)
    }

    /// Puts the current frame through the filter graph, building it on the
    /// first frame, and returns the frame the scaler should read.
    ///
    /// Every way this can go wrong ends at the unfiltered frame: the chain is
    /// an enhancement, and none of what it does is worth losing a movie over.
    ///
    /// # Safety
    ///
    /// `self.frame` holds a decoded frame.
    unsafe fn filtered(&mut self) -> *mut ffi::AVFrame {
        if self.filters.is_empty() || self.graph_failed {
            return self.frame;
        }
        if self.graph.is_none() {
            // SAFETY: the caller's precondition.
            let (size, format, aspect) = unsafe {
                (
                    (
                        (*self.frame).width.max(0) as u32,
                        (*self.frame).height.max(0) as u32,
                    ),
                    (*self.frame).format,
                    (*self.frame).sample_aspect_ratio,
                )
            };
            // SAFETY: the stream is live for the lifetime of self.
            let time_base = unsafe { (*self.codec).pkt_timebase };
            match filter::Graph::new(&self.filters, size, format, time_base, aspect) {
                Ok(graph) => {
                    log::info!("[Video] Filters = {}", graph.spec());
                    self.graph = Some(graph);
                }
                Err(err) => {
                    log::warn!(
                        "[Video] Filters {:?}: {err}; frames go unfiltered",
                        self.filters
                    );
                    self.graph_failed = true;
                    return self.frame;
                }
            }
        }
        let Some(graph) = &mut self.graph else {
            return self.frame;
        };
        // SAFETY: `self.frame` is the decoded frame the graph was built from.
        match unsafe { graph.filter(self.frame) } {
            Ok(Some(filtered)) => filtered,
            Ok(None) => self.frame,
            Err(err) => {
                log::warn!(
                    "[Video] Filters {:?}: {err}; frames go unfiltered",
                    self.filters
                );
                self.graph_failed = true;
                self.graph = None;
                self.frame
            }
        }
    }

    /// Puts the scaled RGBA frame through the post-scale graph, building it on
    /// the first frame at the size the window currently wants.
    ///
    /// Ends at the unscaled-through frame every way it can go wrong, for the
    /// same reason [`VideoDecoder::filtered`] does.
    ///
    /// # Safety
    ///
    /// `self.scaled` holds the scaler's output.
    unsafe fn post_filtered(&mut self) -> *mut ffi::AVFrame {
        if self.post_filters.is_empty() || self.post_failed {
            return self.scaled;
        }
        if self.post_graph.is_none() {
            // One frame per frame and no timestamps involved, so the time base
            // is the clip's frame rate and nothing reads the aspect.
            let square = ffi::AVRational { num: 1, den: 1 };
            let rate = ffi::AVRational {
                num: 1,
                den: self.frame_rate.round().max(1.0) as i32,
            };
            match filter::Graph::new(
                &self.post_filters,
                self.output,
                ffi::AV_PIX_FMT_RGBA,
                rate,
                square,
            ) {
                Ok(graph) => {
                    log::info!(
                        "[Video] FiltersAfterScale = {} at {}x{}",
                        graph.spec(),
                        self.output.0,
                        self.output.1
                    );
                    self.post_graph = Some(graph);
                }
                Err(err) => {
                    log::warn!(
                        "[Video] FiltersAfterScale {:?}: {err}; frames go unfiltered",
                        self.post_filters
                    );
                    self.post_failed = true;
                    return self.scaled;
                }
            }
        }
        let scaled = self.scaled;
        let Some(graph) = &mut self.post_graph else {
            return scaled;
        };
        // SAFETY: `scaled` is the scaler's RGBA output at `self.output`, which
        // is what the graph was built for; a change of size rebuilt it.
        match unsafe { graph.filter(scaled) } {
            Ok(Some(filtered)) => filtered,
            Ok(None) => scaled,
            Err(err) => {
                log::warn!(
                    "[Video] FiltersAfterScale {:?}: {err}; frames go unfiltered",
                    self.post_filters
                );
                self.post_failed = true;
                self.post_graph = None;
                scaled
            }
        }
    }

    /// Converts the decoder's current frame to packed RGBA.
    fn convert_frame(&mut self) -> Result<VideoFrame, Error> {
        let (w, h) = (self.output.0 as usize, self.output.1 as usize);
        let mut rgba = vec![0u8; w * h * 4];

        // SAFETY: the scaler writes `self.scaled`, an RGBA frame of exactly
        // `w` x `h`, and `rgba` is exactly that many packed bytes.
        let index = self.index;
        self.index += 1;

        let timestamp = unsafe {
            // Filtered at the clip's own size, then scaled to the window: a
            // debander cannot see a band the scaler has already stretched.
            let source = self.filtered();
            let scaled = ffi::sws_scale_frame(self.scaler, self.scaled, source);
            if scaled < 0 {
                ffi::av_frame_unref(self.frame);
                return Err(Error::Ffmpeg {
                    what: "sws_scale_frame",
                    code: scaled,
                });
            }
            // Anything that is added to the picture rather than repaired in it
            // happens here, at the size it will be seen — see
            // [`VideoDecoder::set_post_filters`].
            let shown = self.post_filtered();
            // Out of the scaler's own rows, which carry whatever padding its
            // SIMD wanted, into the packed buffer the rest of the engine works
            // in.
            let from = (*shown).data[0];
            let from_stride = (*shown).linesize[0] as usize;
            for (row, out) in rgba.chunks_exact_mut(w * 4).enumerate() {
                std::ptr::copy_nonoverlapping(from.add(row * from_stride), out.as_mut_ptr(), w * 4);
            }

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

        // Last of all, on the frame at the size it will be seen: a dither laid
        // down before the scale is not a dither by the time it is shown.
        self.grain.apply(&mut rgba, w, index);

        Ok(VideoFrame {
            width: self.output.0,
            height: self.output.1,
            rgba,
            index,
            timestamp,
        })
    }
}

/// Which of libswscale's implementations a context may scale through.
///
/// libswscale 9 has two. The *legacy* backend is the one that has always been
/// there: it scales in an internal planar YUV representation, so a packed-RGB
/// source is converted to YUV on the way in and back to RGB on the way out. The
/// *ops* backend, new in 9, compiles a list of per-pixel operations and runs
/// them through SIMD kernels in whatever format the pixels are already in, so
/// an RGBA source is filtered as RGBA.
///
/// For a movie frame that costs nothing either way — the source is YUV420P
/// already, so the legacy backend's internal format *is* the source format and
/// the YUV-to-RGB step is the colour conversion the frame needs regardless.
/// For a still it is most of the cost of the scale. 800x452 to 1920x1080,
/// bicubic, eight slice threads, this machine:
///
/// ```text
///                           -> 1920x1080   -> 3840x2160
///   still  RGBA    legacy       30.4 ms       131.3 ms
///   still  RGBA    ops           9.5 ms        24.4 ms
///   movie  YUV420P legacy         7.3 ms        16.3 ms
///   movie  YUV420P ops            6.7 ms        19.3 ms
/// ```
///
/// The middle two rows are the same destination, the same filter and the same
/// slice threads; only the source pixel format differs. That is what says the
/// cost is the RGB round trip and not the kernel — and it is also why nearest
/// neighbour used to be no cheaper than bicubic here, since both paid it.
///
/// Two further things the legacy backend does to an RGB source, both from
/// `sws_init_context` in `libswscale/utils.c`: it forces `SWS_FULL_CHR_H_INT`
/// ("Forcing full internal H chroma due to input having non subsampled
/// chroma"), which puts the whole output through the full-chroma writer, and
/// there is no way for a caller to turn that back off. `SWS_FAST_BILINEAR` is
/// the single exception the condition carves out, which is why that one filter
/// was always the fast one.
///
/// [`Ops`](Self::Ops) is set through `SWS_UNSTABLE`, which upstream documents
/// as experimental. Two things make that a supportable thing to ship here: the
/// ffmpeg this links is vendored and pinned to an exact commit in
/// `third_party/`, so "semantics subject to change at any point in time" is a
/// change this project makes deliberately and re-tests; and the flag only
/// *adds* the ops backend to the set `graph.c:add_convert_pass` may choose
/// from, so anything it cannot compile an operation list for still falls back
/// to legacy rather than failing.
///
/// The output was checked against the legacy backend on three real backgrounds
/// from the game, at 1920x1080, 3840x2160, 1366x768 and 1367x771 — the last for
/// an odd width, which is its own path in `sws_init_context`. Bicubic, bilinear,
/// lanczos and area all differ by at most 7 of 255 in any channel, with a mean
/// absolute difference of 0.13 to 0.24: rounding. It is the same filter either
/// way, and swscale's own log names it — `SWS_OP_FILTER_H : 800 -> 1920 bicubic
/// (4 taps)` — so a still and a movie still go up through the filter the player
/// chose, which is the point of [`crate::media::image`].
///
/// Two divergences are real rather than rounding, and both are written down
/// here because a divergence nobody recorded is indistinguishable from one
/// nobody found:
///
/// - **`SWS_FAST_BILINEAR` has no ops-backend filter at all.** `format.c`'s
///   `get_scaler_fallback` maps the legacy flag bits onto the `SwsScaler` enum,
///   and that bit is not in the list, so it falls through to `SWS_SCALE_AUTO`,
///   which `filters.c` resolves to *bicubic*. A still would have been filtered
///   bicubic while the movie beside it kept legacy fast bilinear — exactly the
///   seam this engine puts stills through swscale to avoid. It measured as a
///   mean absolute difference of 3 to 6 of 255 against legacy, two orders of
///   magnitude above every other filter's. So [`Self::for_still`] keeps that
///   one filter on the legacy backend, where it is already the fast path.
/// - **Nearest neighbour picks a different source pixel at some output
///   columns.** Bit-identical between the two backends at 1920x1080 and
///   3840x2160, but at 1366x768 and 1367x771 between 0.1% and 0.5% of pixels
///   take the neighbouring source pixel instead. That is a half-pixel
///   difference in where the sample origin rounds, not a different filter;
///   both answers are nearest neighbour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ScaleBackend {
    /// The legacy backend only. What a movie frame wants.
    Legacy,
    /// Prefer the ops backend, falling back to legacy. What a still wants.
    Ops,
}

impl ScaleBackend {
    /// The backend a still should be scaled through with `filter`.
    ///
    /// Everything but [`VideoScaler::FastBilinear`], which the ops backend
    /// would quietly turn into bicubic; see the note above.
    pub(super) fn for_still(filter: VideoScaler) -> ScaleBackend {
        match filter {
            VideoScaler::FastBilinear => ScaleBackend::Legacy,
            _ => ScaleBackend::Ops,
        }
    }

    /// The `sws_flags` bits this adds.
    fn flag(self) -> i64 {
        match self {
            ScaleBackend::Legacy => 0,
            ScaleBackend::Ops => i64::from(ffi::SWS_UNSTABLE),
        }
    }
}

/// Builds the colour-conversion and scaling context.
///
/// Nothing about a size is set here. Modern libswscale reads what to do from
/// the two frames it is handed and reconfigures itself when they change, so all
/// this fixes is *how*:
///
/// The **filter**, which is where a movie frame is scaled now and not only
/// where it is converted. The original leaves scaling to Direct3D's bilinear
/// filter — `FUN_0044a3d0` sets `D3DTEXF_LINEAR` on every sampler stage — and
/// this engine's deliberate departure is to do better than a driver's bilinear,
/// so the default is [`VideoScaler::Spline`] and `DaysEngine.ini` can say
/// otherwise. At 1:1 it costs nothing either way: swscale takes its unscaled
/// path whatever the flag says.
///
/// **Slice threads**, which is why this goes the long way round through
/// `sws_alloc_context` rather than `sws_getContext`: the convenience
/// constructor has nowhere to put an option, the default is one thread, and one
/// thread scaling a frame to 4K costs more than the whole 41ms a frame gets.
/// Measured on eight cores, 800x452 to 3840x2160 with bicubic: 389ms on one
/// thread against 131ms on eight. The threads are also why
/// [`VideoDecoder::convert_frame`] calls `sws_scale_frame` and not `sws_scale`
/// — the older entry point ignores them.
///
/// And the **backend**, for which see [`ScaleBackend`].
///
/// # Safety
///
/// The returned context is owned by the caller and must be freed with
/// `sws_freeContext`.
pub(super) unsafe fn new_scaler(
    filter: VideoScaler,
    backend: ScaleBackend,
) -> Result<*mut ffi::SwsContext, Error> {
    // SAFETY: the context is freed on the one path that does not return it,
    // and both options below are ones swscale defines on its own context.
    unsafe {
        let scaler = ffi::sws_alloc_context();
        if scaler.is_null() {
            return Err(Error::Alloc("SwsContext"));
        }
        let opt = scaler.cast();
        let set =
            |name: &std::ffi::CStr, value: i64| ffi::av_opt_set_int(opt, name.as_ptr(), value, 0);
        let flags = set(c"sws_flags", filter.flag() | backend.flag());
        // One slice per core. A machine that will not say how many it has gets
        // one, which is swscale's own default.
        let threads = std::thread::available_parallelism()
            .map_or(1, std::num::NonZeroUsize::get)
            .min(64) as i64;
        let threaded = set(c"threads", threads);
        if flags < 0 || threaded < 0 {
            // Not fatal on its own — the defaults still convert — but it means
            // this build of swscale is not the one the timing assumes.
            log::warn!("libswscale refused its own options; video scaling will be slow");
        }
        Ok(scaler)
    }
}

/// Allocates an RGBA frame of `size` for the scaler to write into.
///
/// # Safety
///
/// The returned frame is owned by the caller and must be freed with
/// `av_frame_free`.
pub(super) unsafe fn new_rgba_frame(size: (u32, u32)) -> Result<*mut ffi::AVFrame, Error> {
    // SAFETY: the frame is freed on every path that does not return it.
    unsafe {
        let mut frame = ffi::av_frame_alloc();
        if frame.is_null() {
            return Err(Error::Alloc("AVFrame"));
        }
        (*frame).format = ffi::AV_PIX_FMT_RGBA;
        (*frame).width = size.0 as c_int;
        (*frame).height = size.1 as c_int;
        // 0 lets ffmpeg pick the row alignment its SIMD wants.
        if ffi::av_frame_get_buffer(frame, 0) < 0 {
            ffi::av_frame_free(&mut frame);
            return Err(Error::Alloc("AVFrame buffer"));
        }
        Ok(frame)
    }
}

impl Drop for VideoDecoder {
    fn drop(&mut self) {
        // SAFETY: each pointer was allocated by the matching ffmpeg call and is
        // freed exactly once.
        unsafe {
            ffi::sws_freeContext(self.scaler);
            ffi::av_frame_free(&mut self.scaled);
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
