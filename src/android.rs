//! The Android entry point: what runs instead of `main`.
//!
//! An Android app has no `main`. The system starts an activity, the activity
//! loads shared libraries, and SDL's `SDLActivity` then looks up `SDL_main` in
//! the last of them and calls it on a thread of its own. So [`SDL_main`] below
//! is this platform's `src/main.rs`, and what it ends in is
//! [`game::run_with`] — the same loop every other platform runs, given no
//! arguments.
//!
//! Three things have to be true before that call, and none of them is true by
//! default on Android:
//!
//! * **The log has to go somewhere.** `env_logger` writes to standard error,
//!   which on Android is discarded. [`Logcat`] is the same `log` facade
//!   pointed at `__android_log_write` instead, so `adb logcat -s DaysEngine`
//!   shows exactly what a terminal shows on a desktop.
//! * **The install has to be found.** There is nothing to find: no working
//!   directory holds the game and no path names it. The player granted a
//!   folder, and [`install::saf`] is the backend that makes that folder look
//!   like one. Installing it is also how `discover_game_dir` learns the root.
//! * **`DaysEngine.ini` has to live somewhere writable.** "Beside the running
//!   binary" is `/system/bin` here. The activity's own files directory is what
//!   [`install::engine::set_directory`] is given.
//!
//! # Touch
//!
//! Nothing in the engine knows what a finger is, and nothing needs to. SDL
//! reports a touch as a mouse as well — [`TOUCH_MOUSE_EVENTS`] — so a tap
//! arrives as motion to where the finger landed followed by a left button
//! press at the same point, in that order, in one pump. That ordering is what
//! makes the menus work untouched: every screen here highlights what the
//! pointer is over and acts on the click, so the widget under the finger is
//! selected by the motion before the press is read.
//!
//! The reverse conversion is turned off. With both on, a genuine mouse — one
//! on a Chromebook, or over USB — would raise a synthetic touch beside its own
//! click and every press would arrive twice.

#![allow(unsafe_code)]

use crate::game;
use crate::install::{engine, saf, storage};
use std::ffi::{c_char, c_int, CString};
use std::path::PathBuf;

/// The tag every line of the log is filed under. `adb logcat -s DaysEngine`.
const TAG: &str = "DaysEngine";

/// SDL's hint for reporting a touch as a mouse as well. On by default; set
/// here so the pairing with [`MOUSE_TOUCH_EVENTS`] is in one place.
const TOUCH_MOUSE_EVENTS: &str = "SDL_TOUCH_MOUSE_EVENTS";

/// SDL's hint for reporting a mouse as a touch as well. See the module note.
const MOUSE_TOUCH_EVENTS: &str = "SDL_MOUSE_TOUCH_EVENTS";

// Android's own logging, out of liblog. There is no safe binding to call
// instead, and this is the whole of the FFI here.
#[link(name = "log")]
extern "C" {
    fn __android_log_write(priority: c_int, tag: *const c_char, text: *const c_char) -> c_int;
}

// The priorities `android/log.h` defines, of which these five are the ones
// `log::Level` maps onto.
const ANDROID_LOG_ERROR: c_int = 6;
const ANDROID_LOG_WARN: c_int = 5;
const ANDROID_LOG_INFO: c_int = 4;
const ANDROID_LOG_DEBUG: c_int = 3;
const ANDROID_LOG_VERBOSE: c_int = 2;

/// The `log` backend for this platform.
struct Logcat;

impl log::Log for Logcat {
    fn enabled(&self, _: &log::Metadata) -> bool {
        true
    }

    fn log(&self, record: &log::Record) {
        let priority = match record.level() {
            log::Level::Error => ANDROID_LOG_ERROR,
            log::Level::Warn => ANDROID_LOG_WARN,
            log::Level::Info => ANDROID_LOG_INFO,
            log::Level::Debug => ANDROID_LOG_DEBUG,
            log::Level::Trace => ANDROID_LOG_VERBOSE,
        };
        // An interior NUL cannot reach logcat as one string, and a log line is
        // never worth failing over: what cannot be represented is written with
        // the NULs shown rather than dropped.
        let text = record.args().to_string();
        let Ok(text) = CString::new(text.replace('\0', "\\0")) else {
            return;
        };
        let Ok(tag) = CString::new(TAG) else {
            return;
        };
        unsafe { __android_log_write(priority, tag.as_ptr(), text.as_ptr()) };
    }

    fn flush(&self) {}
}

static LOGCAT: Logcat = Logcat;

/// Points the `log` facade at logcat. Doing it twice is not an error.
fn start_logging() {
    if log::set_logger(&LOGCAT).is_ok() {
        log::set_max_level(log::LevelFilter::Info);
    }
}

/// What `SDLActivity` calls once the libraries are loaded.
///
/// # Safety
///
/// `argc` and `argv` are SDL's, and are ignored: see [`boot`].
#[no_mangle]
pub unsafe extern "C" fn SDL_main(_argc: c_int, _argv: *mut *mut c_char) -> c_int {
    match boot() {
        Ok(()) => 0,
        Err(err) => {
            log::error!("{err:?}");
            1
        }
    }
}

/// Everything [`SDL_main`] does, without the FFI signature around it.
///
/// The arguments SDL passes are Android's `app_process` command line and have
/// nothing to do with this engine — a script name or a `--game` would be read
/// out of them by accident — so the loop is given none. There is no command
/// line on a phone, and the inspection subcommands are a desktop tool.
fn boot() -> anyhow::Result<()> {
    start_logging();
    sdl3::hint::set(TOUCH_MOUSE_EVENTS, "1");
    sdl3::hint::set(MOUSE_TOUCH_EVENTS, "0");

    if !saf::present() {
        anyhow::bail!(
            "the Android storage bridge did not come up; \
             org.daysengine.Saf is missing from the app"
        );
    }
    let root = saf::root_document()
        .map_err(|err| anyhow::anyhow!("no granted folder to play from: {err}"))?;
    storage::install(Box::new(saf::Saf::new(root)))
        .map_err(|_| anyhow::anyhow!("the storage backend was already installed"))?;

    match saf::files_directory() {
        Ok(dir) => {
            let _ = engine::set_directory(PathBuf::from(dir));
        }
        // The settings are all defaults anyway if there is no file, so this
        // costs the player their `DaysEngine.ini` and not the session — the
        // rule a missing asset follows everywhere else here.
        Err(err) => log::warn!("no private directory for {}: {err}", engine::FILE),
    }

    game::run_with(Vec::new())
}
