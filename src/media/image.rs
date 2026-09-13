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
//! It is also why the mouth overlays are patched into the still *before* it
//! gets here. The original writes them straight into the background's own
//! surface with a `memcpy` (`FUN_00444b80`) and composites the result, so the
//! patch and the pixels around it are one image by the time anything scales
//! them. Scaling them separately and placing the smaller one by arithmetic puts
//! the patch on its own grid, which is what made it land beside the mouth it
//! belongs to rather than on it.

use super::video::VideoScaler;
use super::Error;
use rusty_ffmpeg::ffi;

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
        // SAFETY: every pointer is checked, and `Drop` frees each exactly once.
        let scaler = unsafe { super::video::new_scaler(filter)? };
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
    /// Returns `None` when the two sizes are equal — there is nothing to do and
    /// the caller already holds the pixels — and on a zero in any dimension or
    /// a buffer that is not the length `src` says it is.
    pub fn scale(
        &mut self,
        rgba: &[u8],
        src: (u32, u32),
        dst: (u32, u32),
    ) -> Result<Option<Vec<u8>>, Error> {
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

        let (w, h) = (dst.0 as usize, dst.1 as usize);
        let mut out = vec![0u8; w * h * 4];
        // SAFETY: `fit` has just made `self.src` an RGBA frame of exactly
        // `src` and `self.out` one of exactly `dst`; `rgba` is the packed
        // length checked above, and `out` is exactly `w * h * 4` bytes. Both
        // frames are writable — nothing else holds a reference to either.
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

            // Out of the scaler's own rows, which carry whatever padding its
            // SIMD wanted, into the packed buffer the rest of the engine works
            // in.
            let from = (*self.out).data[0];
            let from_stride = (*self.out).linesize[0] as usize;
            for (row, line) in out.chunks_exact_mut(w * 4).enumerate() {
                std::ptr::copy_nonoverlapping(
                    from.add(row * from_stride),
                    line.as_mut_ptr(),
                    w * 4,
                );
            }
        }
        Ok(Some(out))
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

    /// The output is exactly the packed size asked for, and a flat colour
    /// survives any filter unchanged — which is what says the rows came out of
    /// the scaler's padded stride and into a packed buffer correctly. A stride
    /// mistake shows here as a sheared or truncated image.
    #[test]
    fn a_flat_colour_scales_to_a_packed_buffer_of_the_same_colour() {
        let mut scaler = ImageScaler::new(VideoScaler::Bicubic).expect("scaler");
        let src = flat((800, 452), [17, 34, 51, 255]);
        let out = scaler
            .scale(&src, (800, 452), (1920, 1085))
            .expect("scale")
            .expect("a different size scales");
        assert_eq!(out.len(), 1920 * 1085 * 4);
        for (n, px) in out.as_chunks::<4>().0.iter().enumerate() {
            assert_eq!(*px, [17, 34, 51, 255], "pixel {n}");
        }
    }
}
