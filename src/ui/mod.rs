//! The game's own menus, composited from the art in the player's install.
//!
//! [`menu`] is the state machine — the mode integer the executable and the
//! menu DLL pass between them — and it delegates the screens with enough
//! behaviour of their own to [`options`] and [`replay`]. [`screen`] is how any
//! one screen is drawn: base art, `_CHIP` sprite sheet and hit map. [`ending`]
//! supplies the backdrop the title screen picks from what the player has seen.
//!
//! Two of these are not menu modes at all but UI the engine puts over playback:
//! [`bar`] is the control strip, which the executable drives through the DLL's
//! `_SetMenuBar@4` object rather than through `SystemInit`, and [`select`] is
//! the choice box, which the executable owns outright and which ships no art
//! beyond its hit maps.

pub mod bar;
pub mod ending;
pub mod menu;
pub mod options;
pub mod replay;
pub mod screen;
pub mod select;
