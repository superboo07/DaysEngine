//! The Win32 dialog templates in the player's own executable.
//!
//! One piece of the game's UI is not drawn from its art at all. When the save
//! screen asks the player to name a save, the menu DLL does not draw a box: it
//! hands the current comment to host slot `+0xdc`, which stores it and posts
//! `WM_USER` to the game window. The window procedure's `0x400` case opens a
//! **modal dialog from the executable's own resources**:
//!
//! ```text
//! host +0xdc  ->  SendMessageA(hwnd, WM_USER, 0, 0)
//! wnd proc    ->  DialogBoxParamA(hinst, 0x73 | 0x77, hwnd, FUN_0042e4a0, 0)
//!                   0x73 Japanese, 0x77 English -- chosen by host +0x5c
//! OK          ->  GetWindowTextA(edit 0x40c, buf, 0x79) -> _CommentSet@4
//! Cancel      ->  EndDialog(hwnd, 2), and nothing is set
//! ```
//!
//! DaysEngine cannot open a Win32 dialog, so it draws one — but the layout,
//! the caption, the prompt and the button captions are **read out of the
//! player's executable** rather than invented, which is the same rule the rest
//! of the engine follows. This module is that reader.
//!
//! # The template
//!
//! Both are `DIALOGEX`, which is a header followed by one record per control,
//! each aligned to four bytes:
//!
//! ```text
//! dlgVer=1  signature=0xFFFF  helpID  exStyle  style
//! cDlgItems  x  y  cx  cy
//! menu  windowClass  title              each a sz_Or_Ord
//! pointsize weight italic charset typeface   only if DS_SETFONT
//! per item:
//!   helpID  exStyle  style  x  y  cx  cy  id
//!   windowClass  title                   each a sz_Or_Ord
//!   extraCount  extra[extraCount]
//! ```
//!
//! A `sz_Or_Ord` is an empty `0x0000`, an ordinal introduced by `0xFFFF`, or a
//! NUL-terminated UTF-16 string.
//!
//! # Dialog units
//!
//! The rectangles are in dialog units, not pixels: Windows scales them by the
//! dialog font's own base units, `x * base_x / 4` and `y * base_y / 8`. The
//! original's size therefore depended on whatever `MS Shell Dlg` resolved to on
//! the player's machine. [`Rect::to_pixels`] applies the same rule against the
//! base units the caller measures from the font it is drawing with, so the
//! proportions are the template's even though the pixels cannot be.

use std::path::Path;

/// What went wrong reading a template.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("not a PE image: {0}")]
    NotPe(&'static str),
    #[error("the executable has no dialog resource {0:#x}")]
    NoResource(u16),
    #[error("dialog resource {0:#x} is not a DIALOGEX")]
    NotDialogEx(u16),
    #[error("dialog resource {id:#x} is truncated in {what}")]
    Truncated { id: u16, what: &'static str },
}

/// A rectangle in dialog units.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i16,
    pub y: i16,
    pub cx: i16,
    pub cy: i16,
}

impl Rect {
    /// The rectangle in pixels, by Windows' own rule.
    ///
    /// `base` is the dialog base units of the font being drawn with: the
    /// average character width and the character height.
    pub fn to_pixels(self, base: (i32, i32)) -> (i32, i32, i32, i32) {
        let x = |v: i16| i32::from(v) * base.0 / 4;
        let y = |v: i16| i32::from(v) * base.1 / 8;
        (x(self.x), y(self.y), x(self.cx), y(self.cy))
    }
}

/// The kind of control, by the window class the record names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Class {
    Button,
    Edit,
    Static,
    ListBox,
    ScrollBar,
    ComboBox,
    /// A class named rather than given as one of the six ordinals.
    Named(String),
}

impl Class {
    fn from_ordinal(n: u16) -> Class {
        match n {
            0x80 => Class::Button,
            0x81 => Class::Edit,
            0x82 => Class::Static,
            0x83 => Class::ListBox,
            0x84 => Class::ScrollBar,
            0x85 => Class::ComboBox,
            n => Class::Named(format!("{n:#06x}")),
        }
    }
}

/// One control.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub id: u32,
    pub class: Class,
    pub rect: Rect,
    pub style: u32,
    pub text: String,
}

/// A dialog template.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Template {
    pub id: u16,
    pub title: String,
    pub rect: Rect,
    /// Point size and typeface, when the template sets a font.
    pub font: Option<(u16, String)>,
    pub items: Vec<Item>,
}

impl Template {
    /// Reads dialog resource `id` out of a PE image.
    pub fn find(exe: &[u8], id: u16) -> Result<Template, Error> {
        let at = resource(exe, RT_DIALOG, id)?.ok_or(Error::NoResource(id))?;
        parse(exe, at, id)
    }

    /// Reads a dialog resource out of the executable beside the game.
    ///
    /// The comment dialog lives in `SCHOOLDAYS HQ.exe`; nothing in the packs
    /// carries it.
    pub fn from_game(game: &Path, id: u16) -> Result<Template, Error> {
        let path = game.join("SCHOOLDAYS HQ.exe");
        let bytes = std::fs::read(&path).map_err(|_| Error::NotPe("cannot read the executable"))?;
        Template::find(&bytes, id)
    }

    /// The control with this id.
    pub fn item(&self, id: u32) -> Option<&Item> {
        self.items.iter().find(|i| i.id == id)
    }
}

/// `RT_DIALOG`.
const RT_DIALOG: u32 = 5;

/// The resource ids of the comment dialog, from the window procedure's own
/// `WM_USER` case: Japanese when host `+0x5c` says so, English otherwise.
pub const COMMENT_JP: u16 = 0x73;
pub const COMMENT_EN: u16 = 0x77;

/// Which of the two to open.
pub fn comment_dialog(english: bool) -> u16 {
    if english {
        COMMENT_EN
    } else {
        COMMENT_JP
    }
}

/// The control ids the dialog procedure uses.
pub mod control {
    /// `IDOK`, read by `FUN_0042e4a0` as `param_3 == 1`.
    pub const OK: u32 = 1;
    /// `IDCANCEL`.
    pub const CANCEL: u32 = 2;
    /// The edit field, `GetDlgItem(hwnd, 0x40c)`.
    pub const EDIT: u32 = 0x40c;
    /// The prompt above it.
    pub const PROMPT: u32 = 0x410;
}

fn u16le(b: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(o..o + 2)?.try_into().ok()?))
}

fn u32le(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?))
}

/// Where a resource's bytes start in the file, by type and id.
fn resource(exe: &[u8], kind: u32, id: u16) -> Result<Option<usize>, Error> {
    let (base, sections) = sections(exe)?;
    let dir = data_directory(exe, 2)?;
    let Some(root) = to_file(&sections, base + dir) else {
        return Ok(None);
    };

    // Each level is a header then named entries then id entries; the high bit
    // of an offset means it points at another directory rather than at a leaf.
    let entries = |off: usize| -> Vec<(u32, u32)> {
        let named = u16le(exe, off + 12).unwrap_or(0) as usize;
        let ids = u16le(exe, off + 14).unwrap_or(0) as usize;
        (0..named + ids)
            .filter_map(|i| {
                let at = off + 16 + i * 8;
                Some((u32le(exe, at)?, u32le(exe, at + 4)?))
            })
            .collect()
    };

    for (name, child) in entries(root) {
        if name & 0x7fff_ffff != kind || child & 0x8000_0000 == 0 {
            continue;
        }
        for (rid, langs) in entries(root + (child & 0x7fff_ffff) as usize) {
            if rid & 0x7fff_ffff != u32::from(id) || langs & 0x8000_0000 == 0 {
                continue;
            }
            let Some(&(_, leaf)) = entries(root + (langs & 0x7fff_ffff) as usize).first() else {
                continue;
            };
            // A leaf is an IMAGE_RESOURCE_DATA_ENTRY: an RVA and a size.
            let rva = u32le(exe, root + leaf as usize).unwrap_or(0);
            return Ok(to_file(&sections, base + rva));
        }
    }
    Ok(None)
}

struct Cursor<'a> {
    b: &'a [u8],
    o: usize,
    id: u16,
}

impl Cursor<'_> {
    fn u16(&mut self, what: &'static str) -> Result<u16, Error> {
        let v = u16le(self.b, self.o).ok_or(Error::Truncated { id: self.id, what })?;
        self.o += 2;
        Ok(v)
    }

    fn i16(&mut self, what: &'static str) -> Result<i16, Error> {
        Ok(self.u16(what)? as i16)
    }

    fn u32(&mut self, what: &'static str) -> Result<u32, Error> {
        let v = u32le(self.b, self.o).ok_or(Error::Truncated { id: self.id, what })?;
        self.o += 4;
        Ok(v)
    }

    fn align(&mut self) {
        self.o = self.o.next_multiple_of(4);
    }

    /// An empty marker, an ordinal, or a NUL-terminated UTF-16 string.
    fn sz_or_ord(&mut self, what: &'static str) -> Result<(String, Option<u16>), Error> {
        let first = u16le(self.b, self.o).ok_or(Error::Truncated { id: self.id, what })?;
        if first == 0 {
            self.o += 2;
            return Ok((String::new(), None));
        }
        if first == 0xffff {
            self.o += 2;
            let ord = self.u16(what)?;
            return Ok((String::new(), Some(ord)));
        }
        let mut units = Vec::new();
        loop {
            let u = self.u16(what)?;
            if u == 0 {
                break;
            }
            units.push(u);
        }
        Ok((String::from_utf16_lossy(&units), None))
    }
}

fn parse(exe: &[u8], at: usize, id: u16) -> Result<Template, Error> {
    let mut c = Cursor { b: exe, o: at, id };
    let (ver, sig) = (c.u16("the version")?, c.u16("the signature")?);
    if ver != 1 || sig != 0xffff {
        return Err(Error::NotDialogEx(id));
    }
    let _help = c.u32("the help id")?;
    let _ex = c.u32("the extended style")?;
    let style = c.u32("the style")?;
    let count = c.u16("the item count")?;
    let rect = Rect {
        x: c.i16("the rectangle")?,
        y: c.i16("the rectangle")?,
        cx: c.i16("the rectangle")?,
        cy: c.i16("the rectangle")?,
    };
    let _menu = c.sz_or_ord("the menu")?;
    let _class = c.sz_or_ord("the class")?;
    let (title, _) = c.sz_or_ord("the title")?;

    /// `DS_SETFONT`. Present in both comment dialogs, which is why the font
    /// block has to be stepped over before the items.
    const DS_SETFONT: u32 = 0x40;
    let font = if style & DS_SETFONT != 0 {
        let points = c.u16("the font size")?;
        let _weight = c.u16("the font weight")?;
        let _italic_and_charset = c.u16("the font flags")?;
        let (face, _) = c.sz_or_ord("the typeface")?;
        Some((points, face))
    } else {
        None
    };

    let mut items = Vec::with_capacity(count as usize);
    for _ in 0..count {
        c.align();
        let _help = c.u32("an item's help id")?;
        let ex = c.u32("an item's extended style")?;
        let style = c.u32("an item's style")?;
        let rect = Rect {
            x: c.i16("an item's rectangle")?,
            y: c.i16("an item's rectangle")?,
            cx: c.i16("an item's rectangle")?,
            cy: c.i16("an item's rectangle")?,
        };
        let id = c.u32("an item's id")?;
        let (class_name, class_ord) = c.sz_or_ord("an item's class")?;
        let (text, _) = c.sz_or_ord("an item's text")?;
        let extra = c.u16("an item's extra count")?;
        c.o += extra as usize;
        let _ = ex;
        items.push(Item {
            id,
            class: match class_ord {
                Some(n) => Class::from_ordinal(n),
                None => Class::Named(class_name),
            },
            rect,
            style,
            text,
        });
    }

    Ok(Template {
        id,
        title,
        rect,
        font,
        items,
    })
}

/// One PE section, reduced to the mapping this module needs.
struct Section {
    va: u32,
    vsize: u32,
    raw: u32,
    rsize: u32,
}

fn sections(exe: &[u8]) -> Result<(u32, Vec<Section>), Error> {
    if exe.get(..2) != Some(b"MZ") {
        return Err(Error::NotPe("no MZ signature"));
    }
    let pe = u32le(exe, 0x3c).ok_or(Error::NotPe("truncated at e_lfanew"))? as usize;
    if exe.get(pe..pe + 4) != Some(b"PE\0\0") {
        return Err(Error::NotPe("no PE signature"));
    }
    let count = u16le(exe, pe + 6).ok_or(Error::NotPe("truncated file header"))? as usize;
    let opt_size = u16le(exe, pe + 20).ok_or(Error::NotPe("truncated file header"))? as usize;
    let opt = pe + 24;
    if u16le(exe, opt) != Some(0x10b) {
        return Err(Error::NotPe("not a 32-bit PE32 image"));
    }
    let base = u32le(exe, opt + 28).ok_or(Error::NotPe("truncated optional header"))?;
    let table = opt + opt_size;
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let h = table + i * 40;
        out.push(Section {
            va: base + u32le(exe, h + 12).ok_or(Error::NotPe("truncated section table"))?,
            vsize: u32le(exe, h + 8).ok_or(Error::NotPe("truncated section table"))?,
            rsize: u32le(exe, h + 16).ok_or(Error::NotPe("truncated section table"))?,
            raw: u32le(exe, h + 20).ok_or(Error::NotPe("truncated section table"))?,
        });
    }
    Ok((base, out))
}

/// The RVA of one of the optional header's data directories.
fn data_directory(exe: &[u8], index: usize) -> Result<u32, Error> {
    let pe = u32le(exe, 0x3c).ok_or(Error::NotPe("truncated at e_lfanew"))? as usize;
    let opt = pe + 24;
    let count = u32le(exe, opt + 92).ok_or(Error::NotPe("truncated optional header"))? as usize;
    if index >= count {
        return Ok(0);
    }
    u32le(exe, opt + 96 + index * 8).ok_or(Error::NotPe("truncated data directory"))
}

fn to_file(secs: &[Section], va: u32) -> Option<usize> {
    secs.iter().find_map(|s| {
        let off = va.checked_sub(s.va)?;
        (off < s.vsize && off < s.rsize).then_some((s.raw + off) as usize)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a PE32 image whose resource directory holds one dialog.
    fn image(id: u16, template: &[u8]) -> Vec<u8> {
        const PE: usize = 0x80;
        const OPT: usize = 0xe0;
        const RAW: u32 = 0x400;
        const VA: u32 = 0x1000;
        let mut v = vec![0u8; RAW as usize];
        v[0..2].copy_from_slice(b"MZ");
        v[0x3c..0x40].copy_from_slice(&(PE as u32).to_le_bytes());
        v[PE..PE + 4].copy_from_slice(b"PE\0\0");
        v[PE + 6..PE + 8].copy_from_slice(&1u16.to_le_bytes());
        v[PE + 20..PE + 22].copy_from_slice(&(OPT as u16).to_le_bytes());
        v[PE + 24..PE + 26].copy_from_slice(&0x10bu16.to_le_bytes());
        v[PE + 24 + 28..PE + 24 + 32].copy_from_slice(&0u32.to_le_bytes()); // image base 0
        v[PE + 24 + 92..PE + 24 + 96].copy_from_slice(&3u32.to_le_bytes()); // data directories
        v[PE + 24 + 96 + 16..PE + 24 + 96 + 20].copy_from_slice(&VA.to_le_bytes()); // resources

        // Three levels of directory, each one header and one entry, then the
        // leaf, then the template itself.
        let mut r: Vec<u8> = Vec::new();
        let dir = |child: u32, name: u32, out: &mut Vec<u8>| {
            out.extend_from_slice(&[0u8; 12]);
            out.extend_from_slice(&0u16.to_le_bytes()); // named entries
            out.extend_from_slice(&1u16.to_le_bytes()); // id entries
            out.extend_from_slice(&name.to_le_bytes());
            out.extend_from_slice(&child.to_le_bytes());
        };
        const LEVEL: u32 = 24;
        dir(0x8000_0000 | LEVEL, RT_DIALOG, &mut r);
        dir(0x8000_0000 | (LEVEL * 2), u32::from(id), &mut r);
        dir(LEVEL * 3, 0x409, &mut r);
        let body = VA + LEVEL * 3 + 16;
        r.extend_from_slice(&body.to_le_bytes());
        r.extend_from_slice(&(template.len() as u32).to_le_bytes());
        r.extend_from_slice(&[0u8; 8]);
        r.extend_from_slice(template);

        let h = PE + 24 + OPT;
        v[h + 8..h + 12].copy_from_slice(&(r.len() as u32).to_le_bytes());
        v[h + 12..h + 16].copy_from_slice(&VA.to_le_bytes());
        v[h + 16..h + 20].copy_from_slice(&(r.len() as u32).to_le_bytes());
        v[h + 20..h + 24].copy_from_slice(&RAW.to_le_bytes());
        v.extend_from_slice(&r);
        v
    }

    fn wide(s: &str) -> Vec<u8> {
        s.encode_utf16()
            .chain(std::iter::once(0))
            .flat_map(u16::to_le_bytes)
            .collect()
    }

    /// A DIALOGEX shaped like the comment dialog: a caption, a font block, a
    /// prompt, an edit field and two buttons.
    fn template() -> Vec<u8> {
        let mut t: Vec<u8> = Vec::new();
        t.extend_from_slice(&1u16.to_le_bytes()); // dlgVer
        t.extend_from_slice(&0xffffu16.to_le_bytes()); // signature
        t.extend_from_slice(&0u32.to_le_bytes()); // helpID
        t.extend_from_slice(&0u32.to_le_bytes()); // exStyle
        t.extend_from_slice(&0x90c0_02cau32.to_le_bytes()); // style, DS_SETFONT set
        t.extend_from_slice(&2u16.to_le_bytes()); // items
        for v in [0i16, 0, 280, 62] {
            t.extend_from_slice(&v.to_le_bytes());
        }
        t.extend_from_slice(&0u16.to_le_bytes()); // no menu
        t.extend_from_slice(&0u16.to_le_bytes()); // no class
        t.extend_from_slice(&wide("Enter comment"));
        t.extend_from_slice(&8u16.to_le_bytes()); // point size
        t.extend_from_slice(&400u16.to_le_bytes()); // weight
        t.extend_from_slice(&[0, 1]); // italic, charset
        t.extend_from_slice(&wide("MS Shell Dlg"));

        let item = |id: u32, class: u16, rect: [i16; 4], text: &str, t: &mut Vec<u8>| {
            while !t.len().is_multiple_of(4) {
                t.push(0);
            }
            t.extend_from_slice(&0u32.to_le_bytes()); // helpID
            t.extend_from_slice(&0u32.to_le_bytes()); // exStyle
            t.extend_from_slice(&0x5081_0080u32.to_le_bytes()); // style
            for v in rect {
                t.extend_from_slice(&v.to_le_bytes());
            }
            t.extend_from_slice(&id.to_le_bytes());
            t.extend_from_slice(&0xffffu16.to_le_bytes());
            t.extend_from_slice(&class.to_le_bytes());
            t.extend_from_slice(&wide(text));
            t.extend_from_slice(&0u16.to_le_bytes()); // no creation data
        };
        item(control::EDIT, 0x81, [7, 21, 266, 14], "", &mut t);
        item(control::OK, 0x80, [168, 41, 50, 14], "OK", &mut t);
        t
    }

    #[test]
    fn reads_a_template_out_of_the_resource_directory() {
        let exe = image(COMMENT_EN, &template());
        let t = Template::find(&exe, COMMENT_EN).expect("finds it");
        assert_eq!(t.title, "Enter comment");
        assert_eq!(
            t.rect,
            Rect {
                x: 0,
                y: 0,
                cx: 280,
                cy: 62
            }
        );
        assert_eq!(t.font, Some((8, "MS Shell Dlg".to_owned())));
        assert_eq!(t.items.len(), 2);
    }

    /// The font block sits between the header and the items, so a reader that
    /// skipped it would take the typeface for the first control.
    #[test]
    fn steps_over_the_font_block_to_reach_the_items() {
        let exe = image(COMMENT_EN, &template());
        let t = Template::find(&exe, COMMENT_EN).unwrap();
        let edit = t.item(control::EDIT).expect("the edit field");
        assert_eq!(edit.class, Class::Edit);
        assert_eq!(
            edit.rect,
            Rect {
                x: 7,
                y: 21,
                cx: 266,
                cy: 14
            }
        );
        let ok = t.item(control::OK).expect("the OK button");
        assert_eq!(ok.class, Class::Button);
        assert_eq!(ok.text, "OK");
    }

    #[test]
    fn a_resource_that_is_not_there_is_named_rather_than_guessed() {
        let exe = image(COMMENT_EN, &template());
        assert!(matches!(
            Template::find(&exe, COMMENT_JP),
            Err(Error::NoResource(_))
        ));
    }

    /// Windows' own rule: `x * base_x / 4` and `y * base_y / 8`.
    #[test]
    fn converts_dialog_units_the_way_windows_does() {
        let r = Rect {
            x: 7,
            y: 21,
            cx: 266,
            cy: 14,
        };
        assert_eq!(r.to_pixels((6, 13)), (10, 34, 399, 22));
        // A wider font makes the box wider in the same proportion.
        assert_eq!(r.to_pixels((12, 13)).2, 798);
    }

    #[test]
    fn picks_the_dialog_for_the_installs_language() {
        assert_eq!(comment_dialog(true), COMMENT_EN);
        assert_eq!(comment_dialog(false), COMMENT_JP);
    }
}
