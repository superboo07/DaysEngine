//! Parser for FILMEngine `.ORS` scripts.
//!
//! An `.ORS` file is a **timeline**, not a program. Every statement carries a
//! start and an end timecode and the engine plays the whole file against one
//! clock, closer to a video editor's EDL than to VN bytecode:
//!
//! ```text
//! [PlayMovie]=00:04:05\tMovie00/00-00/00-00-A00/00-00-A00-001\t0\t00:11:00;
//! [PrintText]=00:30:13\tMakoto\t......\t00:31:11;
//! ```
//!
//! Statements are `[Command]=arg\targ\t...;`. The first argument is always the
//! start timecode and the last is always the end timecode, except for
//! [`Command::SkipFrame`] and [`Command::Next`], which carry a single timecode.
//!
//! Those two are the script's two boundaries and they are not the same one.
//! `[Next]` is where the script ends; `[SkipFRAME]` is where the control bar's
//! skip button jumps to, which is the frame the choice is raised at when the
//! script has one. They coincide in the 1,570 retail scripts with no choice
//! and differ in the 287 that have one, where `[SkipFRAME]` is exactly the
//! `[SetSELECT]` start. See [`Script::length`] and [`Script::skip_to`].
//!
//! Timecodes are `MM:SS:FF` at **24 fps** — the frames field runs 0..=23 across
//! all 1,857 retail scripts, and the movies are 24 fps.

#![forbid(unsafe_code)]

use std::time::Duration;

/// Frames per second of the script clock, and of every movie in the game.
pub const FPS: u32 = 24;

/// A point on the script timeline, in frames from the start.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct Frame(pub u32);

impl Frame {
    pub const ZERO: Frame = Frame(0);

    /// Parses an `MM:SS:FF` timecode.
    ///
    /// Out-of-range frame fields are folded in rather than rejected: two
    /// statements in the retail English scripts carry `:26` in a 24 fps field
    /// (`01-00-E01`), and the original engine plays them without complaint.
    pub fn parse(s: &str) -> Result<Frame, Error> {
        let bad = || Error::BadTimecode(s.to_string());
        let mut parts = s.trim().split(':');
        let mm: u32 = parts.next().ok_or_else(bad)?.parse().map_err(|_| bad())?;
        let ss: u32 = parts.next().ok_or_else(bad)?.parse().map_err(|_| bad())?;
        let ff: u32 = parts.next().ok_or_else(bad)?.parse().map_err(|_| bad())?;
        if parts.next().is_some() {
            return Err(bad());
        }
        if ff >= FPS {
            log::debug!("timecode {s} has an out-of-range frame field; folding it in");
        }
        Ok(Frame((mm * 60 + ss) * FPS + ff))
    }

    pub fn as_duration(self) -> Duration {
        Duration::from_nanos(u64::from(self.0) * 1_000_000_000 / u64::from(FPS))
    }

    pub fn from_duration(d: Duration) -> Frame {
        Frame::from_duration_at(d, 1.0)
    }

    /// Elapsed wall time as frames, scaled by a playback rate.
    ///
    /// This is the shape of the executable's own clock. `FUN_00422f70` reads
    /// the frame the session is at as
    ///
    /// ```text
    /// frame = base + ROUND(elapsed_ms * fps * rate) / 1000
    /// ```
    ///
    /// where `base` is the object's `+0x540`, `elapsed_ms` is `timeGetTime()`
    /// less the origin it kept at `+0x550`, `fps` is `DAT_0050c468` and `rate`
    /// is the float at `+0x538` that host slot `+0x8c` stores out of the speed
    /// table. The millisecond truncation is the original's, so it is kept: the
    /// product is formed from whole milliseconds and rounded before the
    /// divide, which is not the same as scaling seconds directly.
    ///
    /// A rate of 1.0 is what `FUN_00423130` initialises `+0x538` to, so the
    /// unscaled clock is this function with the rate the session starts at.
    pub fn from_duration_at(d: Duration, rate: f32) -> Frame {
        let ms = d.as_millis().min(u128::from(u32::MAX)) as u32;
        let scaled = (f64::from(ms) * f64::from(FPS) * f64::from(rate)).round() / 1000.0;
        Frame(scaled.clamp(0.0, f64::from(u32::MAX)) as u32)
    }

    pub fn as_seconds(self) -> f64 {
        f64::from(self.0) / f64::from(FPS)
    }
}

impl std::fmt::Display for Frame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let total_seconds = self.0 / FPS;
        write!(
            f,
            "{:02}:{:02}:{:02}",
            total_seconds / 60,
            total_seconds % 60,
            self.0 % FPS
        )
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("malformed timecode {0:?}")]
    BadTimecode(String),
    #[error("line {line}: [{command}] wants {want} arguments, got {got}")]
    Arity {
        line: usize,
        command: String,
        want: &'static str,
        got: usize,
    },
    #[error("line {line}: {value:?} is not a valid {what}")]
    BadValue {
        line: usize,
        what: &'static str,
        value: String,
    },
    #[error("script is not valid UTF-8 or UTF-16")]
    BadEncoding,
}

/// Direction of a screen fade.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fade {
    /// Fade from the colour to the scene.
    In,
    /// Fade from the scene to the colour.
    Out,
}

/// One statement's payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Dialogue line. The speaker is a display name, not an ID.
    PrintText {
        speaker: String,
        text: String,
    },
    /// Voice clip. `men_voice` is the engine's 0/1 flag for "this line is a
    /// male character"; the retail engine drops the clip when the player has
    /// turned the `MenVoice` option off (`FUN_0044e800` calls the menu DLL's
    /// `GetMenVoice` export and returns without playing when it is zero).
    /// `tag` is a short speaker code (`kot`, `sek`, `xxx` for narration) and is
    /// what selects the lip-sync mouth overlays; see `crate::lipsync` in the
    /// engine.
    PlayVoice {
        path: String,
        men_voice: bool,
        tag: String,
    },
    /// Still background. The only kind seen in retail data is `BGS`.
    CreateBg {
        kind: String,
        path: String,
    },
    /// Sound effect on one of the game's nine sound slots.
    ///
    /// `slot` is the index outright: the engine's `[PlaySe]` arm stores at
    /// `slot * 4 + 0x39c` with no adjustment, so `0` is a slot like any other
    /// and `8` is the ninth. Retail scripts use all nine. Slots are sometimes
    /// handed a `Voice...` path — the game reuses them for voice that is not
    /// meant to drive a mouth overlay.
    PlaySe {
        slot: u8,
        path: String,
    },
    PlayMovie {
        path: String,
        looping: bool,
    },
    PlayBgm {
        path: String,
    },
    /// A one-shot into sound slot 8, despite the name.
    ///
    /// The `[EndBGM]` arm stops slot 8 and stores its own sound there, opening
    /// it unlooped and with no `_int`/`_loop` pair — so it is a sound effect on
    /// the sound-effect volume, not a music stream, and a later `[PlaySe]` on
    /// slot 8 cuts it off.
    EndBgm {
        path: String,
    },
    /// Ending credits movie.
    EndRoll {
        path: String,
    },
    BlackFade(Fade),
    WhiteFade(Fade),
    /// A binary choice. **Carries no targets** — where each choice leads is
    /// decided by `RouteProcSDHQ.dll`, not by the script. `b` is `None` when
    /// the script writes `null`, which makes it a single-option prompt.
    SetSelect {
        a: String,
        b: Option<String>,
    },
    /// Drives the SOMCON peripheral for the length of its window.
    ///
    /// The intensity is 1 to 5 in every retail script. What each one means as
    /// a level, and the conditions the engine puts in front of the statement,
    /// are the runtime's — see `daysengine::playback::som`. With no device
    /// attached the statement does nothing, which is the engine's answer and
    /// not the parser's.
    MoveSom {
        intensity: i32,
    },
    /// Where the "skip" control jumps to: the frame the choice is raised at,
    /// or the end of the script when there is no choice. Always the first
    /// statement. See [`Script::skip_to`].
    SkipFrame,
    /// End of script. Always the last statement. See [`Script::length`].
    Next,
}

/// A statement: a command and the window it occupies on the timeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    pub start: Frame,
    pub end: Frame,
    pub command: Command,
}

impl Event {
    /// True while `at` falls inside `[start, end)`.
    pub fn is_active_at(&self, at: Frame) -> bool {
        at >= self.start && at < self.end
    }

    pub fn duration(&self) -> Frame {
        Frame(self.end.0.saturating_sub(self.start.0))
    }
}

/// A parsed script, with events sorted by start time.
#[derive(Debug, Clone, Default)]
pub struct Script {
    /// Script name as the route layer knows it, e.g. `"00-00-A00"`.
    pub name: String,
    /// Total length, from `[Next]` — or `[Exit]`, which the executable treats
    /// as the same statement. `FUN_0043b640` parses either into the timeline
    /// object's `+0x21c`, which `FUN_004315a0` reads back as the end.
    pub length: Frame,
    /// Where the control bar's skip button jumps to, from `[SkipFRAME]`:
    /// `FUN_0043b640` parses it into the timeline object's `+0x22c` and
    /// `FUN_004315c0` reads it back.
    ///
    /// This is what the file says. Whether the engine *records* it is a
    /// separate question — the parse arm is gated on the engine's skip flag —
    /// and that belongs to the player, not to the format: see
    /// `daysengine::playback::stage::apply_skip_flag`.
    ///
    /// In the 287 retail scripts that raise a choice this is exactly the
    /// `[SetSELECT]` start; in the other 1,570 it equals [`Script::length`],
    /// which is how the engine tells "no choice ahead" from "a choice at
    /// frame *n*" — see `FUN_00425bf0`'s case 6.
    pub skip_to: Frame,
    pub events: Vec<Event>,
}

impl Script {
    /// Parses a script from raw pack bytes.
    ///
    /// Handles both encodings the engine supports: the English scripts are
    /// UTF-8, and `FILMENGINE.INI [UnicodeFile]` selects UTF-16LE elsewhere.
    pub fn parse(name: &str, bytes: &[u8]) -> Result<Script, Error> {
        let text = decode(bytes)?;
        Self::parse_str(name, &text)
    }

    pub fn parse_str(name: &str, text: &str) -> Result<Script, Error> {
        let mut events = Vec::new();
        let mut length = Frame::ZERO;
        let mut skip_to = Frame::ZERO;

        for (line, raw) in statements(text) {
            let Some((command, body)) = split_statement(raw) else {
                continue;
            };
            let args: Vec<&str> = split_fields(body);
            let Some(event) = parse_command(line, command, &args)? else {
                continue;
            };
            match event.command {
                Command::Next => length = event.start,
                Command::SkipFrame => skip_to = event.start,
                _ => {}
            }
            events.push(event);
        }

        // Statements appear in start order in retail data, but nothing in the
        // format guarantees it and the player relies on it.
        events.sort_by_key(|e| (e.start, e.end));

        if length == Frame::ZERO {
            length = events.iter().map(|e| e.end).max().unwrap_or(Frame::ZERO);
            log::warn!("{name} has no [Next]; inferred length {length}");
        }
        // No `[SkipFRAME]` means nothing to skip to, which the engine spells as
        // a target equal to the end: `FUN_00425bf0`'s case 6 compares the two
        // and finishes the script when they match.
        if skip_to == Frame::ZERO {
            skip_to = length;
        }

        Ok(Script {
            name: name.to_string(),
            length,
            skip_to,
            events,
        })
    }

    /// Events whose window contains `at`.
    pub fn active_at(&self, at: Frame) -> impl Iterator<Item = &Event> {
        self.events.iter().filter(move |e| e.is_active_at(at))
    }

    /// Events starting in `(after, upto]` — the frames to fire when the clock
    /// advances. Half-open at the bottom so a frame is never fired twice.
    pub fn events_between(&self, after: Frame, upto: Frame) -> impl Iterator<Item = &Event> {
        self.events
            .iter()
            .filter(move |e| e.start > after && e.start <= upto)
    }

    /// The choice this script ends on, if any.
    pub fn selection(&self) -> Option<&Event> {
        self.events
            .iter()
            .find(|e| matches!(e.command, Command::SetSelect { .. }))
    }
}

/// Splits a script into `(line_number, statement)` pairs.
///
/// Statements end at `;`, but the format has no escaping and dialogue is free
/// text, so a bare `;` inside a line is ambiguous. `05-KC-F00` contains
/// `I know; I am, too.` — splitting naively there swallows the rest of the
/// statement and mis-parses the tail as a timecode.
///
/// The disambiguator: a `;` only terminates a statement when the next
/// non-whitespace character is `[` (the start of the next statement) or the
/// input ends. Every other `;` is literal text.
fn statements(text: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let mut line = 1usize;
    let mut rest = text;
    let mut search_from = 0usize;

    while let Some(rel) = rest[search_from..].find(';') {
        let end = search_from + rel;
        let tail = &rest[end + 1..];
        // `;` also terminates when an empty statement follows: `05-KI-OP1` has a
        // stray ` ;` between two real statements, and treating the preceding
        // `;` as literal would swallow it into the previous statement's last
        // field.
        let next = tail.trim_start_matches(['\r', '\n', ' ', '\t']);
        let terminates = next.starts_with('[') || next.starts_with(';') || next.is_empty();
        if !terminates {
            // Literal semicolon inside a text field; keep looking.
            search_from = end + 1;
            continue;
        }

        let stmt = &rest[..end];
        line += stmt.matches('\n').count();
        let trimmed = stmt.trim_start_matches(['\r', '\n', ' ', '\t', '\u{feff}']);
        if !trimmed.is_empty() {
            out.push((line, trimmed));
        }
        rest = tail;
        search_from = 0;
    }
    out
}

/// Splits a statement body into fields.
///
/// Fields are tab-separated, except in `05-KI-OP1`, which a scripter wrote with
/// `, ` separators. That script ships in the retail game and plays, so accept
/// both: fall back to commas only when there is no tab at all, so commas inside
/// ordinary dialogue stay untouched.
fn split_fields(body: &str) -> Vec<&str> {
    if body.contains('\t') {
        body.split('\t').map(str::trim_end).collect()
    } else {
        body.split(',').map(str::trim).collect()
    }
}

/// `[Command]=body` -> `("Command", "body")`.
fn split_statement(stmt: &str) -> Option<(&str, &str)> {
    let rest = stmt.strip_prefix('[')?;
    let close = rest.find("]=")?;
    Some((&rest[..close], &rest[close + 2..]))
}

/// Parses one statement, or `None` for one the engine itself drops.
///
/// A statement the original refuses is not a broken script. `FUN_0042d8c0`
/// checks each command's field count before it reads any of them and, when it
/// does not match, bumps a counter and moves to the next statement — so a
/// malformed statement costs its own line and nothing else. Refusing the whole
/// script over one would make four Shiny Days scenes unplayable over four
/// scripter typos.
fn parse_command(line: usize, command: &str, args: &[&str]) -> Result<Option<Event>, Error> {
    let arity = |want: &'static str| Error::Arity {
        line,
        command: command.to_string(),
        want,
        got: args.len(),
    };

    // Single-timecode forms.
    if matches!(command, "SkipFRAME" | "Next") {
        if args.len() != 1 {
            return Err(arity("1"));
        }
        let at = Frame::parse(args[0])?;
        // Both are markers rather than things that happen over a window, and
        // both sit at a point: giving either a zero start would sort it to the
        // front of the event list and make it look like a statement that runs
        // from the beginning.
        return Ok(Some(Event {
            start: at,
            end: at,
            command: if command == "SkipFRAME" {
                Command::SkipFrame
            } else {
                Command::Next
            },
        }));
    }

    // `[PrintText]` is the one command the original sizes exactly rather than
    // by a minimum: `FUN_0042d8c0` takes its field count, compares it to four —
    // start, speaker, text, end — and skips the statement when it differs,
    // before reading any field. The check belongs here for the same reason,
    // ahead of the timecodes: two of the four Shiny Days statements it rescues
    // would otherwise fail converting a field that is not a timecode at all.
    //
    // The comparison here is `< 4`, not `!= 4`, and the difference is a claim
    // we cannot make. The original counts fields in **its** tokenisation, and
    // whether that keeps an empty one is **not recovered**; ours keeps them, so
    // `03-KB-D10` line 29, which has a stray empty field after the dialogue,
    // reaches us as five. Under `< 4` the two readings cannot disagree on
    // anything in either install: every statement they would judge differently
    // has more than four fields, and every malformed one has fewer.
    //
    // The four that have fewer are all Shiny Days. `03-32-A29` and `Z2-21-A18`
    // each have a line whose speaker and text were typed into one field with a
    // comma between them, `03-3K-G34` one whose end timecode is stuck to the
    // end of the text with no tab, and `04-K2-A00` one written with spaces
    // throughout. All four are dropped and the scenes play without that line.
    if command == "PrintText" && args.len() < 4 {
        log::warn!(
            "line {line}: [PrintText] has {} fields, fewer than 4; dropping the statement",
            args.len()
        );
        return Ok(None);
    }

    if args.len() < 2 {
        return Err(arity("at least 2"));
    }
    let start = Frame::parse(args[0])?;
    let end = Frame::parse(args[args.len() - 1])?;
    let mid = &args[1..args.len() - 1];

    // Fields are read by position with a default, rather than matched against an
    // exact shape. Retail scripts are inconsistent about optional fields: some
    // `PlayVoice` statements leave the male-voice flag and speaker tag empty but
    // still write the tabs (`05-SE-C08` line 81), and one `PrintText` has a
    // trailing empty field (`03-KB-D10` line 29). Only the fields a command
    // genuinely needs are required.
    let field = |i: usize| mid.get(i).copied().unwrap_or_default();
    let need = |n: usize, want: &'static str| -> Result<(), Error> {
        if mid.len() < n {
            Err(Error::Arity {
                line,
                command: command.to_string(),
                want,
                got: args.len(),
            })
        } else {
            Ok(())
        }
    };

    let flag = |v: &str| !matches!(v.trim(), "" | "0");

    let command = match command {
        "PrintText" => Command::PrintText {
            speaker: field(0).to_string(),
            text: field(1).to_string(),
        },
        // Shiny Days' ambient bed. Recognised so it is not reported as a gap in
        // our vocabulary, and dropped because acting on it would be a guess:
        // the arm runs behind a gate — host vtable slot `+0x130`, or the film
        // object's `+0x320`. `+0x130` is the engine's state word, engine
        // `+0x254`, and 1 is the state a film plays in; **whether it is 1 while
        // a script is being parsed is not recovered**. See `docs/FORMATS.md`.
        // Until it is, a looping bed we started where the original stayed
        // silent would be worse than silence.
        "PlayES" => return Ok(None),
        "PlayVoice" => {
            need(1, "a voice path")?;
            Command::PlayVoice {
                path: field(0).to_string(),
                men_voice: flag(field(1)),
                tag: field(2).to_string(),
            }
        }
        "CreateBG" => {
            need(2, "a kind and an image path")?;
            Command::CreateBg {
                kind: field(0).to_string(),
                path: field(1).to_string(),
            }
        }
        "PlaySe" => {
            need(2, "a slot and a sound path")?;
            let slot = field(0);
            Command::PlaySe {
                slot: slot.trim().parse().map_err(|_| Error::BadValue {
                    line,
                    what: "SE slot",
                    value: slot.to_string(),
                })?,
                path: field(1).to_string(),
            }
        }
        "PlayMovie" => {
            need(1, "a movie path")?;
            Command::PlayMovie {
                path: field(0).to_string(),
                looping: flag(field(1)),
            }
        }
        "PlayBgm" | "EndBGM" | "EndRoll" => {
            need(1, "a path")?;
            let path = field(0).to_string();
            match command {
                "PlayBgm" => Command::PlayBgm { path },
                "EndBGM" => Command::EndBgm { path },
                _ => Command::EndRoll { path },
            }
        }
        "BlackFade" | "WhiteFade" => {
            need(1, "a direction")?;
            let fade = match field(0).trim() {
                "IN" => Fade::In,
                "OUT" => Fade::Out,
                other => {
                    return Err(Error::BadValue {
                        line,
                        what: "fade direction",
                        value: other.to_string(),
                    })
                }
            };
            if command == "BlackFade" {
                Command::BlackFade(fade)
            } else {
                Command::WhiteFade(fade)
            }
        }
        "SetSELECT" => {
            need(1, "at least one choice label")?;
            let b = field(1);
            Command::SetSelect {
                a: unquote(field(0)),
                b: (!b.trim().is_empty() && b.trim() != "null").then(|| unquote(b)),
            }
        }
        "MoveSom" => {
            need(1, "an intensity")?;
            let n = field(0);
            Command::MoveSom {
                intensity: n.trim().parse().map_err(|_| Error::BadValue {
                    line,
                    what: "MoveSom intensity",
                    value: n.to_string(),
                })?,
            }
        }
        other => {
            // An unknown command is a gap in our vocabulary, not bad data. Log
            // it loudly and drop the statement rather than refusing the script.
            log::warn!("line {line}: unknown command [{other}]; ignoring");
            return Ok(None);
        }
    };

    Ok(Some(Event {
        start,
        end,
        command,
    }))
}

/// Choice labels are wrapped in single quotes in the script.
fn unquote(s: &str) -> String {
    s.trim().trim_matches('\'').to_string()
}

/// Decodes script bytes as UTF-16LE if there is a BOM, otherwise UTF-8.
fn decode(bytes: &[u8]) -> Result<String, Error> {
    if let Some(rest) = bytes.strip_prefix(&[0xff, 0xfe]) {
        let units: Vec<u16> = rest
            .as_chunks::<2>()
            .0
            .iter()
            .map(|p| u16::from_le_bytes(*p))
            .collect();
        return String::from_utf16(&units).map_err(|_| Error::BadEncoding);
    }
    String::from_utf8(bytes.to_vec()).map_err(|_| Error::BadEncoding)
}

#[cfg(test)]
mod tests {

    /// The rated clock is the executable's own: `frame = ROUND(ms * fps * rate)
    /// / 1000`, so a rate of 1.0 is the plain clock and the rest are multiples
    /// of it.
    #[test]
    fn the_clock_scales_with_the_playback_rate() {
        let one_second = std::time::Duration::from_secs(1);
        assert_eq!(Frame::from_duration_at(one_second, 1.0), Frame(FPS));
        assert_eq!(Frame::from_duration_at(one_second, 2.0), Frame(FPS * 2));
        assert_eq!(Frame::from_duration_at(one_second, 4.0), Frame(FPS * 4));
        assert_eq!(Frame::from_duration_at(one_second, 12.0), Frame(FPS * 12));
        assert_eq!(Frame::from_duration_at(one_second, 24.0), Frame(FPS * 24));
    }

    /// The unrated clock has to stay exactly the 1x case, because every
    /// existing caller is that case.
    #[test]
    fn the_plain_clock_is_the_rated_clock_at_1x() {
        for ms in [0u64, 1, 41, 42, 999, 1000, 1001, 60_000, 1_234_567] {
            let d = std::time::Duration::from_millis(ms);
            assert_eq!(
                Frame::from_duration(d),
                Frame::from_duration_at(d, 1.0),
                "{ms} ms"
            );
        }
    }

    /// A frame counted at one rate and then continued at another is the sum of
    /// the two stretches, which is what re-basing the clock on a rate change
    /// buys: time already played is never re-scaled.
    #[test]
    fn rebasing_on_a_rate_change_keeps_the_frames_already_run() {
        let half = std::time::Duration::from_millis(500);
        let at_1x = Frame::from_duration_at(half, 1.0);
        let then_4x = Frame::from_duration_at(half, 4.0);
        assert_eq!(at_1x, Frame(FPS / 2));
        assert_eq!(then_4x, Frame(FPS * 2));
        assert_eq!(Frame(at_1x.0 + then_4x.0), Frame(FPS / 2 + FPS * 2));
    }
    use super::*;

    #[test]
    fn timecode_is_24fps() {
        assert_eq!(Frame::parse("00:00:00").unwrap(), Frame(0));
        assert_eq!(Frame::parse("00:00:23").unwrap(), Frame(23));
        assert_eq!(Frame::parse("00:01:00").unwrap(), Frame(24));
        assert_eq!(Frame::parse("01:00:00").unwrap(), Frame(60 * 24));
        assert_eq!(
            Frame::parse("01:35:15").unwrap(),
            Frame((60 + 35) * 24 + 15)
        );
        assert!(Frame::parse("nope").is_err());
    }

    /// `01-00-E01` ships `00:20:26` in a 24 fps field. The retail engine plays
    /// it, so we fold the overflow in rather than rejecting the script.
    #[test]
    fn tolerates_the_retail_frame_field_typo() {
        assert_eq!(Frame::parse("00:20:26").unwrap(), Frame(20 * 24 + 26));
    }

    #[test]
    fn timecode_round_trips_through_display() {
        for tc in ["00:00:00", "01:35:15", "13:59:23"] {
            assert_eq!(Frame::parse(tc).unwrap().to_string(), tc);
        }
    }

    /// The opening of 00-00-A00, verbatim.
    const OPENING: &str = "[SkipFRAME]=01:35:15;\n\n\
        [PlaySe]=00:00:00\t1\tSe00/00-00/00-00-A00/SE00-00-A00-002\t00:07:12;\n\n\
        [PlayMovie]=00:00:00\tMovie00/00-00/00-00-A00/00-00-A00-000\t0\t00:04:05;\n\n\
        [BlackFade]=00:04:05\tIN\t00:04:17;\n\n\
        [PrintText]=00:30:13\tMakoto\t......\t00:31:11;\n\n\
        [PlayVoice]=00:30:13\tVoice00/00-00/00-00-A00/00-00-A00-0080\t1\txxx\t00:31:11;\n\n\
        [Next]=01:35:15;\n";

    #[test]
    fn parses_a_real_script_opening() {
        let s = Script::parse_str("00-00-A00", OPENING).unwrap();
        assert_eq!(s.length, Frame::parse("01:35:15").unwrap());
        assert_eq!(s.events.len(), 7);

        assert!(matches!(
            &s.events.iter().find(|e| matches!(e.command, Command::PlaySe { .. })).unwrap().command,
            Command::PlaySe { slot: 1, path } if path.ends_with("SE00-00-A00-002")
        ));
        assert!(matches!(
            &s.events
                .iter()
                .find(|e| matches!(e.command, Command::PlayMovie { .. }))
                .unwrap()
                .command,
            Command::PlayMovie { looping: false, .. }
        ));
        assert!(s
            .events
            .iter()
            .any(|e| e.command == Command::BlackFade(Fade::In)));

        let text = s
            .events
            .iter()
            .find(|e| matches!(e.command, Command::PrintText { .. }))
            .unwrap();
        assert_eq!(text.start, Frame::parse("00:30:13").unwrap());
        assert_eq!(text.end, Frame::parse("00:31:11").unwrap());
        assert_eq!(
            text.command,
            Command::PrintText {
                speaker: "Makoto".into(),
                text: "......".into()
            }
        );

        let voice = s
            .events
            .iter()
            .find(|e| matches!(e.command, Command::PlayVoice { .. }))
            .unwrap();
        assert_eq!(
            voice.command,
            Command::PlayVoice {
                path: "Voice00/00-00/00-00-A00/00-00-A00-0080".into(),
                men_voice: true,
                tag: "xxx".into()
            }
        );
    }

    #[test]
    fn select_labels_are_unquoted_and_null_becomes_none() {
        let s = Script::parse_str(
            "t",
            "[SetSELECT]=01:32:16\t'Let's break up'\t'Abort the baby'\t01:37:16;\n\
             [SetSELECT]=00:43:06\t'So'\tnull\t00:48:06;\n",
        )
        .unwrap();
        assert_eq!(
            s.events[1].command,
            Command::SetSelect {
                a: "Let's break up".into(),
                b: Some("Abort the baby".into())
            }
        );
        assert_eq!(
            s.events[0].command,
            Command::SetSelect {
                a: "So".into(),
                b: None
            }
        );
    }

    #[test]
    fn windows_and_firing_ranges() {
        let s = Script::parse_str("t", OPENING).unwrap();
        let at = Frame::parse("00:30:20").unwrap();
        assert!(s
            .active_at(at)
            .any(|e| matches!(e.command, Command::PrintText { .. })));

        // Advancing across a start fires it exactly once.
        let before = Frame::parse("00:30:12").unwrap();
        let after = Frame::parse("00:30:13").unwrap();
        assert_eq!(s.events_between(before, after).count(), 2); // text + voice
        assert_eq!(s.events_between(after, after).count(), 0);
    }

    /// `05-KC-F00` line 241: a semicolon inside dialogue must not end the
    /// statement.
    #[test]
    fn semicolon_inside_dialogue_is_literal() {
        let s = Script::parse_str(
            "05-KC-F00",
            "[PrintText]=02:52:05\tMakoto\tI know; I am, too.\t02:56:09;\n\
             [Next]=02:56:09;\n",
        )
        .unwrap();
        let text = s
            .events
            .iter()
            .find(|e| matches!(e.command, Command::PrintText { .. }))
            .unwrap();
        assert_eq!(
            text.command,
            Command::PrintText {
                speaker: "Makoto".into(),
                text: "I know; I am, too.".into()
            }
        );
        assert_eq!(text.end, Frame::parse("02:56:09").unwrap());
        assert_eq!(s.events.len(), 2);
    }

    /// `03-KB-D10` line 29 has a trailing empty field after the dialogue.
    #[test]
    fn trailing_empty_fields_are_dropped() {
        let s = Script::parse_str(
            "03-KB-D10",
            "[PrintText]=00:16:19\tTaisuke\tHi.\t\t00:24:01;",
        )
        .unwrap();
        assert_eq!(
            s.events[0].command,
            Command::PrintText {
                speaker: "Taisuke".into(),
                text: "Hi.".into()
            }
        );
    }

    /// The two boundaries are different statements. `[Next]` ends the script;
    /// `[SkipFRAME]` is where the skip button jumps to, which in a script with
    /// a choice is the frame the choice is raised at. Reading the length off
    /// `[SkipFRAME]` cut every such script short at its own choice.
    #[test]
    fn a_choice_puts_the_skip_target_before_the_end() {
        let s = Script::parse_str(
            "00-00-A03",
            "[SkipFRAME]=01:02:09;\n\
             [PrintText]=00:00:00\tMakoto\tWell.\t00:02:00;\n\
             [SetSELECT]=01:02:09\t'So what?'\t'No'\t01:08:00;\n\
             [Next]=01:09:00;\n",
        )
        .unwrap();
        assert_eq!(s.length, Frame::parse("01:09:00").unwrap());
        assert_eq!(s.skip_to, Frame::parse("01:02:09").unwrap());
        assert!(s.skip_to < s.length);
    }

    /// Without a choice the two coincide, which is how the engine spells
    /// "nothing to skip to": `FUN_00425bf0`'s case 6 compares them and finishes
    /// the script when they are equal.
    #[test]
    fn no_choice_puts_the_skip_target_at_the_end() {
        let s = Script::parse_str(
            "00-00-A02",
            "[SkipFRAME]=01:35:06;\n\
             [PrintText]=00:00:00\tMakoto\tWell.\t00:02:00;\n\
             [Next]=01:35:06;\n",
        )
        .unwrap();
        assert_eq!(s.length, Frame::parse("01:35:06").unwrap());
        assert_eq!(s.skip_to, s.length);
    }

    /// A script with no `[SkipFRAME]` has nothing to skip to, which is the same
    /// thing as a target at the end.
    #[test]
    fn a_missing_skip_marker_falls_back_to_the_end() {
        let s = Script::parse_str("x", "[Next]=00:30:00;\n").unwrap();
        assert_eq!(s.length, Frame::parse("00:30:00").unwrap());
        assert_eq!(s.skip_to, s.length);
    }

    /// `05-KI-OP1` is written with `, ` separators instead of tabs, and has a
    /// stray whitespace-only statement.
    #[test]
    fn comma_separated_script_parses() {
        let s = Script::parse_str(
            "05-KI-OP1",
            "[SkipFRAME]=02:04:20;\r\n\r\n\
             [PlaySe]=00:00:00, 1, BGM/Vocal/SDV02, 02:04:20;\r\n\r\n\
             [PlayMovie]=00:00:00, System/OP/SDHQ_KOTONOHA, 0, 02:04:20;\r\n\r\n\
              ;\r\n\r\n\
             [Next]=02:04:20;\r\n",
        )
        .unwrap();
        assert_eq!(s.length, Frame::parse("02:04:20").unwrap());
        assert!(s.events.iter().any(|e| e.command
            == Command::PlaySe {
                slot: 1,
                path: "BGM/Vocal/SDV02".into()
            }));
        assert!(s.events.iter().any(|e| e.command
            == Command::PlayMovie {
                path: "System/OP/SDHQ_KOTONOHA".into(),
                looping: false
            }));
    }

    /// Commas inside dialogue must not be treated as separators when the
    /// statement is tab-delimited.
    #[test]
    fn commas_in_tab_delimited_dialogue_are_literal() {
        let s = Script::parse_str(
            "t",
            "[PrintText]=00:00:00\tSekai\tWell, well, well.\t00:00:10;",
        )
        .unwrap();
        assert_eq!(
            s.events[0].command,
            Command::PrintText {
                speaker: "Sekai".into(),
                text: "Well, well, well.".into()
            }
        );
    }

    #[test]
    fn utf16_scripts_decode() {
        let mut bytes = vec![0xff, 0xfe];
        for u in "[Next]=00:00:01;".encode_utf16() {
            bytes.extend_from_slice(&u.to_le_bytes());
        }
        let s = Script::parse("t", &bytes).unwrap();
        assert_eq!(s.events[0].command, Command::Next);
    }

    /// `FUN_0042d8c0` requires `[PrintText]` to have exactly four fields and
    /// drops the statement otherwise. These are the four shapes the Shiny Days
    /// scripts actually get that wrong in; each costs its own line and nothing
    /// else.
    #[test]
    fn a_print_text_without_four_fields_costs_only_its_own_line() {
        let script = Script::parse_str(
            "quirks",
            "[PrintText]=00:00:06\tSetsuna, That'll be 1280 yen.\t00:03:06;\n\n\
             [PrintText]=02:03:20\tKokoro\tTell my mom next time.02:11:23;\n\n\
             [PrintText]=01:08:02 Manami Her demands are simple. 01:12:05;\n\n\
             [PrintText]=00:00:00\tMakoto\tThis one is fine.\t00:02:00;\n\n\
             [Next]=00:10:00;",
        )
        .unwrap();
        let lines: Vec<&Command> = script
            .events
            .iter()
            .map(|e| &e.command)
            .filter(|c| matches!(c, Command::PrintText { .. }))
            .collect();
        assert_eq!(lines.len(), 1);
        assert!(matches!(lines[0], Command::PrintText { speaker, .. } if speaker == "Makoto"));
        assert_eq!(script.length, Frame::parse("00:10:00").unwrap());
    }

    /// Shiny Days' `[PlayES]` is recognised rather than reported as an unknown
    /// command, and carries no event, because the gate it runs behind is not
    /// recovered.
    #[test]
    fn play_es_is_recognised_and_carries_nothing() {
        let script = Script::parse_str(
            "es",
            "[PlayES]=00:00:00\tGenSe/gaya/GAYA_ekimae_asa\t00:21:05;\n\n[Next]=00:21:05;",
        )
        .unwrap();
        assert_eq!(script.events.len(), 1);
        assert!(matches!(script.events[0].command, Command::Next));
    }
}
