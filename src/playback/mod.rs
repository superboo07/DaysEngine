//! Playing a script: what is on screen and what is audible at a given frame.
//!
//! `.ORS` files are timelines, so this is a scheduler and not an interpreter.
//! [`stage`] walks one clock across the whole file and reports the state at a
//! frame; [`mixer`] sums the audio that state says is playing, [`lipsync`]
//! flaps a speaker's mouth for as long as their clip is audible, [`text`] lays
//! dialogue out in the game's own font, [`compose`] draws the lot on the CPU so
//! a frame can be checked without a display, [`som`] is the levels a script
//! asks a peripheral for, [`prefetch`] gets the statements just ahead of the
//! clock ready on a thread of their own, and [`profile`] logs where a movie's
//! time went.

pub mod compose;
pub mod lipsync;
pub mod mixer;
pub mod prefetch;
pub mod profile;
pub mod scale;
pub mod som;
pub mod stage;
pub mod text;
