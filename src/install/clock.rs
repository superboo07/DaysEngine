//! Local wall-clock time, for the line a save slot shows.
//!
//! The save screen's display string is a timestamp: `FUN_10011b40` calls
//! `GetLocalTime` and formats it. Reproducing that needs the local civil date,
//! which the standard library does not provide — it offers a UTC instant and
//! nothing else.
//!
//! So this is the two pieces that turn one into the other: the civil date from
//! a UNIX timestamp, and the machine's offset from UTC. The date arithmetic is
//! ours, because the standard library has none. The offset is SDL's: it already
//! asks each platform's own clock the question, and asking SDL is what this
//! engine does wherever SDL has the answer — so there is no zone-file reader
//! here and no second code path for Windows.

use sdl3::sys::time::{SDL_DateTime, SDL_TimeToDateTime};
use std::time::{SystemTime, UNIX_EPOCH};

/// A civil date and time, with the fields `GetLocalTime` fills in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Civil {
    pub year: i32,
    /// 1..=12.
    pub month: u32,
    /// 1..=31.
    pub day: u32,
    /// 0 for Sunday, as `SYSTEMTIME.wDayOfWeek` counts.
    pub weekday: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
}

/// The current local date and time.
pub fn now() -> Civil {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    civil_from_unix(secs + local_offset(secs))
}

/// The civil date and time a UNIX timestamp names, with no zone applied.
///
/// Howard Hinnant's `civil_from_days`: shift the epoch to the start of a
/// 400-year era beginning in March, so leap days land at the end of a year and
/// the month arithmetic has no special cases.
pub fn civil_from_unix(secs: i64) -> Civil {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);

    // 1970-01-01 was a Thursday, which is weekday 4 counting from Sunday.
    let weekday = (days + 4).rem_euclid(7) as u32;

    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = (yoe + era * 400 + i64::from(month <= 2)) as i32;

    Civil {
        year,
        month,
        day,
        weekday,
        hour: (rem / 3600) as u32,
        minute: (rem / 60 % 60) as u32,
        second: (rem % 60) as u32,
    }
}

/// The machine's offset from UTC in seconds, at `secs`.
///
/// `SDL_TimeToDateTime` is the platform's own local-time conversion — the zone
/// database on Unix, `SystemTimeToTzSpecificLocalTime` on Windows — and it
/// reports what it applied in `utc_offset`, seconds east of UTC. Only that
/// field is used: the civil fields are recomputed by [`civil_from_unix`], whose
/// weekday counts from Sunday the way `SYSTEMTIME.wDayOfWeek` does.
///
/// It needs no initialised subsystem, so the headless inspection subcommands
/// get the same answer the game does.
///
/// Zero when SDL declines to convert, which is honest rather than silent: a
/// timestamp an hour out is better than no save line, and the log says why.
#[allow(unsafe_code)]
fn local_offset(secs: i64) -> i64 {
    let mut dt = SDL_DateTime::default();
    // SDL counts nanoseconds from the same epoch. The saturating multiply is
    // for a clock so far out that nanoseconds overflow; the offset it then
    // reports is for a different century, which is the least of that machine's
    // problems.
    let ticks = secs.saturating_mul(1_000_000_000);
    // SAFETY: an FFI call with no safe binding in the `sdl3` crate. `dt` is a
    // live, initialised `SDL_DateTime` this frame owns, and SDL only writes
    // through the pointer for the duration of the call.
    let converted = unsafe { SDL_TimeToDateTime(ticks, &mut dt, true) };
    if !converted {
        log::debug!("SDL would not convert to local time; save timestamps are UTC");
        return 0;
    }
    i64::from(dt.utc_offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turns_the_epoch_into_its_own_date() {
        let c = civil_from_unix(0);
        assert_eq!((c.year, c.month, c.day), (1970, 1, 1));
        // 1970-01-01 was a Thursday.
        assert_eq!(c.weekday, 4);
        assert_eq!((c.hour, c.minute, c.second), (0, 0, 0));
    }

    /// The timestamp in the player's own `SaveFile000.DAT` line, which reads
    /// `2012年 1月28日(土)18:02` — Saturday, weekday 6.
    #[test]
    fn matches_a_date_out_of_a_real_save() {
        let c = civil_from_unix(1_327_773_720);
        assert_eq!((c.year, c.month, c.day), (2012, 1, 28));
        assert_eq!(c.weekday, 6);
        assert_eq!((c.hour, c.minute), (18, 2));
    }

    #[test]
    fn handles_a_leap_day_and_a_century_that_is_not_one() {
        let c = civil_from_unix(951_782_400); // 2000-02-29, a leap year
        assert_eq!((c.year, c.month, c.day), (2000, 2, 29));
        // 2100 is not a leap year, so the day after the 28th is March.
        let c = civil_from_unix(4_107_456_000);
        assert_eq!((c.year, c.month, c.day), (2100, 2, 28));
        let c = civil_from_unix(4_107_456_000 + 86_400);
        assert_eq!((c.year, c.month, c.day), (2100, 3, 1));
    }

    #[test]
    fn dates_before_the_epoch_go_backwards_rather_than_wrapping() {
        let c = civil_from_unix(-1);
        assert_eq!((c.year, c.month, c.day), (1969, 12, 31));
        assert_eq!((c.hour, c.minute, c.second), (23, 59, 59));
    }
}
