//! The game's own menus, composited from the art in the player's install.
//!
//! [`menu`] is the state machine — the mode integer the executable and the
//! menu DLL pass between them — and it delegates the screens with enough
//! behaviour of their own to [`options`] and [`replay`]. [`screen`] is how any
//! one screen is drawn: base art, `_CHIP` sprite sheet and hit map. [`ending`]
//! supplies the backdrop the title screen picks from what the player has seen.

pub mod ending;
pub mod menu;
pub mod options;
pub mod replay;
pub mod screen;
