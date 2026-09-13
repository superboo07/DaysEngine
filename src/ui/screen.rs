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
use crate::playback::scale::Scaler;
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

    /// The art set the game would load for a given display mode.
    ///
    /// `FUN_10013470` is the rule, and every screen's loader repeats it:
    ///
    /// ```text
    /// if (host->+0xb8() == 1)                  // wide
    ///     if (host->+0xbc() == 1)              // full screen
    ///         host->+0xc8() ? "_Wide_Note" : "_Wide_Full"
    ///     else "_Wide"
    /// else ""
    /// ```
    ///
    /// `+0xb8` is the engine's aspect, `+0xbc` is `FUN_0040e830`'s full-screen
    /// member, and `+0xc8` returns `DAT_0050b314`, which `FUN_0040cbb0` reads
    /// out of the player's own `Config.DAT` key **`TypeMiniNote`**. The two
    /// full-screen sizes are `DX9GRAPHIC.INI`'s `[FullWideWidth]`/`[Height]`
    /// (1280x720) and `[FullNoteWidth]`/`[Height]` (1024x576), and the 4:3 one
    /// is `[DisplayWidthSize]`/`[Height]` (800x600), which is where these four
    /// sizes come from.
    pub fn for_display(wide: bool, full_screen: bool, mini_note: bool) -> Resolution {
        match (wide, full_screen, mini_note) {
            (false, _, _) => Resolution::Standard,
            (true, false, _) => Resolution::Wide,
            (true, true, true) => Resolution::Note,
            (true, true, false) => Resolution::Full,
        }
    }

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
    /// Base art, already resampled into display space.
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
        let native_base = Image::decode_png(&read(vfs, &base_path)?)?;
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

        // The base art covers the whole screen, so resampling it is the most
        // expensive thing a composite does — and it is the same work every
        // time, because only the sprites over it change. Done once, here.
        let size = scaled_size(&native_base, scale);
        let base = resampled(
            &native_base,
            (0, 0, native_base.width, native_base.height),
            size,
        )
        .unwrap_or(native_base);

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
    /// Places a rectangle given in 800x450 layout space, the way a widget is
    /// placed.
    ///
    /// Widget records are whole pixels, but some screens offset a sprite inside
    /// its record by half a pixel — the save/load rows sit 4.5 down their own
    /// record — so this takes the rect as floats and rounds where [`Screen`]
    /// rounds: after the scale, not before it.
    pub fn place_layout(&self, rect: (f32, f32, f32, f32)) -> (i64, i64, u32, u32) {
        let (x, y, w, h) = rect;
        (
            (f64::from(x) * self.scale).round() as i64,
            (f64::from(y) * self.scale + self.letterbox).round() as i64,
            (f64::from(w) * self.scale).round().max(1.0) as u32,
            (f64::from(h) * self.scale).round().max(1.0) as u32,
        )
    }

    fn place(&self, w: &Widget) -> (i64, i64, u32, u32) {
        let round = |v: f64| v.round();
        (
            round(f64::from(w.dst.x) * self.scale) as i64,
            round(f64::from(w.dst.y) * self.scale + self.letterbox) as i64,
            round(f64::from(w.dst.width) * self.scale).max(1.0) as u32,
            round(f64::from(w.dst.height) * self.scale).max(1.0) as u32,
        )
    }

    /// Resamples a whole image that is authored in native layout space into
    /// this screen's display space. See [`resampled`].
    pub fn to_display(&self, img: &Image) -> Image {
        let size = scaled_size(img, self.scale);
        resampled(img, (0, 0, img.width, img.height), size).unwrap_or_else(|| img.clone())
    }

    /// Draws an image that is already in display space, at the letterbox offset
    /// the widgets get.
    fn blit_display(&self, out: &mut Image, img: &Image) {
        out.blit_scaled(
            img,
            (0, 0, img.width, img.height),
            (0, self.letterbox.round() as i64, img.width, img.height),
        );
    }

    /// Draws one widget's chip sprite, resampled to the size the display map
    /// gives it.
    fn blit_sprite(&self, out: &mut Image, sheet: &Image, widget: &Widget) {
        let src = (
            widget.src_x,
            widget.src_y,
            widget.dst.width,
            widget.dst.height,
        );
        let dst = self.place(widget);
        match resampled(sheet, src, (dst.2, dst.3)) {
            Some(scaled) => out.blit_scaled(&scaled, (0, 0, dst.2, dst.3), dst),
            None => out.blit_scaled(sheet, src, dst),
        }
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
        self.blit_display(&mut out, &self.base);
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
            self.blit_sprite(&mut out, sheet, widget);
        }
        out
    }

    /// Composites the screen over a backdrop.
    ///
    /// Some screens do not own their background. The title's `Title.png` is
    /// transparent around the logo and menu, and the picture behind it is
    /// `STARTSCRIPT.INI`'s `[BaseFile]` — drawn by the engine, not by the menu
    /// module. It still letterboxes with the rest, so it is placed at the same
    /// offset the widgets get.
    ///
    /// `backdrop` is **already in display space**: run it through
    /// [`Screen::to_display`] first. A caller that draws the same backdrop into
    /// frame after frame would otherwise pay for the biggest resample on the
    /// screen every time, and the answer never changes.
    pub fn compose_over(&self, backdrop: Option<&Image>, states: &[WidgetState]) -> Image {
        let (w, h) = self.size();
        let mut out = Image::black(w, h);

        if let Some(under) = backdrop {
            self.blit_display(&mut out, under);
        }

        self.blit_display(&mut out, &self.base);
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
            self.blit_sprite(out, &self.chip, widget);
        }
    }
}

/// The size a native-space image takes up in display space.
fn scaled_size(img: &Image, scale: f64) -> (u32, u32) {
    (
        (f64::from(img.width) * scale).round().max(1.0) as u32,
        (f64::from(img.height) * scale).round().max(1.0) as u32,
    )
}

/// Resamples `rect` of `src` to `size`, or `None` when it is already that size.
///
/// UI art is authored once, in the native 800x450 layout, and the larger
/// display maps are that art scaled up — 1.28x for the 1024x576 set and 1.6x
/// for 1280x720. `FUN_0044a3d0` is what the original does about that: it sets
/// `D3DSAMP_MAGFILTER` and `D3DSAMP_MINFILTER` to `D3DTEXF_LINEAR` on all eight
/// sampler stages, with `D3DSAMP_ADDRESSU`/`V` clamped, so every sprite the
/// menus draw is filtered by the GPU on the way up. Scaling it by picking the
/// nearest source pixel instead steps the diagonals and hardens the text, which
/// is not what the game looks like. This runs the same cubic B-spline the
/// picture goes through, whose edge clamp is the `ADDRESSU`/`V` above.
///
/// The work is done in premultiplied alpha. `_CHIP` sheets and the transparent
/// parts of a screen's base art are RGBA, and a fully transparent pixel's
/// colour channels are arbitrary — averaging them unpremultiplied would pull
/// that arbitrary colour into the edge of every glyph.
fn resampled(src: &Image, rect: (u32, u32, u32, u32), size: (u32, u32)) -> Option<Image> {
    let (sx, sy, sw, sh) = rect;
    if (sw, sh) == size || sw == 0 || sh == 0 || size.0 == 0 || size.1 == 0 {
        return None;
    }
    let mut cut = vec![0u8; sw as usize * sh as usize * 4];
    for (row, line) in cut.chunks_exact_mut(sw as usize * 4).enumerate() {
        for (col, out) in line.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let Some(p) = src.pixel(sx + col as u32, sy + row as u32) else {
                continue;
            };
            let a = u32::from(p[3]);
            for (o, c) in out[..3].iter_mut().zip(p) {
                *o = (u32::from(c) * a / 255) as u8;
            }
            out[3] = p[3];
        }
    }
    let src_size = (sw as usize, sh as usize);
    let dst_size = (size.0 as usize, size.1 as usize);
    let mut scaler = Scaler::new(src_size, dst_size);
    let mut rgba = scaler.resample(&cut, src_size, dst_size)?.to_vec();
    for px in rgba.as_chunks_mut::<4>().0 {
        let a = u32::from(px[3]);
        if a == 0 {
            continue;
        }
        for c in px[..3].iter_mut() {
            *c = (u32::from(*c) * 255 / a).min(255) as u8;
        }
    }
    Some(Image {
        width: size.0,
        height: size.1,
        rgba,
    })
}

#[cfg(test)]
mod tests {

    /// `FUN_10013470`'s three-way test, which every screen's loader repeats.
    /// 4:3 has one art set whatever else is true; widescreen splits by window
    /// versus full screen, and full screen splits again on `TypeMiniNote`.
    #[test]
    fn the_art_set_follows_the_display_mode() {
        for mini in [false, true] {
            assert_eq!(
                Resolution::for_display(false, false, mini),
                Resolution::Standard
            );
            assert_eq!(
                Resolution::for_display(false, true, mini),
                Resolution::Standard,
                "4:3 has no full-screen art of its own"
            );
            assert_eq!(Resolution::for_display(true, false, mini), Resolution::Wide);
        }
        assert_eq!(
            Resolution::for_display(true, true, false),
            Resolution::Full,
            "1280x720 without TypeMiniNote"
        );
        assert_eq!(
            Resolution::for_display(true, true, true),
            Resolution::Note,
            "1024x576 with it"
        );
    }

    /// Art scaled up to a display size is filtered, not point-sampled: the
    /// original sets `D3DTEXF_LINEAR` on every sampler stage in `FUN_0044a3d0`,
    /// and taking the nearest source pixel instead steps every diagonal and
    /// hardens every glyph. A pixel between two source pixels has to land
    /// between their two values.
    #[test]
    fn scaling_art_up_filters_it() {
        let mut src = Image::black(2, 1);
        src.rgba
            .copy_from_slice(&[0, 0, 0, 255, 255, 255, 255, 255]);
        let out = resampled(&src, (0, 0, 2, 1), (8, 1)).expect("should scale");
        let greys: Vec<u8> = out.rgba.as_chunks::<4>().0.iter().map(|p| p[0]).collect();
        assert!(
            greys.windows(2).all(|w| w[0] <= w[1]),
            "the ramp must not reverse: {greys:?}"
        );
        assert!(
            greys.iter().any(|v| (8..248).contains(v)),
            "point sampling would give only 0 and 255: {greys:?}"
        );
    }

    /// The filtering runs in premultiplied alpha. A `_CHIP` cell's transparent
    /// margin carries arbitrary colour bytes, and averaging those in
    /// unpremultiplied would drag them into the edge of the sprite beside them
    /// — a black halo around every widget the menus light up.
    #[test]
    fn a_transparent_neighbour_does_not_bleed_its_colour() {
        // Opaque white beside transparent black, which is what a cut-out sits
        // against in a sheet.
        let mut src = Image::black(2, 1);
        src.rgba.copy_from_slice(&[255, 255, 255, 255, 0, 0, 0, 0]);
        let out = resampled(&src, (0, 0, 2, 1), (8, 1)).expect("should scale");
        for px in out.rgba.as_chunks::<4>().0 {
            if px[3] == 0 {
                continue;
            }
            assert!(
                px[0] > 200,
                "the white must stay white where it is visible: {px:?}"
            );
        }
    }

    /// Nothing to do is nothing done, so a screen already at its native size
    /// is not softened by a pass that would only blur it.
    #[test]
    fn art_that_is_already_the_right_size_is_left_alone() {
        let src = Image::black(4, 4);
        assert!(resampled(&src, (0, 0, 4, 4), (4, 4)).is_none());
        assert!(resampled(&src, (0, 0, 0, 4), (8, 8)).is_none());
    }

    /// And each of those really does name a `.cmap` suffix the game ships.
    #[test]
    fn every_art_set_has_the_suffix_the_dll_spells() {
        assert_eq!(Resolution::Standard.suffix(), "");
        assert_eq!(Resolution::Wide.suffix(), "_Wide");
        assert_eq!(Resolution::Note.suffix(), "_Wide_Note");
        assert_eq!(Resolution::Full.suffix(), "_Wide_Full");
    }
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
