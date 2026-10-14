//! Where movie playback's time goes, one log line per clip.
//!
//! [`crate::media::Timings`] says what each stage of a decoded frame cost, and
//! `daysengine media --at-size` prints it for one clip with nothing else
//! running. That is half of the question. The other half only exists inside
//! the playback loop: decoding happens on the render thread, so a frame that
//! takes too long holds up input and drawing as well, and a loop that has
//! fallen behind decodes several frames in one pass to catch up. And the
//! texture upload, which is the GPU side of a frame, is not something an
//! inspection command can see at all.
//!
//! So the loop feeds every pass in here, and when a clip ends — another clip
//! takes over, the movie gives way to a still, or the script returns — this
//! logs what that clip cost. One line per clip rather than a running stream,
//! because a clip is a few seconds and that is the unit a slow scene is
//! noticed in. It goes through `log`, which on Android is logcat, so a device
//! with no profiler attached can still say where its frames went.

use super::stage::Started;
use crate::media::Timings;
use std::time::{Duration, Instant};

/// Where each clip's time goes, logged as the clip ends.
#[derive(Default)]
pub struct Profile {
    account: Account,
    /// When the last pass was accounted for, so the gap to the next one can
    /// be. Kept across clips: a stall at a cut is paid by the clip it cuts
    /// to, whose first frames it is that go unseen.
    previous: Option<Instant>,
}

/// What one clip has cost so far.
///
/// Apart from [`Profile`] so that starting a fresh account is an ordinary
/// assignment. `Profile` logs when it is dropped, and assigning over a value
/// drops the old one: a `Profile` resetting itself by assignment would run its
/// own `Drop`, which resets, which drops — recursion until the stack runs out,
/// which is how this crashed on a phone the first time a clip ended. An
/// `Account` has no `Drop`, so replacing one has no side effects.
#[derive(Default)]
struct Account {
    /// The clip being accounted for, as [`super::stage::Visual::movie_id`]
    /// names it. `None` while no movie is on screen.
    clip: Option<String>,
    /// The size the last picture was uploaded at.
    size: (u32, u32),
    /// Every frame decoded, including the ones a catch-up skipped past.
    decoded: Timings,
    /// Pictures uploaded, which are the frames that were actually seen.
    shown: u32,
    /// Time spent in those uploads.
    upload: Duration,
    /// When the first and the latest picture went up, for the rate.
    first: Option<Instant>,
    last: Option<Instant>,
    /// The longest a pass of the loop waited on the decoder.
    stall: Duration,
    /// The most frames a single pass decoded.
    most: u32,
    /// The pass that took longest over starting statements and decoding
    /// together, and how it divided.
    slowest: Option<Slowest>,
    /// The longest from one pass to the next. A movie frame is 41.7ms, so a
    /// gap longer than that is a picture that was never on screen, whatever
    /// the pass spent it on — the two above, or drawing, or anything else.
    gap: Duration,
}

/// The costliest pass of a clip.
#[derive(Debug, Clone, PartialEq)]
struct Slowest {
    /// Starting the statements the pass reached. See [`Started`].
    started: Started,
    /// Decoding the movie forward.
    decoding: Duration,
}

impl Profile {
    pub fn new() -> Profile {
        Profile::default()
    }

    /// One pass of the loop, at `now`: the clip on screen, what decoding it
    /// forward to this frame cost and how long the pass waited for that, and
    /// what starting the statements it reached cost.
    ///
    /// A different clip from the last pass's ends the old one's account,
    /// which is logged.
    pub fn pass(
        &mut self,
        clip: Option<&str>,
        decoded: &Timings,
        waited: Duration,
        started: &Started,
        now: Instant,
    ) {
        if clip != self.account.clip.as_deref() {
            self.log();
            self.account = Account {
                clip: clip.map(str::to_owned),
                ..Account::default()
            };
        }
        let gap = self.previous.map(|previous| now - previous);
        self.previous = Some(now);
        let account = &mut self.account;
        if account.clip.is_none() {
            return;
        }
        if let Some(gap) = gap {
            account.gap = account.gap.max(gap);
        }
        let cost = started.total + waited;
        if account
            .slowest
            .as_ref()
            .is_none_or(|slowest| cost > slowest.started.total + slowest.decoding)
        {
            account.slowest = Some(Slowest {
                started: started.clone(),
                decoding: waited,
            });
        }
        account.decoded.merge(decoded);
        let frames = decoded.frames + decoded.skipped;
        if frames > 0 {
            account.stall = account.stall.max(waited);
            account.most = account.most.max(frames);
        }
    }

    /// One picture uploaded to the GPU at `size`, which took `took`.
    pub fn shown(&mut self, size: (u32, u32), took: Duration, at: Instant) {
        let account = &mut self.account;
        account.size = size;
        account.shown += 1;
        account.upload += took;
        account.first.get_or_insert(at);
        account.last = Some(at);
    }

    /// The clip's account as one line, or `None` when there is no clip or
    /// nothing was decoded for it.
    pub fn summary(&self) -> Option<String> {
        let account = &self.account;
        let clip = account.clip.as_deref()?;
        if account.decoded.frames == 0 {
            return None;
        }
        let rate = match (account.first, account.last) {
            (Some(first), Some(last)) if account.shown > 1 && last > first => format!(
                " at {:.1} fps",
                f64::from(account.shown - 1) / (last - first).as_secs_f64()
            ),
            _ => String::new(),
        };
        let upload = account.upload.as_secs_f64() * 1000.0 / f64::from(account.shown.max(1));
        // The movies are all 24 fps, so this is what a frame has. An average
        // over it means the clip could not have played at speed.
        let budget = 1000.0 / 24.0;
        let average =
            account.decoded.total().as_secs_f64() * 1000.0 / f64::from(account.decoded.frames);
        let over = if average > budget {
            format!(" — over the {budget:.1}ms a 24 fps frame has")
        } else {
            String::new()
        };
        let ms = |d: Duration| d.as_secs_f64() * 1000.0;
        let slowest = match &account.slowest {
            Some(Slowest { started, decoding }) => format!(
                "; slowest pass {:.1}ms: {:.1}ms starting statements{}, {:.1}ms decoding",
                ms(started.total + *decoding),
                ms(started.total),
                match &started.slowest {
                    Some((took, what)) if !took.is_zero() => {
                        format!(" (most of all {what}, {:.1}ms)", ms(*took))
                    }
                    _ => String::new(),
                },
                ms(*decoding),
            ),
            None => String::new(),
        };
        Some(format!(
            "movie {clip} at {}x{}: {} frames decoded, {} shown{rate}; {}{over}; \
             upload {upload:.2}ms a picture; longest wait on the decoder {:.1}ms, \
             at most {} frames decoded in one pass{slowest}; \
             longest gap between passes {:.1}ms",
            account.size.0,
            account.size.1,
            account.decoded.frames + account.decoded.skipped,
            account.shown,
            account.decoded,
            ms(account.stall),
            account.most,
            ms(account.gap),
        ))
    }

    /// Logs the account, if there is one worth logging.
    fn log(&self) {
        if let Some(line) = self.summary() {
            log::info!("{line}");
        }
    }
}

/// A script that returns mid-clip still says what that clip cost.
impl Drop for Profile {
    fn drop(&mut self) {
        self.log();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frames(n: u32, each_ms: u64) -> Timings {
        Timings {
            frames: n,
            decode: Duration::from_millis(each_ms * u64::from(n)),
            worst: Duration::from_millis(each_ms),
            ..Timings::default()
        }
    }

    /// A clip's account covers every frame decoded for it, and the passes
    /// that decoded nothing — most of them, at a display rate above 24 —
    /// count towards neither the stall nor the catch-up.
    #[test]
    fn a_clip_is_accounted_for_across_passes() {
        let mut profile = Profile::new();
        let at = Instant::now();
        let ms = Duration::from_millis;
        let none = Started::default();
        profile.pass(Some("A"), &frames(1, 10), ms(11), &none, at);
        profile.shown((1920, 1080), ms(2), at);
        profile.pass(Some("A"), &Timings::default(), ms(1), &none, at + ms(17));
        profile.pass(Some("A"), &frames(3, 10), ms(31), &none, at + ms(70));
        profile.shown((1920, 1080), ms(4), at + ms(125));

        let line = profile.summary().expect("a clip with frames has a summary");
        assert!(line.contains("movie A at 1920x1080"), "{line}");
        assert!(
            line.contains("4 frames decoded, 2 shown at 8.0 fps"),
            "{line}"
        );
        assert!(line.contains("upload 3.00ms a picture"), "{line}");
        assert!(
            line.contains("longest wait on the decoder 31.0ms"),
            "{line}"
        );
        assert!(
            line.contains("at most 3 frames decoded in one pass"),
            "{line}"
        );
        assert!(line.contains("longest gap between passes 53.0ms"), "{line}");
        assert!(!line.contains("over the"), "{line}");
    }

    /// The slowest pass says how it divided between starting statements and
    /// decoding, and names the statement that cost most.
    #[test]
    fn the_slowest_pass_names_what_it_spent_its_time_on() {
        let mut profile = Profile::new();
        let at = Instant::now();
        let ms = Duration::from_millis;
        let opening = Started {
            total: ms(40),
            slowest: Some((ms(35), "PlayMovie Movie00/clip".to_string())),
        };
        profile.pass(Some("A"), &frames(3, 5), ms(15), &opening, at);
        profile.pass(
            Some("A"),
            &frames(1, 5),
            ms(20),
            &Started::default(),
            at + ms(42),
        );
        let line = profile.summary().unwrap();
        assert!(
            line.contains(
                "slowest pass 55.0ms: 40.0ms starting statements \
                 (most of all PlayMovie Movie00/clip, 35.0ms), 15.0ms decoding"
            ),
            "{line}"
        );
    }

    /// Another clip starts a fresh account, and so does a pass with no movie.
    #[test]
    fn a_new_clip_starts_a_new_account() {
        let mut profile = Profile::new();
        let at = Instant::now();
        let none = Started::default();
        profile.pass(Some("A"), &frames(5, 10), Duration::ZERO, &none, at);
        profile.pass(Some("B"), &frames(1, 50), Duration::ZERO, &none, at);
        let line = profile.summary().unwrap();
        assert!(line.starts_with("movie B"), "{line}");
        assert!(line.contains("1 frames decoded"), "{line}");
        assert!(line.contains("over the 41.7ms"), "{line}");

        profile.pass(None, &Timings::default(), Duration::ZERO, &none, at);
        assert_eq!(profile.summary(), None);
    }
}
