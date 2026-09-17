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
//! A screen composites at its hit map's size by default and at whatever
//! [`Screen::fit_to`] is given otherwise — which is how the engine draws one
//! pass from the 800x450 art straight to the pixels the player sees, rather
//! than into the map's size and again onto the window. Hit testing always asks
//! the map, at the map's own size; see [`Screen::hit`].
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
use crate::ui::playdata;
use crate::ui::replay;
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
        "no widget table in the menu module matches the hit map for {0}; \
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

/// A sprite whose source rectangle is not the same size, or even the same
/// shape, as where it lands.
///
/// A widget record carries one rectangle and uses it for both ends, so a
/// sprite that stretches, or that slides a window across its art, cannot be a
/// [`Widget`]. The control bar's affection gauge is both: `FUN_10026540` gives
/// the level piece a 417-wide cut and a 418-wide destination, and slides the
/// cut's origin by the lead one counter has over the other.
///
/// Coordinates are floats because the original's are: the source goes into
/// `DX9Texture` slot `+8` / `+0xc` as `x / width`, so a cut can start at a
/// fraction of a pixel and can run past the sheet's edge, where the sampler's
/// `D3DTEXADDRESS_CLAMP` holds the last texel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cut {
    /// `(x, y, width, height)` in the `_CHIP` sheet's own pixels.
    pub src: (f32, f32, f32, f32),
    /// `(x, y, width, height)` in the screen's layout space.
    pub dst: (f32, f32, f32, f32),
}

/// A screen drawn **inside** another screen's base art: art of its own that
/// covers the layout, plus the sprites it draws over that.
///
/// The Option screen's tab pages and the Replay screen's two views are these.
/// `FUN_10006110` and `FUN_10024480` both draw the page's background and then
/// its contents, and only after both the frame's own highlight and hover sprite
/// — so a page is not a backdrop and not a sprite, it is a layer between the
/// two. See [`crate::ui::option_pages`] and [`crate::ui::replay_pages`].
pub struct Page<'a, 'b> {
    /// The page's full-screen art, **already in display space**: run it
    /// through [`Screen::to_display`] first, the same as a backdrop.
    pub art: &'a Image,
    /// The sprites, in the order they are drawn, each with the sheet it is cut
    /// from. A page's sheet is never the screen's own, and one page can draw
    /// from more than one: the replay grid takes its arrows and page buttons
    /// from `ReplayThum_Chip.png` and the thumbnail under the pointer from
    /// `Replay_Thm01.png`, in that one layer.
    pub sprites: &'b [(&'a Image, Widget)],
}

/// One draw in a composite, in the order it is drawn.
///
/// Every layer is the same shape, and deliberately so: **realize an image at
/// its final size, then alpha-blit it 1:1**. Nothing is stretched or filtered
/// by the blit — a sprite is resampled onto its destination size first, a
/// [`Cut`] is sampled onto it, a source that is bigger than where it lands is
/// averaged down onto it — so the pixels a layer contributes do not depend on
/// what draws it. That is what lets the SDL path draw the same screen as a run
/// of textured quads while `daysengine ui` composites it on the CPU, and get
/// the same picture out of both.
///
/// The original draws its menus this way too: `FUN_1000c740` and the rest hand
/// Direct3D one quad per sprite and never touch a pixel themselves.
pub struct Layer<'a> {
    pub art: Art<'a>,
    /// Where it lands, in the screen's output space.
    pub at: (i64, i64),
    /// How big it is drawn, which is the size [`Layer::realize`] produces.
    pub size: (u32, u32),
}

/// What a [`Layer`] draws, and how its pixels are arrived at.
pub enum Art<'a> {
    /// An image that is already in display space and already the right size.
    /// The base art, a backdrop, a page's art, a buffer drawn elsewhere.
    Whole(&'a Image),
    /// A rectangle of a sheet, brought to the layer's size.
    Cut { sheet: &'a Image, src: Source },
}

/// Which rectangle of a sheet a [`Art::Cut`] takes, and how it is resized.
///
/// The three ways are not interchangeable: they are the three kinds of source
/// the screens actually have, and each is the sampling that kind needs. See
/// [`resampled`], [`sampled`] and [`Image::downscaled`].
pub enum Source {
    /// Whole source pixels, scaled to the destination by the cubic every piece
    /// of this game's art goes through. A widget's chip sprite.
    Sprite((u32, u32, u32, u32)),
    /// A fractional source rectangle, sampled bilinearly with its edges
    /// clamped, the way the original's sampler reads a [`Cut`].
    Sampled((f32, f32, f32, f32)),
    /// Whole source pixels averaged down. For art rasterised at twice the size
    /// it is drawn — the save/load rows and the backlog's lines.
    Average((u32, u32, u32, u32)),
}

/// What identifies a layer's realized pixels.
///
/// A backend that turns them into something expensive — a GPU texture — keys
/// its cache on this. Two layers with the same key realize to the same pixels,
/// because an [`Image`]'s version changes whenever its own do (see
/// [`Image::version`]); two that differ may still look alike, which costs a
/// rebuild and never a wrong picture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Key {
    art: u64,
    /// The source rectangle, as bits so a fractional one compares exactly.
    src: [u32; 4],
    size: (u32, u32),
}

impl Layer<'_> {
    /// The layer's pixels, at [`Layer::size`], ready to be drawn 1:1.
    ///
    /// Borrowed for an [`Art::Whole`], which is already what it needs to be.
    pub fn realize(&self) -> std::borrow::Cow<'_, Image> {
        use std::borrow::Cow;
        match &self.art {
            Art::Whole(img) => Cow::Borrowed(img),
            Art::Cut { sheet, src } => Cow::Owned(match src {
                Source::Sprite(rect) => sprite_art(sheet, *rect, self.size),
                Source::Sampled(rect) => sampled(sheet, *rect, self.size),
                Source::Average(rect) => sheet.downscaled(*rect, self.size),
            }),
        }
    }

    /// What identifies these pixels; see [`Key`].
    pub fn key(&self) -> Key {
        let (art, src) = match &self.art {
            Art::Whole(img) => (img.version(), [0, 0, img.width, img.height]),
            Art::Cut { sheet, src } => (
                sheet.version(),
                match src {
                    Source::Sprite((x, y, w, h)) | Source::Average((x, y, w, h)) => {
                        [*x, *y, *w, *h]
                    }
                    Source::Sampled((x, y, w, h)) => {
                        [x.to_bits(), y.to_bits(), w.to_bits(), h.to_bits()]
                    }
                },
            ),
        };
        Key {
            art,
            src,
            size: self.size,
        }
    }

    /// Where it lands, as a rectangle.
    pub fn rect(&self) -> (i64, i64, u32, u32) {
        (self.at.0, self.at.1, self.size.0, self.size.1)
    }
}

/// Everything that goes into one composite, in the order it is drawn.
///
/// The screens differ in which of these they have, not in what order they go
/// in: `FUN_10006110` and `FUN_1000c740` both draw the background, then the
/// page between, then the widget sprites over it. One list serves all of them,
/// and the [`Screen::compose`] family are the combinations that are actually
/// asked for.
#[derive(Default)]
pub struct Composite<'a, 'b> {
    /// Drawn under everything. A screen that does not own its background — the
    /// title — is composited over one. Already in display space.
    pub backdrop: Option<&'a Image>,
    /// Whether the screen's own base art is drawn. A layer over playback
    /// leaves it out.
    pub base: bool,
    pub page: Option<Page<'a, 'b>>,
    /// Sprites a screen's own module works out and draws **before** it looks at
    /// the pointer, so the hover art of whatever the pointer is on lands on
    /// top of them. Only the Shiny Days title has any: see
    /// [`crate::ui::menu::title_replay_caption`].
    pub under: &'b [(&'a Image, Widget)],
    /// Indexed by widget; a shorter slice leaves the rest resting.
    pub states: &'b [WidgetState],
    /// Sprites a screen's own module works out, each with its sheet.
    pub sprites: &'b [(&'a Image, Widget)],
    /// Cuts drawn last, from the screen's own `_CHIP` sheet.
    pub cuts: &'b [Cut],
}

/// A loaded screen at one resolution.
pub struct Screen {
    /// Logical path stem, e.g. `System/Title/Title`.
    pub path: String,
    pub resolution: Resolution,
    /// Base art as it was decoded, in the native 800x450 layout.
    native_base: Image,
    /// Base art resampled into output space, which is what gets composited.
    base: Image,
    chip: Image,
    /// Hit map at `resolution`.
    display: Cmap,
    /// Scale from native layout space to `resolution`.
    scale: f64,
    /// Vertical offset in display pixels, non-zero only when letterboxed.
    letterbox: f64,
    /// The size this screen composites at, and the scale and offset that go
    /// with it. Equal to the display map's until [`Screen::fit_to`] says
    /// otherwise — see there for why a caller would.
    out: (u32, u32),
    out_scale: f64,
    out_letterbox: f64,
    atlas: Atlas,
    /// Realized [`Art::Cut`]s, keyed the way a texture cache keys them.
    ///
    /// Bringing a cut to the size it is drawn at is the expensive half of
    /// compositing one — at the 1280x720 art set every sprite goes through the
    /// cubic on the way up — and the answer depends on nothing but the key. The
    /// SDL path keeps textures against the same key and never asks twice; this
    /// is the same saving for the path that draws on the CPU, which is the one
    /// the control bar takes on every frame of a fade or a gauge ramp.
    ///
    /// `RefCell` because compositing takes `&self`: it is a memo of a pure
    /// function, not state. Nothing re-enters it — realizing a cut does not
    /// draw anything.
    /// Shared, so that two screens that draw the same sprites realize them
    /// once between them: see [`Screen::share_realized`].
    realized: std::rc::Rc<std::cell::RefCell<std::collections::HashMap<Key, Image>>>,
}

/// How many realized cuts a screen keeps before it starts again.
///
/// A screen's own sprites are bounded by its widget count, but a [`Cut`] slides
/// its source — the gauge's level piece moves by 2.5 pixels per point of lead —
/// so the keys a long session produces are bounded only by how far it has
/// ramped. Well past what any screen holds at once, and cheaper to drop the lot
/// than to track which of them is coldest.
const REALIZED_MAX: usize = 512;

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
    /// menu module, which is where the widget-to-sprite table lives.
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
        Screen::load_with_art(vfs, dll, path, base, None, resolution)
    }

    /// Loads a screen whose chip sheet is not named after the stem either.
    ///
    /// One screen needs this: a menu module that keeps its cleared title under
    /// `Clear/` gives that state its own art *and* its own chip sheet while
    /// leaving it on the plain title's hit map, so both have to move together.
    /// See [`crate::ui::paths::Paths::title_art`].
    pub fn load_with_art(
        vfs: &Vfs,
        dll: &[u8],
        path: &str,
        base: Option<&str>,
        chip: Option<&str>,
        resolution: Resolution,
    ) -> Result<Screen, Error> {
        let base_path = base
            .map(str::to_string)
            .unwrap_or_else(|| format!("{path}.png"));
        let chip_path = chip
            .map(str::to_string)
            .unwrap_or_else(|| format!("{path}_Chip.png"));
        let native_base = Image::decode_png(&read(vfs, &base_path)?)?;
        let chip = Image::decode_png(&read(vfs, &chip_path)?)?;

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
        let display_size = (display.width(), display.height());

        let mut atlas = atlas::find(dll, native.all_bounds(), (chip.width, chip.height))
            .map_err(|_| Error::NoAtlasFor(path.to_string()))?;

        // The play-data list's rows are laid out from records its hit map does
        // not reproduce, so the generic search extrapolates them and lands on
        // the page buttons' alternate states. `playdata::relocate` puts them
        // back from the indices the shipped code reads; a screen whose table
        // will not anchor has no list to draw, so it is refused rather than
        // drawn from the wrong offsets.
        let stem = path.to_ascii_lowercase();
        if stem.ends_with("replay_playdata")
            && !playdata::relocate(&mut atlas, dll, native.all_bounds())
        {
            return Err(Error::NoAtlasFor(path.to_string()));
        }
        // The grid needs only the second half of that: its own widgets are
        // placed, but the records marking the tab and page it is on are not
        // where the generic search reads alternates from.
        if stem.ends_with("replay_hscene") {
            replay::place_alternates(&mut atlas, dll, native.all_bounds());
        }

        // The base art covers the whole screen, so resampling it is the most
        // expensive thing a composite does — and it is the same work every
        // time, because only the sprites over it change. Done once here, and
        // again only when `fit_to` moves the size it is wanted at.
        let size = scaled_size(&native_base, scale);
        let base = resampled(
            &native_base,
            (0, 0, native_base.width, native_base.height),
            size,
        )
        .unwrap_or_else(|| native_base.clone());

        Ok(Screen {
            path: path.to_string(),
            resolution,
            native_base,
            base,
            chip,
            display,
            scale,
            letterbox,
            out: (display_size.0, display_size.1),
            out_scale: scale,
            out_letterbox: letterbox,
            atlas,
            realized: Default::default(),
        })
    }

    /// The size this screen composites at.
    pub fn size(&self) -> (u32, u32) {
        self.out
    }

    /// The hit map's own size, which is the shape the screen is drawn in
    /// whatever it is composited at.
    pub fn map_size(&self) -> (u32, u32) {
        (self.display.width(), self.display.height())
    }

    /// Composites at `width` x `height` instead of the hit map's own size.
    ///
    /// The art is authored once, at 800x450, and everything larger is that art
    /// scaled. Compositing into the hit map's size and *then* scaling the
    /// result onto the window means filtering it twice — and for the filter
    /// that is the default, [`crate::playback::scale::Kernel::Pixel`], twice is
    /// worse than a blur: it band-limits the edges onto the 1280-wide grid and
    /// then again onto the window's, so they land on neither cleanly. One pass,
    /// straight to the size it will be seen at, is the whole point.
    ///
    /// Output space is display space scaled uniformly, so nothing about the
    /// layout changes — only how many pixels it is drawn with. Hit testing is
    /// unaffected: [`Screen::hit`] still asks the display map, at the map's own
    /// size, having scaled the point back into it.
    ///
    /// A size that is not this screen's aspect takes the width and keeps the
    /// aspect, because a stretched UI is never what was meant.
    pub fn fit_to(&mut self, width: u32, height: u32) {
        let (display_w, display_h) = (self.display.width(), self.display.height());
        if width == 0 || height == 0 || display_w == 0 || display_h == 0 {
            return;
        }
        let k = f64::from(width) / f64::from(display_w);
        let out = (width, (f64::from(display_h) * k).round().max(1.0) as u32);
        if out == self.out {
            return;
        }
        self.out = out;
        self.out_scale = self.scale * k;
        self.out_letterbox = self.letterbox * k;
        let size = scaled_size(&self.native_base, self.out_scale);
        self.base = resampled(
            &self.native_base,
            (0, 0, self.native_base.width, self.native_base.height),
            size,
        )
        .unwrap_or_else(|| self.native_base.clone());
    }

    pub fn scale(&self) -> f64 {
        self.scale
    }

    /// How many output pixels one pixel of the 800x450 layout space is.
    ///
    /// [`Screen::scale`] and [`Screen::output_scale`] multiplied together: the
    /// art set's own scale, and whatever [`Screen::fit_to`] has since asked
    /// for. This is the divisor that takes a point in the space
    /// [`Screen::hit`] works in back to the space the module's records are
    /// written in, which [`Screen::to_layout`] does for a point.
    pub fn out_scale(&self) -> f64 {
        self.out_scale
    }

    /// How much larger than its hit map this screen composites.
    ///
    /// 1.0 until [`Screen::fit_to`] has said otherwise, and after that the
    /// factor everything on the screen is magnified by.
    pub fn output_scale(&self) -> f64 {
        let w = self.display.width();
        if w == 0 {
            1.0
        } else {
            f64::from(self.out.0) / f64::from(w)
        }
    }

    /// Scales an image authored in the hit map's space into output space.
    ///
    /// For an overlay that is not part of the screen but is drawn over it: the
    /// save-comment dialog is composited into the same image, so it has to be
    /// magnified by the same factor or it shrinks away as the window grows.
    /// Filtered the way the screen's own art is — see [`resampled`].
    pub fn to_output(&self, img: &Image) -> Image {
        let size = scaled_size(img, self.output_scale());
        resampled(img, (0, 0, img.width, img.height), size).unwrap_or_else(|| img.clone())
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
        // The point arrives in output space; the map is at its own size.
        let back = f64::from(self.display.width()) / f64::from(self.out.0.max(1));
        let x = (f64::from(x) * back) as u32;
        let y = (f64::from(y) * back) as u32;
        let id = self.display.region_at(x, y);
        if id == 0 {
            return None;
        }
        let index = id as usize - 1;
        (index < self.atlas.widgets.len()).then_some(index)
    }

    /// A point in the output space [`Screen::hit`] takes, back in the 800x450
    /// layout space the DLL's records are written in.
    ///
    /// For a screen whose widgets are not all in its hit map: the Option
    /// screen's pages are rectangles in the module rather than regions in the
    /// map, and [`crate::ui::option_pages::Pages::hit`] tests them where they
    /// are written.
    pub fn to_layout(&self, x: u32, y: u32) -> (f32, f32) {
        let scale = if self.out_scale == 0.0 {
            1.0
        } else {
            self.out_scale
        };
        (
            (f64::from(x) / scale) as f32,
            ((f64::from(y) - self.out_letterbox) / scale) as f32,
        )
    }

    /// A point inside a widget's hit region, in the same output space
    /// [`Screen::hit`] takes.
    ///
    /// For driving a screen without a pointer: a selection that lands on a
    /// widget has to become a position, because the control bar's fade, its
    /// caption strip and its dispatch are all asked about a position rather
    /// than about a widget.
    ///
    /// The middle of the bounding box where the widget owns it, and the first
    /// pixel it does own otherwise — the route map's cells are not
    /// rectangular, and the middle of a bounding box there can belong to
    /// nothing at all.
    pub fn widget_point(&self, widget: usize) -> Option<(u32, u32)> {
        let id = u8::try_from(widget + 1).ok()?;
        let bounds = self.display.bounds(id)?;
        let middle = (bounds.x + bounds.width / 2, bounds.y + bounds.height / 2);
        let point = if self.display.region_at(middle.0, middle.1) == id {
            middle
        } else {
            let mut owned = None;
            'scan: for y in bounds.y..bounds.y + bounds.height {
                for x in bounds.x..bounds.x + bounds.width {
                    if self.display.region_at(x, y) == id {
                        owned = Some((x, y));
                        break 'scan;
                    }
                }
            }
            owned?
        };
        let scale = f64::from(self.out.0) / f64::from(self.display.width().max(1));
        Some((
            (f64::from(point.0) * scale) as u32,
            (f64::from(point.1) * scale) as u32,
        ))
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
            (f64::from(x) * self.out_scale).round() as i64,
            (f64::from(y) * self.out_scale + self.out_letterbox).round() as i64,
            (f64::from(w) * self.out_scale).round().max(1.0) as u32,
            (f64::from(h) * self.out_scale).round().max(1.0) as u32,
        )
    }

    fn place(&self, w: &Widget) -> (i64, i64, u32, u32) {
        let round = |v: f64| v.round();
        (
            round(f64::from(w.dst.x) * self.out_scale) as i64,
            round(f64::from(w.dst.y) * self.out_scale + self.out_letterbox) as i64,
            round(f64::from(w.dst.width) * self.out_scale).max(1.0) as u32,
            round(f64::from(w.dst.height) * self.out_scale).max(1.0) as u32,
        )
    }

    /// Resamples a whole image that is authored in native layout space into
    /// this screen's display space. See [`resampled`].
    pub fn to_display(&self, img: &Image) -> Image {
        let size = scaled_size(img, self.out_scale);
        resampled(img, (0, 0, img.width, img.height), size).unwrap_or_else(|| img.clone())
    }

    /// An image that is already in display space, as a layer at the letterbox
    /// offset the widgets get.
    fn whole<'a>(&self, img: &'a Image) -> Layer<'a> {
        Layer {
            art: Art::Whole(img),
            at: (0, self.out_letterbox.round() as i64),
            size: (img.width, img.height),
        }
    }

    /// One widget's chip sprite, as a layer where the display map puts it.
    fn sprite<'a>(&self, sheet: &'a Image, widget: &Widget) -> Layer<'a> {
        let dst = self.place(widget);
        Layer {
            art: Art::Cut {
                sheet,
                src: Source::Sprite((
                    widget.src_x,
                    widget.src_y,
                    widget.dst.width,
                    widget.dst.height,
                )),
            },
            at: (dst.0, dst.1),
            size: (dst.2, dst.3),
        }
    }

    /// A [`Cut`] from a sheet that is not this screen's own, as a layer in this
    /// screen's layout space.
    ///
    /// For art that belongs to a screen other than the loaded one: the
    /// dress-select screen keeps drawing its two dresses from
    /// `DressSelect_Chip.png` while the popup's map and sheet are the ones
    /// loaded. See [`crate::ui::dress::Slide`].
    pub fn cut_layer<'a>(&self, sheet: &'a Image, cut: &Cut) -> Layer<'a> {
        self.cut(sheet, cut)
    }

    /// A [`Cut`] as a layer, placed by its own layout-space rectangle.
    fn cut<'a>(&self, sheet: &'a Image, cut: &Cut) -> Layer<'a> {
        let dst = self.place_layout(cut.dst);
        Layer {
            art: Art::Cut {
                sheet,
                src: Source::Sampled(cut.src),
            },
            at: (dst.0, dst.1),
            size: (dst.2, dst.3),
        }
    }

    /// A rectangle of a sheet averaged down onto a layout-space rectangle.
    ///
    /// For art rasterised at twice the size it is drawn: the save/load rows and
    /// the backlog's lines. Taking one source pixel in four turns a glyph
    /// stroke into a row of specks, so the cell is averaged instead.
    pub fn averaged<'a>(
        &self,
        sheet: &'a Image,
        src: (u32, u32, u32, u32),
        dst: (f32, f32, f32, f32),
    ) -> Layer<'a> {
        let dst = self.place_layout(dst);
        Layer {
            art: Art::Cut {
                sheet,
                src: Source::Average(src),
            },
            at: (dst.0, dst.1),
            size: (dst.2, dst.3),
        }
    }

    /// An image in display space as a layer, for a caller assembling its own
    /// list — the menu's backlog buffer and dress caption are these.
    pub fn whole_layer<'a>(&self, img: &'a Image) -> Layer<'a> {
        Layer {
            art: Art::Whole(img),
            at: (0, self.letterbox.round() as i64),
            size: (img.width, img.height),
        }
    }

    /// One widget's chip sprite alone, at display scale, with where it goes.
    ///
    /// For a sprite that does not land inside the screen's own rectangle and so
    /// cannot go in its layer: `FUN_10021c20` puts the control bar's
    /// `REPLAYMODE` indicator at y = 80, below the 800x75 strip, so it is drawn
    /// on the picture instead.
    pub fn cut_widget(&self, widget: &Widget) -> (Image, (i64, i64, u32, u32)) {
        let layer = self.sprite(&self.chip, widget);
        (layer.realize().into_owned(), layer.rect())
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
        self.compose_layer_cuts(states, &[])
    }

    /// Composites the screen as a transparent layer, with extra cuts on top.
    ///
    /// `cuts` are drawn after the widget sprites, from the `_CHIP` sheet, in
    /// the order given. That is where the original draws the ones this exists
    /// for: `FUN_10024ca0` walks the gauge's three pieces immediately after the
    /// bed record they sit in.
    pub fn compose_layer_cuts(&self, states: &[WidgetState], cuts: &[Cut]) -> Image {
        self.rasterize(
            false,
            &self.layers(&Composite {
                base: true,
                states,
                cuts,
                ..Default::default()
            }),
        )
    }

    /// The sprites alone, on transparency, with no base art under them.
    ///
    /// For a screen that holds part of itself out of something the rest of it
    /// gets: the control bar's gauge keeps its own alpha while the strip fades,
    /// so it is composited separately and laid over the faded strip.
    pub fn compose_sprites(&self, states: &[WidgetState], cuts: &[Cut]) -> Image {
        self.rasterize(
            false,
            &self.layers(&Composite {
                states,
                cuts,
                ..Default::default()
            }),
        )
    }

    /// Draws cuts from a sheet that is not this screen's onto `out`.
    ///
    /// For a layer that is drawn in this screen's layout space but cut from
    /// another screen's art: the dress-select popup covers the two dresses,
    /// which `FUN_1000c740` keeps drawing from `DressSelect_Chip.png` while the
    /// popup's own map and sheet are the ones loaded. They are [`Cut`]s rather
    /// than widgets because the slide leaves them on a fraction of a pixel —
    /// see [`crate::ui::dress::Slide`].
    pub fn draw_cuts_from(&self, out: &mut Image, sheet: &Image, cuts: &[Cut]) {
        for cut in cuts {
            self.draw(out, &self.cut(sheet, cut));
        }
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
        self.compose_over_page(backdrop, None, states, sprites)
    }

    /// Composites the screen with a [`Page`] over its base art.
    ///
    /// The page goes down between the base and the screen's own widget
    /// sprites, which is where `FUN_10006110` draws it: the frame's tab
    /// highlight and its hover sprite are drawn after the page's contents, so
    /// they sit on top of it.
    pub fn compose_over_page(
        &self,
        backdrop: Option<&Image>,
        page: Option<Page>,
        states: &[WidgetState],
        sprites: &[(&Image, Widget)],
    ) -> Image {
        self.rasterize(
            true,
            &self.layers(&Composite {
                backdrop,
                base: true,
                page,
                states,
                sprites,
                ..Default::default()
            }),
        )
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
        self.rasterize(
            true,
            &self.layers(&Composite {
                backdrop,
                base: true,
                states,
                ..Default::default()
            }),
        )
    }

    /// What a composite draws, in order, without drawing any of it.
    ///
    /// This is the composite: [`Screen::rasterize`] is one backend for it and
    /// the SDL path is the other. A caller with layers of its own — the menu's
    /// backlog buffer, its save/load rows — appends them to this list rather
    /// than compositing over the result, so that both backends see one list.
    pub fn layers<'a>(&'a self, what: &Composite<'a, '_>) -> Vec<Layer<'a>> {
        let mut layers = Vec::new();
        if let Some(under) = what.backdrop {
            layers.push(self.whole(under));
        }
        if what.base {
            layers.push(self.whole(&self.base));
        }
        if let Some(page) = &what.page {
            layers.push(self.whole(page.art));
            for (sheet, sprite) in page.sprites {
                layers.push(self.sprite(sheet, sprite));
            }
        }
        for (sheet, sprite) in what.under {
            layers.push(self.sprite(sheet, sprite));
        }
        for (i, state) in what.states.iter().enumerate() {
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
            layers.push(self.sprite(&self.chip, widget));
        }
        for (sheet, widget) in what.sprites {
            layers.push(self.sprite(sheet, widget));
        }
        for cut in what.cuts {
            layers.push(self.cut(&self.chip, cut));
        }
        layers
    }

    /// Draws a list of layers into an image of this screen's size.
    ///
    /// `opaque` starts it from black rather than from transparency; see
    /// [`Screen::compose_layer`] for which screens want which.
    ///
    /// This is the headless backend, and it is the one the engine is verified
    /// through: the SDL path draws the same layers through the GPU, which no
    /// test can read back.
    pub fn rasterize(&self, opaque: bool, layers: &[Layer]) -> Image {
        let (w, h) = self.size();
        let mut out = if opaque {
            Image::black(w, h)
        } else {
            Image::empty(w, h)
        };
        for layer in layers {
            self.draw(&mut out, layer);
        }
        out
    }

    /// Realizes what this screen draws into the same memo as another screen's.
    ///
    /// The dress-select screen keeps both of its hit maps loaded and draws the
    /// two dresses through whichever of them is up — see
    /// [`crate::ui::menu::Menu::load_dress`]. The sprites are the same sprites
    /// at the same size, and what identifies a realized cut says nothing about
    /// which screen asked for it, so the two share one answer rather than each
    /// paying for it.
    pub fn share_realized(&mut self, other: &Screen) {
        self.realized = std::rc::Rc::clone(&other.realized);
    }

    /// A layer's realized pixels, when this screen has them already.
    ///
    /// `None` rather than realizing them, so that a caller that turns them
    /// into something of its own — the player's textures — follows the pace
    /// [`Screen::warm`] sets rather than paying for a sprite the screen has
    /// not brought to size yet. A copy, because the memo outlives the answer
    /// and copying a sprite costs a fraction of realizing one.
    pub fn realized(&self, layer: &Layer) -> Option<Image> {
        self.realized.borrow().get(&layer.key()).cloned()
    }

    /// Whether [`Screen::realized`] would answer, without the copy.
    pub fn is_realized(&self, layer: &Layer) -> bool {
        self.realized.borrow().contains_key(&layer.key())
    }

    /// Brings a layer's pixels to the size they are drawn at and keeps them,
    /// without drawing anything.
    ///
    /// For a layer that is about to be wanted on a frame that will not have
    /// time to prepare it — see [`crate::ui::menu::Menu::warm_layers`]. True
    /// when this call is what realized it, so that a caller warming a list can
    /// spread the work over as many frames as the list is long. An
    /// [`Art::Whole`] has nothing to prepare and is ignored.
    pub fn warm(&self, layer: &Layer) -> bool {
        if matches!(layer.art, Art::Whole(_)) {
            return false;
        }
        let mut realized = self.realized.borrow_mut();
        if realized.len() >= REALIZED_MAX {
            realized.clear();
        }
        let key = layer.key();
        if realized.contains_key(&key) {
            return false;
        }
        realized.insert(key, layer.realize().into_owned());
        true
    }

    /// Draws one layer onto an image, which is always a 1:1 alpha blit of its
    /// realized pixels.
    ///
    /// A cut is realized once and kept; see [`Screen::realized`]. An
    /// [`Art::Whole`] is not — it is already the pixels it needs to be, and
    /// copying the base art into a cache to read it back is work for nothing.
    pub fn draw(&self, out: &mut Image, layer: &Layer) {
        if let Art::Whole(art) = &layer.art {
            out.blit_scaled(art, (0, 0, art.width, art.height), layer.rect());
            return;
        }
        let mut realized = self.realized.borrow_mut();
        if realized.len() >= REALIZED_MAX {
            realized.clear();
        }
        let art = realized
            .entry(layer.key())
            .or_insert_with(|| layer.realize().into_owned());
        out.blit_scaled(art, (0, 0, art.width, art.height), layer.rect());
    }
}

/// A widget's source rectangle brought to the size it is drawn at.
///
/// [`resampled`] returns `None` when there is nothing to scale, which is every
/// screen at its native size; then the cut is simply copied out of the sheet.
fn sprite_art(sheet: &Image, src: (u32, u32, u32, u32), size: (u32, u32)) -> Image {
    match resampled(sheet, src, size) {
        Some(scaled) => scaled,
        None => {
            let mut cut = Image::empty(size.0, size.1);
            cut.blit_scaled(sheet, src, (0, 0, size.0, size.1));
            cut
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
    Some(Image::from_rgba(size.0, size.1, rgba))
}

/// Samples `rect` of `src` into an image of `size`, the way the GPU samples a
/// textured quad.
///
/// [`resampled`] cannot do this: its source is a whole number of pixels and has
/// to lie inside the sheet. A [`Cut`]'s does neither — the gauge's level piece
/// slides its origin by 2.5 pixels per point of lead and runs off both ends of
/// the sheet at the extremes — so this samples bilinearly at the texel the
/// destination pixel's centre maps to, clamping at the edges as
/// `D3DSAMP_ADDRESSU`/`V` do.
///
/// Premultiplied, for the reason [`resampled`] is: the sheet's transparent
/// pixels carry arbitrary colour, and blending them unpremultiplied would drag
/// it into the edge of the cut.
fn sampled(src: &Image, rect: (f32, f32, f32, f32), size: (u32, u32)) -> Image {
    let (sx, sy, sw, sh) = rect;
    let (dw, dh) = size;
    let mut out = Image::empty(dw, dh);
    if dw == 0 || dh == 0 || src.width == 0 || src.height == 0 {
        return out;
    }
    // Which two source columns each destination pixel mixes, and in what
    // proportion, worked out once for the whole image: the map is the same on
    // every row, and inline it cost a division and two clamps per pixel. This
    // is the saving `Image::blit_scaled` takes, for the same reason.
    let cols: Vec<(usize, usize, f32)> = (0..dw)
        .map(|col| {
            let u = sx + (col as f32 + 0.5) * sw / dw as f32 - 0.5;
            texels(u, src.width)
        })
        .collect();
    // The window of the sheet those columns read, so that a row of it is
    // premultiplied into the width it is sampled through rather than the
    // sheet's own.
    let from = cols.iter().map(|(x0, _, _)| *x0).min().unwrap_or(0);
    let to = cols.iter().map(|(_, x1, _)| *x1 + 1).max().unwrap_or(0);
    let mut top = Row::default();
    let mut bottom = Row::default();
    for (row, line) in out.rgba.chunks_exact_mut(dw as usize * 4).enumerate() {
        let v = sy + (row as f32 + 0.5) * sh / dh as f32 - 0.5;
        let (y0, y1, fy) = texels(v, src.height);
        // Both rows are premultiplied once and read by every destination pixel
        // on the line, and the row a line ends on is usually the row the next
        // one starts on: at 1080p the dresses are drawn 2.4x their own size, so
        // each source row is read by five lines. Converting it per sample cost
        // four divisions a pixel.
        if bottom.at == Some(y0) {
            std::mem::swap(&mut top, &mut bottom);
        }
        top.fill(src, y0, from, to);
        if y1 != y0 {
            bottom.fill(src, y1, from, to);
        }
        let below = if y1 == y0 { &top } else { &bottom };
        for (px, (x0, x1, fx)) in line.as_chunks_mut::<4>().0.iter_mut().zip(&cols) {
            let (x0, x1) = (x0 - from, x1 - from);
            *px = mix(
                [top.px[x0], top.px[x1], below.px[x0], below.px[x1]],
                *fx,
                fy,
            );
        }
    }
    out
}

/// One row of a sheet, premultiplied, over the columns a [`sampled`] cut reads.
///
/// Premultiplied for the reason [`resampled`] is: the sheet's transparent
/// pixels carry arbitrary colour, and blending them unpremultiplied would drag
/// it into the edge of the cut. The fourth channel is the alpha itself, which
/// is what the blend carries through.
#[derive(Default)]
struct Row {
    /// Which row of the sheet this holds, so that a row already converted is
    /// not converted again.
    at: Option<usize>,
    px: Vec<[f32; 4]>,
}

impl Row {
    fn fill(&mut self, src: &Image, row: usize, from: usize, to: usize) {
        if self.at == Some(row) {
            return;
        }
        let stride = src.width as usize * 4;
        let line = &src.rgba[row * stride + from * 4..row * stride + to * 4];
        self.px.clear();
        self.px.extend(line.as_chunks::<4>().0.iter().map(|p| {
            let a = f32::from(p[3]) / 255.0;
            [
                f32::from(p[0]) * a,
                f32::from(p[1]) * a,
                f32::from(p[2]) * a,
                f32::from(p[3]),
            ]
        }));
        self.at = Some(row);
    }
}

/// The two texels a coordinate falls between and how far it is between them,
/// with the edges clamped as `D3DSAMP_ADDRESSU`/`V` clamp them.
fn texels(t: f32, extent: u32) -> (usize, usize, f32) {
    let clamp = |v: f32| (v.max(0.0) as u32).min(extent - 1) as usize;
    let floor = t.floor();
    (
        clamp(floor),
        clamp(floor + 1.0),
        (t - floor).clamp(0.0, 1.0),
    )
}

/// The bilinear blend of four premultiplied texels, in the order top left, top
/// right, bottom left, bottom right.
fn mix(texels: [[f32; 4]; 4], fx: f32, fy: f32) -> [u8; 4] {
    let mut acc = [0.0f32; 4];
    for (p, w) in texels.into_iter().zip([
        (1.0 - fx) * (1.0 - fy),
        fx * (1.0 - fy),
        (1.0 - fx) * fy,
        fx * fy,
    ]) {
        for (slot, c) in acc.iter_mut().zip(p) {
            *slot += c * w;
        }
    }
    let a = acc[3];
    let mut px = [0u8; 4];
    px[3] = a.round().clamp(0.0, 255.0) as u8;
    if a > 0.0 {
        for (o, c) in px[..3].iter_mut().zip(acc) {
            *o = (c * 255.0 / a).round().clamp(0.0, 255.0) as u8;
        }
    }
    px
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

    /// A cut is sampled at the texel its destination pixel's centre falls on,
    /// with the edges clamped the way `D3DSAMP_ADDRESSU`/`V` clamp them: a
    /// source rectangle that runs off the sheet repeats its edge texel rather
    /// than wrapping or dropping out.
    #[test]
    fn a_cut_samples_between_texels_and_clamps_at_the_edges() {
        let mut src = Image::black(2, 1);
        src.rgba
            .copy_from_slice(&[0, 0, 0, 255, 255, 255, 255, 255]);
        let ramp = sampled(&src, (0.0, 0.0, 2.0, 1.0), (8, 1));
        let greys: Vec<u8> = ramp.rgba.as_chunks::<4>().0.iter().map(|p| p[0]).collect();
        assert!(
            greys.windows(2).all(|w| w[0] <= w[1]),
            "the ramp must not reverse: {greys:?}"
        );
        assert!(
            greys.iter().any(|v| (8..248).contains(v)),
            "point sampling would give only 0 and 255: {greys:?}"
        );
        let off = sampled(&src, (-4.0, -4.0, 2.0, 1.0), (4, 1));
        assert!(
            off.rgba.as_chunks::<4>().0.iter().all(|p| p[0] == 0),
            "off the top left, every sample is the corner texel"
        );
    }

    /// A transparent texel's colour is arbitrary — the sheets carry black
    /// under the alpha — so the blend weights colour by alpha and takes it
    /// back out, or the edge of every cut would be dragged towards it.
    #[test]
    fn a_cut_blends_colour_through_alpha() {
        let mut src = Image::black(2, 1);
        src.rgba.copy_from_slice(&[0, 0, 0, 0, 200, 100, 50, 255]);
        let out = sampled(&src, (0.0, 0.0, 2.0, 1.0), (4, 1));
        for px in out.rgba.as_chunks::<4>().0 {
            if px[3] == 0 {
                continue;
            }
            assert_eq!(
                [px[0], px[1], px[2]],
                [200, 100, 50],
                "a half-covered pixel keeps the covered texel's colour"
            );
        }
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
