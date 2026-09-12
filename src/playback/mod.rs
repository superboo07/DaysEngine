//! Playing a script: what is on screen and what is audible at a given frame.
//!
//! `.ORS` files are timelines, so this is a scheduler and not an interpreter.
//! [`stage`] walks one clock across the whole file and reports the state at a
//! frame; [`mixer`] sums the audio that state says is playing, [`lipsync`]
//! flaps a speaker's mouth for as long as their clip is audible, [`text`] lays
//! dialogue out in the game's own font, and [`compose`] draws the lot on the
//! CPU so a frame can be checked without a display.

pub mod compose;
pub mod lipsync;
pub mod mixer;
pub mod stage;
pub mod text;
