//! An `AVIOContext` backed by a byte slice in memory.
//!
//! Game media never exists as a file on disk — it lives inside a `.GPK`, and we
//! hand ffmpeg the decompressed bytes. Every clip is small (movies are a few
//! hundred KB to a couple of MB), so buffering the whole thing is simpler and
//! faster than streaming through the archive.

use rusty_ffmpeg::ffi;
use std::ffi::c_void;
use std::os::raw::c_int;

/// Size of the intermediate buffer ffmpeg reads through. 32 KiB is ffmpeg's own
/// default for file I/O.
const IO_BUFFER_SIZE: usize = 32 * 1024;

/// Owns a media buffer and the `AVIOContext` that reads it.
///
/// The `AVIOContext` holds a raw pointer back into `data`, so this type is not
/// movable-with-impunity: keep it boxed and alive for as long as the format
/// context that uses it.
pub struct MemoryIo {
    data: Box<[u8]>,
    position: usize,
    /// Allocated by `avio_alloc_context`; freed in `Drop`.
    context: *mut ffi::AVIOContext,
}

// The pointer is owned exclusively by this struct and never aliased; ffmpeg only
// touches it from whichever thread drives the decoder.
unsafe impl Send for MemoryIo {}

impl MemoryIo {
    /// Wraps a buffer. The returned box must outlive any format context built on it.
    pub fn new(data: Vec<u8>) -> Box<MemoryIo> {
        let mut io = Box::new(MemoryIo {
            data: data.into_boxed_slice(),
            position: 0,
            context: std::ptr::null_mut(),
        });

        // ffmpeg takes ownership of this buffer and may reallocate it, so it has
        // to come from av_malloc rather than Rust's allocator.
        let buffer = unsafe { ffi::av_malloc(IO_BUFFER_SIZE) } as *mut u8;
        assert!(!buffer.is_null(), "av_malloc failed for the AVIO buffer");

        let opaque = (&mut *io) as *mut MemoryIo as *mut c_void;
        let context = unsafe {
            ffi::avio_alloc_context(
                buffer,
                IO_BUFFER_SIZE as c_int,
                0, // read-only
                opaque,
                Some(read_packet),
                None,
                Some(seek),
            )
        };
        assert!(!context.is_null(), "avio_alloc_context failed");
        io.context = context;
        io
    }

    pub fn context(&self) -> *mut ffi::AVIOContext {
        self.context
    }
}

impl Drop for MemoryIo {
    fn drop(&mut self) {
        if self.context.is_null() {
            return;
        }
        unsafe {
            // `avio_context_free` does not free the read buffer, and because we
            // set AVFMT_FLAG_CUSTOM_IO the format context will not have freed it
            // either — so it is ours to release. ffmpeg may have replaced the
            // buffer we handed it, so free whatever the context currently holds.
            //
            // `av_freep` takes a pointer TO the pointer, not the pointer itself.
            ffi::av_freep(std::ptr::addr_of_mut!((*self.context).buffer) as *mut c_void);
            ffi::avio_context_free(&mut self.context);
        }
    }
}

/// ffmpeg read callback. Returns bytes read, or `AVERROR_EOF` at the end.
unsafe extern "C" fn read_packet(opaque: *mut c_void, buf: *mut u8, size: c_int) -> c_int {
    let io = unsafe { &mut *(opaque as *mut MemoryIo) };
    let remaining = io.data.len().saturating_sub(io.position);
    if remaining == 0 {
        return ffi::AVERROR_EOF;
    }
    let n = remaining.min(size.max(0) as usize);
    unsafe {
        std::ptr::copy_nonoverlapping(io.data.as_ptr().add(io.position), buf, n);
    }
    io.position += n;
    n as c_int
}

/// ffmpeg seek callback. Also answers `AVSEEK_SIZE`, which ASF demuxing needs.
unsafe extern "C" fn seek(opaque: *mut c_void, offset: i64, whence: c_int) -> i64 {
    let io = unsafe { &mut *(opaque as *mut MemoryIo) };
    let len = io.data.len() as i64;

    if whence & ffi::AVSEEK_SIZE as c_int != 0 {
        return len;
    }
    let base = match whence & !(ffi::AVSEEK_FORCE as c_int) {
        0 /* SEEK_SET */ => 0,
        1 /* SEEK_CUR */ => io.position as i64,
        2 /* SEEK_END */ => len,
        _ => return -1,
    };
    let target = base.saturating_add(offset);
    if !(0..=len).contains(&target) {
        return -1;
    }
    io.position = target as usize;
    target
}
