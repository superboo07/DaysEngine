//! `daysengine` — the desktop entry point.
//!
//! Everything this program does is [`daysengine::game::run`]. It lives in the
//! library rather than here because Android has no `main` to put it in: there
//! the activity loads `libdaysengine.so` and SDL calls `SDL_main`, which is
//! `daysengine::android`. Both platforms run the same loop, and this file is
//! the desktop's four lines of it.

#![forbid(unsafe_code)]

fn main() -> anyhow::Result<()> {
    daysengine::game::run()
}
