//! The game's own user interface.
//!
//! FILMEngine's UI is data-driven and every byte of it lives in the user's own
//! install: base art and sprite sheets in `System.GPK`, hit maps beside them,
//! and the table saying which sprite belongs to which widget inside
//! `SysMenuSDHQ.dll`. Nothing here is reproduced or approximated — this crate
//! reads those files and composites them.
//!
//! - [`cmap`] parses the per-pixel hit maps.
//! - [`atlas`] recovers the widget-to-sprite table out of the DLL, by content
//!   rather than by a hardcoded address. Start there: it is the part that was
//!   not derivable from the art.
//! - [`screen`] puts the three together and composites a frame.

#![forbid(unsafe_code)]

pub mod atlas;
pub mod cmap;
pub mod image;
pub mod screen;

pub use atlas::{Atlas, Widget};
pub use cmap::{Cmap, Rect};
pub use image::Image;
pub use screen::{Resolution, Screen, WidgetState};

/// Everything that can go wrong loading a screen.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("the .cmap is shorter than its own dimensions declare")]
    TruncatedCmap,
    #[error("decoding PNG: {0}")]
    Png(#[from] png::DecodingError),
    #[error("palette-indexed PNGs are not used by the game's UI art")]
    UnsupportedPng,
    #[error("{0} is not in the packs")]
    MissingAsset(String),
    #[error("no widget table in the DLL matches this screen's hit map")]
    NoAtlas,
    #[error(
        "no widget table in SysMenuSDHQ.dll matches the hit map for {0}; \
         the chip sprite positions cannot be recovered"
    )]
    NoAtlasFor(String),
}
