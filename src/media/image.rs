//! Still-image scaling through libswscale.
//!
//! A background is a `.png` of the 800x452 stage and a movie is a clip of the
//! same stage; the only difference is where the pixels came from. They used to
//! reach the window by different routes — the clip through libswscale inside
//! its colour conversion, the still through the CPU kernel the menus use — and
//! two filters on the same stage is a seam the player can see. This puts a
//! still through the same scaler a movie goes through, with the same
//! [`VideoScaler`] filter out of `DaysEngine.ini`.
//!
//! Same filter, but not the same code underneath it, and deliberately so: a
//! still reaches swscale as packed RGBA and a movie frame as planar YUV, and
//! the backend that is right for one is four times too slow for the other.
//! `media::video::ScaleBackend` has the measurements that settled which goes
//! where.
//!
//! It is also why the mouth overlays are patched into the still *before* it
//! gets here. The original writes them straight into the background's own
//! surface with a `memcpy` (`FUN_00444b80`) and composites the result, so the
//! patch and the pixels around it are one image by the time anything scales
//! them. Scaling them separately and placing the smaller one by arithmetic puts
//! the patch on its own grid, which is what made it land beside the mouth it
//! belongs to rather than on it.

use super::video::{ScaleBackend, VideoScaler};
use super::Error;
use rusty_ffmpeg::ffi;

/// Scaled pixels, borrowed from the scaler's own output frame.
///
/// The rows carry whatever row padding libswscale's SIMD asked for, so
/// [`Self::pitch`] is the distance from one to the next and is not in general
/// `width * 4`. Everything that takes pixels here takes a pitch with them —
/// `SDL_UpdateTexture` does, and so does `playback::compose` — so
/// handing that one on is what lets the scale end at the scaler. Repacking into
/// a fresh `Vec` instead cost an allocate-and-zero plus a row-by-row copy of
/// the whole output: 3.2ms at 1920x1080 and 12.6ms at 3840x2160, every time a
/// scene changed.
pub struct Scaled<'a> {
    /// The scaled image, `pitch` bytes per row, `height` rows.
    pub rgba: &'a [u8],
    /// Bytes from the start of one row to the start of the next.
    pub pitch: usize,
}

/// A reusable libswscale context for packed-RGBA to packed-RGBA scaling.
///
/// Held across frames rather than rebuilt per call: the context carries the
/// filter tables and the slice threads, and building those for every background
/// change is work with nothing to show for it. libswscale reads the sizes from
/// the frames it is handed and reconfigures itself when they change, so one
/// context serves every size.
pub struct ImageScaler {
    scaler: *mut ffi::SwsContext,
    /// The scratch output frame, kept at the size last asked for.
    out: *mut ffi::AVFrame,
    /// The scratch input frame, kept at the size last handed in.
    src: *mut ffi::AVFrame,
    out_size: (u32, u32),
    src_size: (u32, u32),
}

impl ImageScaler {
    /// Builds a scaler using `filter`.
    pub fn new(filter: VideoScaler) -> Result<ImageScaler, Error> {
        let backend = ScaleBackend::for_still(filter);
        // SAFETY: every pointer is checked, and `Drop` frees each exactly once.
        let scaler = unsafe { super::video::new_scaler(filter, backend)? };
        Ok(ImageScaler {
            scaler,
            out: std::ptr::null_mut(),
            src: std::ptr::null_mut(),
            out_size: (0, 0),
            src_size: (0, 0),
        })
    }

    /// Scales `rgba`, which is `src` packed RGBA with no row padding, to `dst`.
    ///
    /// The result borrows the scaler's own output frame and stays valid until
    /// the next call, which is long enough for a caller to hand it to a texture
    /// or a compositor and is why nothing is copied on the way out.
    ///
    /// Returns `None` when the two sizes are equal — there is nothing to do and
    /// the caller already holds the pixels — and on a zero in any dimension or
    /// a buffer that is not the length `src` says it is.
    pub fn scale(
        &mut self,
        rgba: &[u8],
        src: (u32, u32),
        dst: (u32, u32),
    ) -> Result<Option<Scaled<'_>>, Error> {
        if src == dst
            || src.0 == 0
            || src.1 == 0
            || dst.0 == 0
            || dst.1 == 0
            || rgba.len() != src.0 as usize * src.1 as usize * 4
        {
            return Ok(None);
        }
        self.fit(src, dst)?;

        // SAFETY: `fit` has just made `self.src` an RGBA frame of exactly
        // `src` and `self.out` one of exactly `dst`; `rgba` is the packed
        // length checked above. Both frames are writable — nothing else holds
        // a reference to either, and the slice returned at the end borrows
        // `self` for as long as the caller keeps it.
        unsafe {
            if ffi::av_frame_make_writable(self.src) < 0 {
                return Err(Error::Alloc("AVFrame buffer"));
            }
            let into = (*self.src).data[0];
            let into_stride = (*self.src).linesize[0] as usize;
            for (row, line) in rgba.chunks_exact(src.0 as usize * 4).enumerate() {
                std::ptr::copy_nonoverlapping(
                    line.as_ptr(),
                    into.add(row * into_stride),
                    line.len(),
                );
            }

            let scaled = ffi::sws_scale_frame(self.scaler, self.out, self.src);
            if scaled < 0 {
                return Err(Error::Ffmpeg {
                    what: "sws_scale_frame",
                    code: scaled,
                });
            }

            // `av_frame_get_buffer` sizes the plane at `linesize * height`, so
            // that many bytes from `data[0]` are this frame's own.
            let pitch = (*self.out).linesize[0] as usize;
            Ok(Some(Scaled {
                rgba: std::slice::from_raw_parts((*self.out).data[0], pitch * dst.1 as usize),
                pitch,
            }))
        }
    }

    /// Makes the two scratch frames the sizes this call needs.
    fn fit(&mut self, src: (u32, u32), dst: (u32, u32)) -> Result<(), Error> {
        if self.src_size != src || self.src.is_null() {
            // SAFETY: the new frame is built before the old one is freed, so a
            // failure leaves the scaler holding the frame it already had.
            let frame = unsafe { super::video::new_rgba_frame(src)? };
            unsafe { ffi::av_frame_free(&mut self.src) };
            self.src = frame;
            self.src_size = src;
        }
        if self.out_size != dst || self.out.is_null() {
            // SAFETY: as above.
            let frame = unsafe { super::video::new_rgba_frame(dst)? };
            unsafe { ffi::av_frame_free(&mut self.out) };
            self.out = frame;
            self.out_size = dst;
        }
        Ok(())
    }
}

impl Drop for ImageScaler {
    fn drop(&mut self) {
        // SAFETY: each pointer was allocated by the matching ffmpeg call and is
        // freed exactly once. `av_frame_free` accepts a null pointer.
        unsafe {
            ffi::sws_freeContext(self.scaler);
            ffi::av_frame_free(&mut self.out);
            ffi::av_frame_free(&mut self.src);
        }
    }
}

/// The scratch frames and the context are owned outright by one `ImageScaler`
/// and never shared, so moving one between threads is sound. It is deliberately
/// not `Sync`: two threads scaling through one context at once is exactly what
/// the frames cannot take.
unsafe impl Send for ImageScaler {}

#[cfg(test)]
mod tests {
    use super::*;

    /// A flat colour, so a filter cannot change any pixel of it.
    fn flat(size: (u32, u32), colour: [u8; 4]) -> Vec<u8> {
        colour
            .iter()
            .copied()
            .cycle()
            .take(size.0 as usize * size.1 as usize * 4)
            .collect()
    }

    /// Asking for the size it already is gives `None` rather than a copy: the
    /// caller has those pixels, and at 1:1 there is nothing for a filter to do.
    #[test]
    fn scaling_to_the_same_size_is_nothing_to_do() {
        let mut scaler = ImageScaler::new(VideoScaler::Bicubic).expect("scaler");
        let src = flat((8, 8), [10, 20, 30, 255]);
        assert!(scaler.scale(&src, (8, 8), (8, 8)).expect("scale").is_none());
    }

    /// Every filter the player can name reaches a working scaler, at a size
    /// with an odd width and an odd height so that nothing lands on a
    /// convenient path. A flat colour survives all of them, so a backend that
    /// failed to build an operation list, or built the wrong one, shows up here
    /// rather than on the player's screen.
    #[test]
    fn every_filter_scales() {
        let src = flat((800, 452), [200, 120, 40, 255]);
        for name in VideoScaler::NAMES.split(", ") {
            let filter = VideoScaler::from_name(name).expect(name);
            let mut scaler = ImageScaler::new(filter).expect("scaler");
            let out = scaler
                .scale(&src, (800, 452), (1367, 771))
                .expect("scale")
                .expect("a different size scales");
            for (n, row) in out.rgba.chunks_exact(out.pitch).enumerate() {
                for (x, px) in row[..1367 * 4].as_chunks::<4>().0.iter().enumerate() {
                    assert_eq!(*px, [200, 120, 40, 255], "{name}: pixel {x} of row {n}");
                }
            }
        }
    }

    /// Fast bilinear is the one filter that stays on libswscale's legacy
    /// backend, because the ops backend has no fast bilinear and would silently
    /// give a still bicubic while the movie beside it kept fast bilinear. See
    /// `media::video::ScaleBackend`.
    #[test]
    fn fast_bilinear_is_the_one_filter_that_stays_legacy() {
        for name in VideoScaler::NAMES.split(", ") {
            let filter = VideoScaler::from_name(name).expect(name);
            let want = if filter == VideoScaler::FastBilinear {
                ScaleBackend::Legacy
            } else {
                ScaleBackend::Ops
            };
            assert_eq!(ScaleBackend::for_still(filter), want, "{name}");
        }
    }

    /// Every pixel of the image proper is reachable at the pitch reported, and
    /// a flat colour survives any filter unchanged. A pitch that did not match
    /// the rows behind it shows here as a sheared or truncated image — and the
    /// odd output height is there because an odd *width* would make swscale
    /// take a different path, so this keeps the one the game uses.
    #[test]
    fn a_flat_colour_survives_the_scale_at_the_pitch_reported() {
        let mut scaler = ImageScaler::new(VideoScaler::Bicubic).expect("scaler");
        let src = flat((800, 452), [17, 34, 51, 255]);
        let out = scaler
            .scale(&src, (800, 452), (1920, 1085))
            .expect("scale")
            .expect("a different size scales");
        assert!(out.pitch >= 1920 * 4, "pitch {} is short", out.pitch);
        assert_eq!(out.rgba.len(), out.pitch * 1085);
        for (n, row) in out.rgba.chunks_exact(out.pitch).enumerate() {
            for (x, px) in row[..1920 * 4].as_chunks::<4>().0.iter().enumerate() {
                assert_eq!(*px, [17, 34, 51, 255], "pixel {x} of row {n}");
            }
        }
    }
}
