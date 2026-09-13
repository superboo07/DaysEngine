//! `[MoveSom]` — the levels a script asks a peripheral for.
//!
//! SOMCON is the device the game was built to drive: a toy on a COM port,
//! spoken to in two ASCII commands. `s%02x` sets a level and `b` stops — see
//! `docs/FORMATS.md` for the port protocol, which is the menu DLL's and is not
//! this module's business. What is this module's business is the timeline
//! side: which level is asked for, when, and under what conditions the engine
//! asks for it at all.
//!
//! The device is discontinued and its protocol is proprietary to it. What is
//! left is the levels, and a level is a level: a rumble motor takes the same
//! number. So this engine sends them to a controller. The scripts are
//! unchanged, the Option screen's switch is unchanged, and the recovered rule
//! below is followed exactly — only the thing on the other end is one a player
//! can still buy.
//!
//! # The statement
//!
//! ```text
//! [MoveSom]=00:04:11 <tab> 5 <tab> 00:06:18;
//! ```
//!
//! Start frame, intensity, end frame. `FUN_0043dbe0` is the per-tick statement
//! walk that dispatches it, and it does four things in order:
//!
//! 1. `_GetSomFlag@0()` — the DLL's `+0x31c`, which is set only once a port
//!    has actually been opened. With no device the statement does nothing at
//!    all, and nothing else about the tick changes.
//! 2. Host `+0x114` must be 1. That slot is `FUN_0042bfb0`, which returns the
//!    engine's `+0x1f4` — its state number — and 1 is the playback tick. So a
//!    menu open over the script (state 3), a skip (4) or leaving (5) all stop
//!    the device, which is [`gated`]'s first half.
//! 3. Host `+0x11c` must be 0. That slot is `FUN_0042c060`, returning the
//!    engine's `+0x504`: the lit speed widget. **So the device only moves at
//!    1x.** Fast-forwarding is silent to it, which is the other half.
//! 4. The intensity is parsed, must be above zero, and goes through
//!    [`level`] before `FUN_0042a200` stores it and calls `_SomMove@4`.
//!
//! `FUN_0042a200` keeps three members — the end frame at `+0x790`, an active
//! flag at `+0x794` and the level at `+0x798` — and `FUN_0042a250`, called
//! from the playback tick `FUN_00424020`, sends `_SomStop@0` once the frame
//! reaches the end. Suspending playback stops it too and resuming puts the
//! same level back: `FUN_00424910` calls `_SomStop@0` while the flag is set
//! and `FUN_00424a10` calls `_SomMove@4(+0x798)`. That is exactly a statement
//! that is live across its own `[start, end)` window and nowhere else, which
//! is how every other statement in a `.ORS` behaves and how this one is
//! scheduled here.

/// Something a level can be sent to.
///
/// The five operations are the ones the menu DLL has, named for what they do
/// rather than for how it does them: `FUN_10007850` is [`Device::detect`],
/// trying each port in turn and keeping the first that answers;
/// `FUN_10021070` is [`Device::open`]; `s%02x` is [`Device::set_level`] and
/// `b` is [`Device::stop`]. A backend that is not a COM port answers the same
/// five questions its own way.
///
/// The engine holds one of these and knows nothing else about it, which is the
/// point: `daysengine` drives a controller's rumble motor through it today,
/// and a backend that speaks to [Intiface](https://intiface.com/) — where the
/// toys this game was actually written for still live — is another
/// implementation of this trait and no change anywhere else. The Option
/// screen's SOMCON tab is already the right screen for either: its ten `Port
/// number` buttons are a device list, and [`Device::ports`] is what fills it.
pub trait Device {
    /// What a player can pick from, in the order the tab's `Port number`
    /// buttons stand for. At most [`crate::ui::options::SOM_PORTS`] of them
    /// are reachable.
    fn ports(&self) -> Vec<String>;

    /// Takes the port at `port`, reporting whether it answered.
    fn open(&mut self, port: usize) -> bool;

    /// Tries every port in turn and keeps the first that answers, which is
    /// what the tab's find button does.
    fn detect(&mut self) -> Option<usize>;

    /// Lets go of whatever port is held.
    fn close(&mut self);

    /// Sets the level, 0 to 255. Zero and [`Device::stop`] are the same thing
    /// to a motor, but not to the original's protocol — taking a port sends
    /// `s00` and the stop command is its own — so both are here.
    fn set_level(&mut self, level: u8);

    /// Stops the device where it is.
    fn stop(&mut self);
}

/// The level a script intensity asks for, from `FUN_00438470`.
///
/// A five-case switch and nothing else: `1` to `5` map to a fifth of full
/// scale apiece, and **every other value maps to zero** — the default case
/// falls through to the `local_8 = 0` the function opens with. Retail scripts
/// use 1 to 5 and nothing else; 46 of the 1857 scripts carry a `[MoveSom]` at
/// all.
///
/// Zero is not "no statement". `FUN_0043dbe0` refuses an intensity that parses
/// to zero or less *before* it gets here, so a level of zero out of this is an
/// intensity of 6 or more — a statement that runs its window and asks for
/// nothing.
pub fn level(intensity: i32) -> u8 {
    match intensity {
        1 => 0x33,
        2 => 0x66,
        3 => 0x99,
        4 => 0xcc,
        5 => 0xff,
        _ => 0,
    }
}

/// The level the Option screen's `SOMCON test` sends, from `FUN_100214d0`'s
/// caller: `s96`.
///
/// Not one of [`level`]'s five. The test button is the DLL's own, and it asks
/// for rather more than half.
pub const TEST_LEVEL: u8 = 0x96;

/// Whether the engine is in a state that drives the device at all.
///
/// Both halves of `FUN_0043dbe0`'s pair of host questions: `speed` is the lit
/// speed widget's index, which must be 0 (1x), and `playing` is whether the
/// engine is in state 1 rather than in a menu, a skip or on its way out.
pub fn gated(playing: bool, speed: usize) -> bool {
    playing && speed == 0
}

/// The level actually sent, once the engine's own settings have had their say.
///
/// `strength` is `DaysEngine.ini`'s `[Rumble] Strength`, a percentage of what
/// the script asked for — this engine's knob, not the game's, and the only
/// thing here that is not recovered. 100 leaves the level alone.
pub fn scaled(level: u8, strength: u16) -> u16 {
    let of_full = u32::from(level) * 257;
    let scaled = of_full * u32::from(strength) / 100;
    scaled.min(u32::from(u16::MAX)) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `FUN_00438470`'s whole switch, including the default nobody reaches
    /// from retail data — an intensity outside 1..=5 asks for nothing rather
    /// than for something sensible.
    #[test]
    fn the_five_intensities_are_a_fifth_of_full_scale_apiece() {
        assert_eq!(
            (1..=5).map(level).collect::<Vec<_>>(),
            [0x33, 0x66, 0x99, 0xcc, 0xff]
        );
        assert_eq!(level(0), 0);
        assert_eq!(level(6), 0);
        assert_eq!(level(-1), 0);
    }

    /// The device is silent above 1x, and silent while the engine is not on
    /// its playback tick. Both are host questions the statement asks before it
    /// does anything.
    #[test]
    fn the_device_only_moves_at_normal_speed_while_playing() {
        assert!(gated(true, 0));
        assert!(!gated(true, 1));
        assert!(!gated(false, 0));
    }

    /// Full scale reaches the motor's full scale, and the engine's own
    /// percentage cannot push it past it.
    #[test]
    fn full_scale_arrives_as_full_scale_and_cannot_overflow() {
        assert_eq!(scaled(0xff, 100), u16::MAX);
        assert_eq!(scaled(0, 100), 0);
        assert_eq!(scaled(0xff, 0), 0);
        assert_eq!(scaled(0xff, 400), u16::MAX);
        assert_eq!(scaled(0x33, 100), 0x33 * 257);
    }
}
