//! A UI screen: base art, chip sprites, hit map, and how they compose.
//!
//! # How a screen is drawn
//!
//! `NAME.PNG` is the resting state and always covers the screen — on the title
//! that is the logo and the five menu labels in plain text. `NAME_CHIP.PNG`
//! holds *replacement* art for individual widgets: the title sheet's first row
//! is the same five labels inside a rounded button frame, which is what the
//! selected entry shows, and its second row holds a greyed-out `REPLAY` for
//! when replay is still locked. So compositing is: draw the base, then draw the
//! chip sprite for whichever widgets are currently in a non-resting state.
//!
//! # Resolution
//!
//! Nothing here is hardcoded to a resolution. The widget table in the DLL is in
//! the screen's native space, the shipped `.CMAP`s are that same layout already
//! scaled, and both the scale factor and the letterbox offset are recovered by
//! comparing the two maps:
//!
//! ```text
//! scale     = display_map.width / native_map.width
//! letterbox = (display_map.height - native_map.height * scale) / 2
//! ```
//!
//! For the title that yields 1.0/1.28/1.6 and a 75px offset in 4:3 — the game
//! is authored at 800x450 and centred between bars in 800x600 — and for the
//! menubar, whose map is a 800x75 strip, it yields the same scales and no
//! offset, with no special case for either.

use crate::install::vfs::Vfs;
use days_ui::atlas::{self, Atlas, Widget};
use days_ui::cmap::Cmap;
use days_ui::Image;

/// What can go wrong putting a screen together.
///
/// Reading and parsing the UI data is [`days_ui::Error`]; these two are about
/// the install rather than the format, so they live with the code that goes
/// looking in the packs.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Ui(#[from] days_ui::Error),
    #[error("{0} is not in the packs")]
    MissingAsset(String),
    #[error(
        "no widget table in SysMenuSDHQ.dll matches the hit map for {0}; \
         the chip sprite positions cannot be recovered"
    )]
    NoAtlasFor(String),
}

/// The four sizes the game ships UI art for, named by `.CMAP` filename suffix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    /// 800x600, 4:3. The native 800x450 layout letterboxed.
    Standard,
    /// 800x450, windowed widescreen. The native layout.
    Wide,
    /// 1024x576, "note" fullscreen.
    Note,
    /// 1280x720, wide fullscreen.
    Full,
}

impl Resolution {
    /// The `.CMAP` filename suffix, e.g. `Title_Wide_Full.cmap`.
    pub fn suffix(self) -> &'static str {
        match self {
            Resolution::Standard => "",
            Resolution::Wide => "_Wide",
            Resolution::Note => "_Wide_Note",
            Resolution::Full => "_Wide_Full",
        }
    }

    pub const ALL: [Resolution; 4] = [
        Resolution::Standard,
        Resolution::Wide,
        Resolution::Note,
        Resolution::Full,
    ];

    /// Parses a resolution from a CLI-friendly name.
    pub fn from_name(name: &str) -> Option<Resolution> {
        match name.to_ascii_lowercase().as_str() {
            "standard" | "800x600" | "4:3" => Some(Resolution::Standard),
            "wide" | "800x450" => Some(Resolution::Wide),
            "note" | "1024x576" => Some(Resolution::Note),
            "full" | "1280x720" => Some(Resolution::Full),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Resolution::Standard => "standard",
            Resolution::Wide => "wide",
            Resolution::Note => "note",
            Resolution::Full => "full",
        }
    }
}

/// What a widget is currently showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WidgetState {
    /// The base art shows through; nothing is drawn over it.
    #[default]
    Resting,
    /// The widget's own chip sprite: hovered, or keyboard-selected.
    Active,
    /// One of the trailing alternate-state records, by index into
    /// [`Atlas::extras`] — a disabled or alternate-caption sprite.
    Extra(usize),
}

/// A loaded screen at one resolution.
pub struct Screen {
    /// Logical path stem, e.g. `System/Title/Title`.
    pub path: String,
    pub resolution: Resolution,
    base: Image,
    chip: Image,
    /// Hit map at `resolution`.
    display: Cmap,
    /// Scale from native layout space to `resolution`.
    scale: f64,
    /// Vertical offset in display pixels, non-zero only when letterboxed.
    letterbox: f64,
    atlas: Atlas,
}

/// Reads `path` from the VFS case-insensitively, accepting the mixed casing the
/// `.INI` and DLL path strings use against the packs' upper-case entry names.
fn read(vfs: &Vfs, path: &str) -> Result<Vec<u8>, Error> {
    vfs.read_path(path)
        .map_err(|_| Error::MissingAsset(path.to_string()))
}

impl Screen {
    /// Loads a screen.
    ///
    /// `path` is the stem shared by the three files, as the DLL spells it, e.g.
    /// `System/Title/Title`. `dll` is the bytes of the user's own
    /// `SysMenuSDHQ.dll`, which is where the widget-to-sprite table lives.
    pub fn load(
        vfs: &Vfs,
        dll: &[u8],
        path: &str,
        resolution: Resolution,
    ) -> Result<Screen, Error> {
        Screen::load_with_base(vfs, dll, path, None, resolution)
    }

    /// Loads a screen whose base art is not named after the stem.
    ///
    /// Most screens are `NAME.PNG` / `NAME_CHIP.PNG` / `NAME*.CMAP`, but a few
    /// share one chip sheet and hit map across several backgrounds and pick the
    /// background by context: `Exit/Popup` draws either `Popup_Exit.png` or
    /// `Popup_Title.png` depending on where it was opened from, and
    /// `SaveLoad/SaveLoad` draws `Save.png` or `Load.png`. For those, `base` is
    /// the logical path of the background to use.
    pub fn load_with_base(
        vfs: &Vfs,
        dll: &[u8],
        path: &str,
        base: Option<&str>,
        resolution: Resolution,
    ) -> Result<Screen, Error> {
        let base_path = base
            .map(str::to_string)
            .unwrap_or_else(|| format!("{path}.png"));
        let base = Image::decode_png(&read(vfs, &base_path)?)?;
        let chip = Image::decode_png(&read(vfs, &format!("{path}_Chip.png"))?)?;

        // The native map is the one the DLL's table is expressed in: the
        // widescreen variant where there is one, otherwise the only map there
        // is. `MENUBAR` ships just `MenuBar.cmap` and is already native.
        let native_path = format!("{path}_Wide.cmap");
        let native = match vfs.read_path(&native_path) {
            Ok(bytes) => Cmap::parse(&bytes)?,
            Err(_) => Cmap::parse(&read(vfs, &format!("{path}.cmap"))?)?,
        };

        let display_path = format!("{path}{}.cmap", resolution.suffix());
        let display = match vfs.read_path(&display_path) {
            Ok(bytes) => Cmap::parse(&bytes)?,
            Err(_) => {
                log::warn!("{display_path} is absent; using the native map for hit testing");
                Cmap::parse(&read(vfs, &format!("{path}.cmap"))?)?
            }
        };

        let scale = f64::from(display.width()) / f64::from(native.width());
        let letterbox = (f64::from(display.height()) - f64::from(native.height()) * scale) / 2.0;

        let atlas = atlas::find(dll, native.all_bounds(), (chip.width, chip.height))
            .map_err(|_| Error::NoAtlasFor(path.to_string()))?;

        Ok(Screen {
            path: path.to_string(),
            resolution,
            base,
            chip,
            display,
            scale,
            letterbox,
            atlas,
        })
    }

    pub fn size(&self) -> (u32, u32) {
        (self.display.width(), self.display.height())
    }

    pub fn scale(&self) -> f64 {
        self.scale
    }

    pub fn letterbox(&self) -> f64 {
        self.letterbox
    }

    pub fn atlas(&self) -> &Atlas {
        &self.atlas
    }

    /// The `_CHIP` sheet, for a screen that draws a sprite of its own from it.
    pub fn chip(&self) -> &Image {
        &self.chip
    }

    /// Number of widgets, i.e. of hit regions.
    pub fn widget_count(&self) -> usize {
        self.atlas.widgets.len()
    }

    /// The widget under a display-space point, as a 0-based index.
    ///
    /// This is a hit-map lookup, not a rectangle test — widgets on the route
    /// map screens are not rectangular, and the bounding boxes the atlas is
    /// keyed by would claim pixels that belong to nothing.
    pub fn hit(&self, x: u32, y: u32) -> Option<usize> {
        let id = self.display.region_at(x, y);
        if id == 0 {
            return None;
        }
        let index = id as usize - 1;
        (index < self.atlas.widgets.len()).then_some(index)
    }

    /// Maps a native-space rectangle into display space.
    fn place(&self, w: &Widget) -> (i64, i64, u32, u32) {
        let round = |v: f64| v.round();
        (
            round(f64::from(w.dst.x) * self.scale) as i64,
            round(f64::from(w.dst.y) * self.scale + self.letterbox) as i64,
            round(f64::from(w.dst.width) * self.scale).max(1.0) as u32,
            round(f64::from(w.dst.height) * self.scale).max(1.0) as u32,
        )
    }

    /// Draws a whole image that is authored in native layout space, applying
    /// the same scale and letterbox the widgets get.
    fn blit_native(&self, out: &mut Image, img: &Image) {
        let w = (f64::from(img.width) * self.scale).round().max(1.0) as u32;
        let h = (f64::from(img.height) * self.scale).round().max(1.0) as u32;
        out.blit_scaled(
            img,
            (0, 0, img.width, img.height),
            (0, self.letterbox.round() as i64, w, h),
        );
    }

    /// Composites the screen. `states` is indexed by widget; a shorter slice
    /// leaves the rest resting.
    pub fn compose(&self, states: &[WidgetState]) -> Image {
        self.compose_over(None, states)
    }

    /// Composites the screen as a **transparent layer**, for one that sits over
    /// playback rather than replacing it.
    ///
    /// The full-screen menus own their background and are composited onto black.
    /// The in-game control bar does not: `MENUBAR.PNG` is RGBA, and the engine
    /// draws the strip's sprites over whatever frame is underneath. Flattening
    /// it onto black first would fill the transparent part of the strip with a
    /// black bar, which is not what the original shows.
    pub fn compose_layer(&self, states: &[WidgetState]) -> Image {
        let (w, h) = self.size();
        let mut out = Image::empty(w, h);
        self.blit_native(&mut out, &self.base);
        self.draw_states(&mut out, states);
        out
    }

    /// Composites the screen over a backdrop, with extra sprites on top.
    ///
    /// Two things a screen draws are not widget states and so cannot be
    /// expressed as one: the Sound tab's three volume bars, which are one
    /// record stretched to the width of however many level cells are filled,
    /// and the replay grid's thumbnails, which come from a second sheet
    /// entirely. Both are sprites the screen's own module builds at draw time,
    /// so they arrive here already worked out, each paired with the sheet it is
    /// cut from.
    pub fn compose_over_sprites(
        &self,
        backdrop: Option<&Image>,
        states: &[WidgetState],
        sprites: &[(&Image, Widget)],
    ) -> Image {
        let mut out = self.compose_over(backdrop, states);
        for (sheet, widget) in sprites {
            out.blit_scaled(
                sheet,
                (
                    widget.src_x,
                    widget.src_y,
                    widget.dst.width,
                    widget.dst.height,
                ),
                self.place(widget),
            );
        }
        out
    }

    /// Composites the screen over a backdrop.
    ///
    /// Some screens do not own their background. The title's `Title.png` is
    /// transparent around the logo and menu, and the picture behind it is
    /// `STARTSCRIPT.INI`'s `[BaseFile]` — drawn by the engine, not by the menu
    /// module. The backdrop is placed in native space like everything else, so
    /// it letterboxes with the rest.
    pub fn compose_over(&self, backdrop: Option<&Image>, states: &[WidgetState]) -> Image {
        let (w, h) = self.size();
        let mut out = Image::black(w, h);

        if let Some(under) = backdrop {
            self.blit_native(&mut out, under);
        }

        self.blit_native(&mut out, &self.base);
        self.draw_states(&mut out, states);
        out
    }

    /// Draws the non-resting widget sprites onto an already-started frame.
    fn draw_states(&self, out: &mut Image, states: &[WidgetState]) {
        for (i, state) in states.iter().enumerate() {
            let widget = match state {
                WidgetState::Resting => continue,
                WidgetState::Active => self.atlas.widgets.get(i),
                WidgetState::Extra(n) => self.atlas.extras.get(*n),
            };
            let Some(widget) = widget else {
                log::warn!(
                    "{}: no chip sprite for widget {i} in state {state:?}",
                    self.path
                );
                continue;
            };
            out.blit_scaled(
                &self.chip,
                (
                    widget.src_x,
                    widget.src_y,
                    widget.dst.width,
                    widget.dst.height,
                ),
                self.place(widget),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolution_names_round_trip() {
        for r in Resolution::ALL {
            assert_eq!(Resolution::from_name(r.name()), Some(r));
        }
        assert_eq!(Resolution::from_name("1280x720"), Some(Resolution::Full));
        assert_eq!(Resolution::from_name("nonsense"), None);
    }

    #[test]
    fn the_suffix_of_the_native_size_is_the_wide_one() {
        assert_eq!(Resolution::Wide.suffix(), "_Wide");
        assert_eq!(Resolution::Standard.suffix(), "");
    }
}
