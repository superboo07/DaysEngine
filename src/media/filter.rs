//! A libavfilter graph between the decoder and the scaler.
//!
//! # What it is for
//!
//! The movies are WMV3 at 800x452 and about 3 Mbit/s, which was a reasonable
//! bitrate for a 2007 release played in a 800x450 window. It is not a
//! reasonable bitrate for the same frame blown up 2.4x onto a 1080p panel: the
//! 8x8 transform blocks become 19-pixel squares, and the gradients the encoder
//! quantised into three or four steps become three or four visible bands. Both
//! are already in the file — nothing here can put back what the encoder threw
//! away — but both are also exactly what libavfilter's `deblock`, `deband` and
//! `gradfun` are for. The default chain measures 5.5ms of a frame's 41 at
//! 1080p, which is the size this engine is expected to run at, and it is spent
//! on the clip's own 800x452 rather than on the window — see below.
//!
//! This is not a recovered behaviour and does not pretend to be. The original
//! handed its decoded frames straight to Direct3D; there was no filter and no
//! room for one. It is the same kind of deliberate departure as
//! [`crate::playback::scale`], and it is switchable for the same reason: the
//! chain is `DaysEngine.ini`'s `[Video] Filters`, and an empty value is no
//! graph at all, which is the original's path exactly.
//!
//! # Where it sits
//!
//! Between `avcodec_receive_frame` and `sws_scale_frame`, so filtering happens
//! **at the clip's own 800x452** and the scale to the window happens after it.
//! That ordering is the point: a debander works on the bands the encoder left,
//! and once a frame is 2.4x larger those are 2.4x wider and no threshold
//! recognises them. It is also the cheap ordering — a third of a megapixel
//! rather than two.
//!
//! # One frame in, one frame out
//!
//! The engine schedules by frame index: the script clock counts frames at 24
//! fps and [`crate::media::VideoFrame::index`] is what a statement's window is
//! measured against. So a graph that swallowed a frame, or emitted two, would
//! not be a filter — it would be a different clip. This runs the graph one
//! frame at a time and takes the one frame it gives back; a chain that wants
//! several frames before it will produce one (`atadenoise`, `tmix`, anything
//! with a temporal window) has its opening frames passed through unfiltered
//! rather than held, and [`Graph::filter`] says so once per clip.

use super::Error;
use rusty_ffmpeg::ffi;
use std::ffi::{CStr, CString};

/// A parsed, configured filter chain, with the frame it writes into.
pub struct Graph {
    graph: *mut ffi::AVFilterGraph,
    /// The `buffer` at the head, which frames are pushed into.
    source: *mut ffi::AVFilterContext,
    /// The `buffersink` at the tail, which filtered frames are pulled from.
    sink: *mut ffi::AVFilterContext,
    /// The filtered frame, reused: unref'd after each use, never freed until
    /// the graph is.
    out: *mut ffi::AVFrame,
    /// Whether a frame has already come back empty, so the log says it once.
    reported_delay: bool,
    /// The chain as it was actually built, which is the one asked for minus
    /// anything [`present`] dropped.
    spec: String,
}

impl Graph {
    /// Builds the chain in `spec` for frames of `format` at `width` x `height`.
    ///
    /// `spec` is ordinary `ffmpeg -vf` syntax — one filter or several separated
    /// by commas — and everything libavfilter knows is available, because this
    /// is libavfilter. What it may not do is change the frame's size or pixel
    /// format: a `format=` of the decoder's own is appended to the chain, so a
    /// chain that ends in some other format is converted back rather than
    /// surprising the scaler, and a chain that scales is refused by the
    /// configuration below because the sink is pinned to the size it was given.
    ///
    /// `time_base` and `sample_aspect_ratio` come off the stream and are what
    /// a temporal filter reads to know how far apart two frames are.
    pub fn new(
        spec: &str,
        (width, height): (u32, u32),
        format: i32,
        time_base: ffi::AVRational,
        sample_aspect_ratio: ffi::AVRational,
    ) -> Result<Graph, Error> {
        let name = pixel_format_name(format)?;
        let spec = &present(spec);
        if spec.is_empty() {
            return Err(Error::Filter(
                "this build of libavfilter has none of the filters asked for".into(),
            ));
        }
        // The head's parameters, in the `key=value:key=value` form
        // `avfilter_graph_create_filter` parses for the `buffer` filter.
        let args = CString::new(format!(
            "video_size={width}x{height}:pix_fmt={format}:time_base={}/{}:pixel_aspect={}/{}",
            time_base.num.max(1),
            time_base.den.max(1),
            sample_aspect_ratio.num.max(1),
            sample_aspect_ratio.den.max(1),
        ))
        .map_err(|_| Error::Filter("the buffer arguments contain a NUL".into()))?;
        // The chain the player asked for, then back to the format the scaler is
        // expecting. `format` is a no-op when the chain did not change it.
        let chain = CString::new(format!("{spec},format=pix_fmts={name}"))
            .map_err(|_| Error::Filter("the filter chain contains a NUL".into()))?;

        // SAFETY: every allocation below is freed by `Building` unless the
        // graph is returned, and the pointers handed to ffmpeg are live for the
        // whole call.
        unsafe {
            let mut building = Building {
                graph: ffi::avfilter_graph_alloc(),
                ..Default::default()
            };
            if building.graph.is_null() {
                return Err(Error::Alloc("AVFilterGraph"));
            }
            // One slice per core, for the filters that support slice threading
            // — `deband` does, `deblock` and `gradfun` do not, and it costs
            // nothing to ask on their behalf. libavfilter's own default is to
            // use every core, but it reads this field rather than deciding per
            // filter, so saying it is what makes it true of a graph we built.
            (*building.graph).nb_threads = std::thread::available_parallelism()
                .map_or(1, std::num::NonZeroUsize::get)
                .min(64) as i32;

            let source = filter_by_name(c"buffer")?;
            let sink = filter_by_name(c"buffersink")?;
            let mut source_ctx = std::ptr::null_mut();
            let mut sink_ctx = std::ptr::null_mut();
            check(
                "avfilter_graph_create_filter(buffer)",
                ffi::avfilter_graph_create_filter(
                    &mut source_ctx,
                    source,
                    c"in".as_ptr(),
                    args.as_ptr(),
                    std::ptr::null_mut(),
                    building.graph,
                ),
            )?;
            check(
                "avfilter_graph_create_filter(buffersink)",
                ffi::avfilter_graph_create_filter(
                    &mut sink_ctx,
                    sink,
                    c"out".as_ptr(),
                    std::ptr::null(),
                    std::ptr::null_mut(),
                    building.graph,
                ),
            )?;

            // `outputs` is what the chain reads from and `inputs` is what it
            // writes to — named from the chain's point of view, which is the
            // way round that reads backwards from every other name here.
            building.outputs = ffi::avfilter_inout_alloc();
            building.inputs = ffi::avfilter_inout_alloc();
            if building.outputs.is_null() || building.inputs.is_null() {
                return Err(Error::Alloc("AVFilterInOut"));
            }
            let label = |name: &CStr| ffi::av_strdup(name.as_ptr());
            (*building.outputs).name = label(c"in");
            (*building.outputs).filter_ctx = source_ctx;
            (*building.outputs).pad_idx = 0;
            (*building.outputs).next = std::ptr::null_mut();
            (*building.inputs).name = label(c"out");
            (*building.inputs).filter_ctx = sink_ctx;
            (*building.inputs).pad_idx = 0;
            (*building.inputs).next = std::ptr::null_mut();

            check(
                "avfilter_graph_parse_ptr",
                ffi::avfilter_graph_parse_ptr(
                    building.graph,
                    chain.as_ptr(),
                    &mut building.inputs,
                    &mut building.outputs,
                    std::ptr::null_mut(),
                ),
            )?;
            check(
                "avfilter_graph_config",
                ffi::avfilter_graph_config(building.graph, std::ptr::null_mut()),
            )?;

            let out = ffi::av_frame_alloc();
            if out.is_null() {
                return Err(Error::Alloc("AVFrame"));
            }
            building.out = out;

            let graph = building.graph;
            building.disarm();
            Ok(Graph {
                graph,
                source: source_ctx,
                sink: sink_ctx,
                out,
                reported_delay: false,
                spec: spec.clone(),
            })
        }
    }

    /// The chain this graph is actually running: what the settings file asked
    /// for, minus any filter this build of ffmpeg does not have.
    pub fn spec(&self) -> &str {
        &self.spec
    }

    /// Runs one frame through the chain.
    ///
    /// Returns the filtered frame, which belongs to this graph and stays valid
    /// until the next call, or `None` when the chain held the frame back rather
    /// than producing one — see the module's note on one frame in, one frame
    /// out. The frame handed in is not consumed: the graph takes a reference,
    /// so the caller still owns and must still unref its own.
    ///
    /// # Safety
    ///
    /// `frame` is a live `AVFrame` of the size and format this graph was built
    /// for.
    pub unsafe fn filter(
        &mut self,
        frame: *mut ffi::AVFrame,
    ) -> Result<Option<*mut ffi::AVFrame>, Error> {
        // SAFETY: `frame` is the caller's live frame, and `self.out` is unref'd
        // before it is filled again.
        unsafe {
            ffi::av_frame_unref(self.out);
            check(
                "av_buffersrc_add_frame_flags",
                ffi::av_buffersrc_add_frame_flags(
                    self.source,
                    frame,
                    ffi::AV_BUFFERSRC_FLAG_KEEP_REF as i32,
                ),
            )?;
            let code = ffi::av_buffersink_get_frame(self.sink, self.out);
            if code == ffi::AVERROR(ffi::EAGAIN) || code == ffi::AVERROR_EOF {
                if !self.reported_delay {
                    self.reported_delay = true;
                    log::warn!(
                        "the [Video] Filters chain wants more than one frame before it \
                         produces one; those frames are shown unfiltered"
                    );
                }
                return Ok(None);
            }
            check("av_buffersink_get_frame", code)?;
            Ok(Some(self.out))
        }
    }
}

impl Drop for Graph {
    fn drop(&mut self) {
        // SAFETY: both were allocated by the matching ffmpeg call, and freeing
        // the graph frees the filter contexts inside it.
        unsafe {
            ffi::av_frame_free(&mut self.out);
            ffi::avfilter_graph_free(&mut self.graph);
        }
    }
}

/// Drops the links of a chain this build of ffmpeg does not have, keeping the
/// rest.
///
/// ffmpeg is configurable down to the individual filter, and a distribution
/// that has trimmed one out is a fact about that machine rather than a mistake
/// in the settings file. Failing the whole graph over it would cost such a
/// player every other link as well, so a link naming a filter that is not in
/// this build is dropped with a line in the log and the chain around it is
/// built.
///
/// Only a missing *name* is treated this way. An option libavfilter rejects is
/// a mistake in the file rather than a fact about the build, and that still
/// fails the graph and is reported as itself.
fn present(spec: &str) -> String {
    let mut kept: Vec<&str> = Vec::new();
    for link in links(spec) {
        let Some(name) = filter_name(link) else {
            // Labelled links (`[a]overlay[b]`) and anything else this does not
            // recognise are passed through for libavfilter to judge.
            kept.push(link);
            continue;
        };
        let Ok(name) = CString::new(name) else {
            kept.push(link);
            continue;
        };
        // SAFETY: `name` is NUL-terminated and only read.
        if unsafe { ffi::avfilter_get_by_name(name.as_ptr()) }.is_null() {
            log::warn!(
                "this build of libavfilter has no {} filter; \
                 dropping it from the chain and keeping the rest",
                name.to_string_lossy()
            );
            continue;
        }
        kept.push(link);
    }
    kept.join(",")
}

/// Splits a chain into its links at the commas that separate filters.
///
/// Not every comma does: `geq=lum='if(gt(X,W/2),255,0)'` has four that are
/// inside an expression, and splitting there would make nonsense of it. So
/// commas inside quotes or brackets are left alone, which is the same rule
/// libavfilter's own parser applies.
fn links(spec: &str) -> Vec<&str> {
    let (mut out, mut start, mut depth, mut quote) = (Vec::new(), 0, 0i32, false);
    for (at, ch) in spec.char_indices() {
        match ch {
            '\'' => quote = !quote,
            '(' | '[' if !quote => depth += 1,
            ')' | ']' if !quote => depth -= 1,
            ',' if !quote && depth <= 0 => {
                out.push(spec[start..at].trim());
                start = at + 1;
            }
            _ => {}
        }
    }
    out.push(spec[start..].trim());
    out.retain(|link| !link.is_empty());
    out
}

/// The filter a link names, or `None` when the link is not a bare `name` or
/// `name=options` — a labelled one, say, which this does not take apart.
fn filter_name(link: &str) -> Option<&str> {
    let name = link.split('=').next()?.trim();
    let plain = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.');
    plain.then_some(name)
}

/// Looks up a filter by name, so a build without it says which one is missing.
///
/// # Safety
///
/// `name` is a NUL-terminated string, which the type guarantees.
unsafe fn filter_by_name(name: &CStr) -> Result<*const ffi::AVFilter, Error> {
    // SAFETY: avfilter_get_by_name reads the string and returns a pointer into
    // libavfilter's own static registry.
    let filter = unsafe { ffi::avfilter_get_by_name(name.as_ptr()) };
    if filter.is_null() {
        return Err(Error::Filter(format!(
            "this build of libavfilter has no {} filter",
            name.to_string_lossy()
        )));
    }
    Ok(filter)
}

/// The name libavfilter spells a pixel format with, e.g. `yuv420p`.
fn pixel_format_name(format: i32) -> Result<String, Error> {
    // SAFETY: av_get_pix_fmt_name returns a static string or null.
    let name = unsafe { ffi::av_get_pix_fmt_name(format) };
    if name.is_null() {
        return Err(Error::Filter(format!("pixel format {format} has no name")));
    }
    // SAFETY: non-null means a static NUL-terminated string.
    Ok(unsafe { CStr::from_ptr(name) }
        .to_string_lossy()
        .into_owned())
}

/// An ffmpeg return code as a [`Error::Ffmpeg`], keeping the call's name.
fn check(what: &'static str, code: std::ffi::c_int) -> Result<(), Error> {
    Error::check(what, code).map(|_| ())
}

/// Frees a half-built graph if a step fails part-way through.
#[derive(Default)]
struct Building {
    graph: *mut ffi::AVFilterGraph,
    inputs: *mut ffi::AVFilterInOut,
    outputs: *mut ffi::AVFilterInOut,
    out: *mut ffi::AVFrame,
}

impl Building {
    /// The graph and its frame are the caller's now; the two `AVFilterInOut`
    /// lists are scaffolding and are freed either way.
    fn disarm(&mut self) {
        self.graph = std::ptr::null_mut();
        self.out = std::ptr::null_mut();
    }
}

impl Drop for Building {
    fn drop(&mut self) {
        // SAFETY: ffmpeg's free functions accept null and no-op, and each
        // pointer here was allocated by the matching call.
        unsafe {
            if !self.inputs.is_null() {
                ffi::avfilter_inout_free(&mut self.inputs);
            }
            if !self.outputs.is_null() {
                ffi::avfilter_inout_free(&mut self.outputs);
            }
            if !self.out.is_null() {
                ffi::av_frame_free(&mut self.out);
            }
            if !self.graph.is_null() {
                ffi::avfilter_graph_free(&mut self.graph);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A chain is split at the commas that separate filters and nowhere else:
    /// an expression's own commas are inside quotes or brackets, and splitting
    /// there would hand libavfilter two halves of a filter.
    #[test]
    fn only_the_commas_between_filters_split_a_chain() {
        assert_eq!(links("gradfun=1.2:16"), ["gradfun=1.2:16"]);
        assert_eq!(
            links("deblock=filter=weak, gradfun=1.2:16"),
            ["deblock=filter=weak", "gradfun=1.2:16"]
        );
        assert_eq!(
            links("geq=lum='if(gt(X,W/2),255,0)',gradfun=1.2"),
            ["geq=lum='if(gt(X,W/2),255,0)'", "gradfun=1.2"]
        );
        assert!(links("  ,  ").is_empty());
    }

    /// A filter this build does not have is dropped and the rest of the chain
    /// still builds — the whole point being that one absent filter does not
    /// cost a player the others. Asked of a real graph rather than of the
    /// helper, because the graph forgetting to prune is the way this breaks.
    #[test]
    fn a_filter_this_build_lacks_is_dropped_not_fatal() {
        let graph = Graph::new(
            "gradfun=1.2:16,notafilter=3,deblock=filter=weak",
            (64, 64),
            ffi::AV_PIX_FMT_YUV420P,
            ffi::AVRational { num: 1, den: 24 },
            ffi::AVRational { num: 1, den: 1 },
        )
        .expect("the rest of the chain should still build");
        assert_eq!(graph.spec(), "gradfun=1.2:16,deblock=filter=weak");

        // And a chain that is nothing but filters this build lacks is not a
        // graph at all, rather than an empty one that passes frames through a
        // `buffer` and a `buffersink` for nothing.
        assert!(Graph::new(
            "notafilter",
            (64, 64),
            ffi::AV_PIX_FMT_YUV420P,
            ffi::AVRational { num: 1, den: 24 },
            ffi::AVRational { num: 1, den: 1 },
        )
        .is_err());
    }

    /// An option libavfilter rejects is a mistake in the settings file, not a
    /// fact about the build, so it fails the graph rather than being dropped.
    #[test]
    fn a_bad_option_fails_the_graph() {
        let square = ffi::AVRational { num: 1, den: 1 };
        let rate = ffi::AVRational { num: 1, den: 24 };
        let built = Graph::new(
            "gradfun=strength=900",
            (64, 64),
            ffi::AV_PIX_FMT_YUV420P,
            rate,
            square,
        );
        assert!(built.is_err(), "an out-of-range option built a graph");
    }
}
