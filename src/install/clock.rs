//! Local wall-clock time, for the line a save slot shows.
//!
//! The save screen's display string is a timestamp: `FUN_10011b40` calls
//! `GetLocalTime` and formats it. Reproducing that needs the local civil date,
//! which the standard library does not provide — it offers a UTC instant and
//! nothing else.
//!
//! So this is the two pieces that turns one into the other: the civil date
//! from a UNIX timestamp, and the machine's offset from UTC. The offset comes
//! from the system zone file (`/etc/localtime`, in the TZif format every Unix
//! ships), read far enough to find the offset in force right now. A platform
//! with no such file — Windows — falls back to UTC and says so, which makes
//! the timestamp wrong by the zone offset rather than absent.
//!
//! Around 150 lines rather than a date crate, which is the trade this project
//! makes for a need this small.

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
/// Zero when there is no zone file to read, which is honest rather than silent:
/// a timestamp an hour out is better than no save line, and the log says why.
fn local_offset(secs: i64) -> i64 {
    match std::fs::read("/etc/localtime") {
        Ok(bytes) => tzif_offset(&bytes, secs).unwrap_or_else(|| {
            log::warn!("/etc/localtime is not a zone file this understands; timestamps are UTC");
            0
        }),
        Err(_) => {
            log::debug!("no /etc/localtime; save timestamps are UTC");
            0
        }
    }
}

/// The UTC offset a TZif file gives for `at`.
///
/// Only the version-1 block is read: it is present in every TZif file whatever
/// the version, its 32-bit transition times cover every date this will ever be
/// asked about, and reading it needs no 64-bit second block.
///
/// ```text
/// "TZif"  magic       4 bytes
/// version             1 byte
/// reserved            15 bytes
/// six counts          6 x u32 big-endian: isutcnt isstdcnt leapcnt
///                                        timecnt typecnt charcnt
/// timecnt x i32       transition times, ascending
/// timecnt x u8        the type index in force after each
/// typecnt x (i32,u8,u8)   offset, is_dst, abbreviation index
/// ```
fn tzif_offset(b: &[u8], at: i64) -> Option<i64> {
    if b.get(..4)? != b"TZif" {
        return None;
    }
    let u32be =
        |o: usize| -> Option<u32> { Some(u32::from_be_bytes(b.get(o..o + 4)?.try_into().ok()?)) };
    let timecnt = u32be(0x20)? as usize;
    let typecnt = u32be(0x24)? as usize;
    if typecnt == 0 {
        return None;
    }

    let times = 0x2c;
    let indices = times + timecnt * 4;
    let types = indices + timecnt;
    let offset_of = |kind: usize| -> Option<i64> {
        let at = types + kind * 6;
        Some(i32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?) as i64)
    };

    // The last transition at or before `at` names the type in force. Before
    // the first transition the file's first type applies.
    let mut kind = 0usize;
    for i in 0..timecnt {
        let when = i32::from_be_bytes(b.get(times + i * 4..times + i * 4 + 4)?.try_into().ok()?);
        if i64::from(when) > at {
            break;
        }
        kind = *b.get(indices + i)? as usize;
    }
    if kind >= typecnt {
        return None;
    }
    offset_of(kind)
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

    #[test]
    fn a_file_that_is_not_a_zone_file_yields_no_offset() {
        assert_eq!(tzif_offset(b"not a zone file at all", 0), None);
    }

    /// A minimal TZif with one transition: UTC before it, +1h after.
    #[test]
    fn reads_the_offset_in_force_at_a_moment() {
        let mut b = Vec::new();
        b.extend_from_slice(b"TZif2");
        b.extend_from_slice(&[0u8; 15]);
        for count in [0u32, 0, 0, 1, 2, 0] {
            b.extend_from_slice(&count.to_be_bytes());
        }
        b.extend_from_slice(&1_000_000i32.to_be_bytes()); // the transition
        b.push(1); // into type 1
        b.extend_from_slice(&0i32.to_be_bytes());
        b.extend_from_slice(&[0, 0]);
        b.extend_from_slice(&3600i32.to_be_bytes());
        b.extend_from_slice(&[1, 0]);
        assert_eq!(tzif_offset(&b, 999_999), Some(0));
        assert_eq!(tzif_offset(&b, 1_000_001), Some(3600));
    }
}
