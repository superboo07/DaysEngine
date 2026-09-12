//! Naming a save: the comment dialog, drawn rather than opened.
//!
//! The original hands this to Windows. The save screen calls host `+0xdc`,
//! which posts `WM_USER` to the game window, and the window procedure opens a
//! modal dialog from the executable's own resources — see
//! [`crate::install::dialog`], which reads the template.
//!
//! We cannot open a Win32 dialog, so this draws one from that template: the
//! caption, the prompt, the button captions and every rectangle come out of
//! the player's executable, and only the colours are ours. What the player
//! types goes through SDL's text input, which is what carries an IME.
//!
//! # What the dialog does
//!
//! `FUN_0042e4a0` is its procedure, and the flow is short:
//!
//! - **OK** reads at most `0x79` bytes from edit control `0x40c`, widens them
//!   and calls `_CommentSet@4`, which stores the text on the save screen's
//!   module and sets its `+0x98`. That member is the same one the confirm
//!   popup sets, and the screen's next tick sees it and writes the slot.
//! - **Cancel** ends the dialog and calls nothing, so `+0x98` stays clear and
//!   **no save happens**.
//!
//! So the dialog is not a decoration on a save that has already been decided:
//! it is the confirmation.
//!
//! # The length limit
//!
//! `GetWindowTextA` into a 121-byte buffer, so 120 **bytes** in the game's ANSI
//! codepage — which is why the English prompt says 120 characters and the
//! Japanese one says 60. [`Comment::BUDGET`] is that limit, and [`cost`]
//! approximates the codepage's billing as one byte for ASCII and two for
//! everything else. That is an approximation of Shift-JIS, not an
//! implementation of it: it agrees with both prompts and with every character
//! the game's own font can draw.

use crate::install::dialog::{self, control, Class, Template};
use crate::playback::text;
use days_font::Font;
use days_ui::Image;

/// Colours for the drawn dialog.
///
/// These are ours. The original is a Windows dialog and took the player's own
/// theme, so there is nothing to recover: what is recovered is the layout.
mod paint {
    pub const FACE: [u8; 4] = [0x30, 0x34, 0x3c, 0xf2];
    pub const CAPTION: [u8; 4] = [0x1b, 0x33, 0x5c, 0xff];
    pub const FIELD: [u8; 4] = [0x12, 0x14, 0x18, 0xff];
    pub const EDGE: [u8; 4] = [0x8a, 0x94, 0xa4, 0xff];
    pub const BUTTON: [u8; 4] = [0x45, 0x4b, 0x57, 0xff];
    pub const BUTTON_ON: [u8; 4] = [0x2f, 0x5f, 0x9a, 0xff];
    pub const TEXT: [u8; 3] = [0xf0, 0xf2, 0xf5];
    pub const CARET: [u8; 4] = [0xf0, 0xf2, 0xf5, 0xff];
}

/// Which button the pointer or the keyboard is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Edit,
    Ok,
    Cancel,
}

/// What the editor asks for when a key or a click settles it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Act {
    /// Still editing.
    None,
    /// The player accepted: save with this comment.
    Accept(String),
    /// The player cancelled, which in the original means no save at all.
    Cancel,
}

/// The comment dialog, drawn from the player's own template.
#[derive(Debug, Clone)]
pub struct Comment {
    template: Template,
    /// What the player has typed, as characters.
    text: Vec<char>,
    /// Insertion point, in characters.
    caret: usize,
    focus: Focus,
    /// The composition an IME is still working on, which is shown but not yet
    /// part of the text.
    composing: String,
    english: bool,
}

impl Comment {
    /// The dialog procedure's own limit: 120 bytes of the ANSI text.
    pub const BUDGET: usize = 0x79 - 1;

    /// Opens the dialog for the install's language, seeded with the comment a
    /// slot already has.
    pub fn open(exe: &[u8], english: bool, initial: &str) -> Result<Comment, dialog::Error> {
        Comment::open_id(exe, dialog::comment_dialog(english), english, initial)
    }

    /// Opens a named dialog resource, for inspecting either language.
    pub fn open_id(
        exe: &[u8],
        id: u16,
        english: bool,
        initial: &str,
    ) -> Result<Comment, dialog::Error> {
        let template = Template::find(exe, id)?;
        let text: Vec<char> = initial.chars().collect();
        Ok(Comment {
            template,
            caret: text.len(),
            text,
            focus: Focus::Edit,
            composing: String::new(),
            english,
        })
    }

    /// The text as it stands.
    pub fn text(&self) -> String {
        self.text.iter().collect()
    }

    /// The template being drawn from, for anything that wants to report it.
    pub fn template(&self) -> &Template {
        &self.template
    }

    pub fn focus(&self) -> Focus {
        self.focus
    }

    /// Accepts typed text, as SDL's text input delivers it.
    ///
    /// Characters that would take the text past the dialog's budget are
    /// dropped, which is what the shipped read does at the other end: it takes
    /// the first 120 bytes and discards the rest.
    pub fn insert(&mut self, typed: &str) {
        self.composing.clear();
        for c in typed.chars() {
            if c.is_control() {
                continue;
            }
            if self.used() + cost(c) > Self::BUDGET {
                break;
            }
            self.text.insert(self.caret, c);
            self.caret += 1;
        }
    }

    /// An IME composition in progress: shown at the caret, not yet committed.
    pub fn compose(&mut self, text: &str) {
        self.composing = text.to_owned();
    }

    /// How many bytes of the budget the text uses.
    pub fn used(&self) -> usize {
        self.text.iter().copied().map(cost).sum()
    }

    pub fn backspace(&mut self) {
        if self.caret > 0 {
            self.caret -= 1;
            self.text.remove(self.caret);
        }
    }

    pub fn delete(&mut self) {
        if self.caret < self.text.len() {
            self.text.remove(self.caret);
        }
    }

    pub fn left(&mut self) {
        self.caret = self.caret.saturating_sub(1);
    }

    pub fn right(&mut self) {
        self.caret = (self.caret + 1).min(self.text.len());
    }

    pub fn home(&mut self) {
        self.caret = 0;
    }

    pub fn end(&mut self) {
        self.caret = self.text.len();
    }

    /// Moves the focus between the field and the two buttons.
    pub fn tab(&mut self, back: bool) {
        self.focus = match (self.focus, back) {
            (Focus::Edit, false) | (Focus::Cancel, true) => Focus::Ok,
            (Focus::Ok, false) | (Focus::Edit, true) => Focus::Cancel,
            (Focus::Cancel, false) | (Focus::Ok, true) => Focus::Edit,
        };
    }

    /// What Return does, which depends on where the focus is.
    pub fn enter(&self) -> Act {
        match self.focus {
            Focus::Cancel => Act::Cancel,
            _ => Act::Accept(self.text()),
        }
    }

    /// What Escape does.
    pub fn escape(&self) -> Act {
        Act::Cancel
    }

    /// Where a click lands, in the dialog's own pixel space.
    ///
    /// The controls are drawn below the caption bar, so the hit test has to
    /// shift by the same amount the drawing does or every button is off by the
    /// bar's height.
    pub fn click(&mut self, at: (i32, i32), base: (i32, i32)) -> Act {
        let caption = caption_height(base);
        for item in &self.template.items {
            let (x, y, cx, cy) = item.rect.to_pixels(base);
            let y = y + caption;
            let inside = at.0 >= x && at.0 < x + cx && at.1 >= y && at.1 < y + cy;
            if !inside {
                continue;
            }
            match item.id {
                control::OK => return Act::Accept(self.text()),
                control::CANCEL => return Act::Cancel,
                control::EDIT => self.focus = Focus::Edit,
                _ => {}
            }
        }
        Act::None
    }

    /// The dialog's size in pixels.
    pub fn size(&self, base: (i32, i32)) -> (u32, u32) {
        let (_, _, cx, cy) = self.template.rect.to_pixels(base);
        (cx.max(1) as u32, (cy + caption_height(base)).max(1) as u32)
    }

    /// Draws the dialog.
    ///
    /// Everything but the colours comes from the template: the frame is the
    /// dialog's own rectangle, each control sits where its record says, and the
    /// captions are the strings the resource carries.
    pub fn compose_image(&self, font: &Font, base: (i32, i32)) -> Image {
        let (w, h) = self.size(base);
        let mut img = Image::empty(w, h);
        let caption = caption_height(base);

        fill(&mut img, 0, 0, w as i32, h as i32, paint::FACE);
        fill(&mut img, 0, 0, w as i32, caption, paint::CAPTION);
        outline(&mut img, 0, 0, w as i32, h as i32, paint::EDGE);
        self.draw_text(&mut img, font, &self.template.title, 6, 2, base);

        for item in &self.template.items {
            let (x, y, cx, cy) = item.rect.to_pixels(base);
            let y = y + caption;
            match item.class {
                Class::Edit => {
                    fill(&mut img, x, y, cx, cy, paint::FIELD);
                    outline(&mut img, x, y, cx, cy, paint::EDGE);
                    self.draw_field(&mut img, font, (x, y, cx, cy), base);
                }
                Class::Button => {
                    let on = matches!(
                        (item.id, self.focus),
                        (control::OK, Focus::Ok) | (control::CANCEL, Focus::Cancel)
                    );
                    fill(
                        &mut img,
                        x,
                        y,
                        cx,
                        cy,
                        if on { paint::BUTTON_ON } else { paint::BUTTON },
                    );
                    outline(&mut img, x, y, cx, cy, paint::EDGE);
                    // A push button's caption is centred in both axes, which
                    // is what a Windows button does with it. The horizontal
                    // centre is measured on the glyphs' own ink, not on the
                    // sum of their advances: the last cell is wider than its
                    // advance and the line carries a trailing cell so the
                    // outline is not clipped, so advances alone lean left.
                    let (left, top, right, bottom) = self.ink_box(font, &item.text, base);
                    self.draw_text(
                        &mut img,
                        font,
                        &item.text,
                        x + (cx - (right - left)).max(0) / 2 - left,
                        y + (cy - (bottom - top)).max(0) / 2 - top,
                        base,
                    );
                }
                Class::Static => {
                    self.draw_text(&mut img, font, &item.text, x, y, base);
                }
                _ => {}
            }
        }
        img
    }

    /// The edit field: the text, the composition, and the caret.
    ///
    /// Left-aligned and vertically centred, which is what the template asks
    /// for: its style is `ES_AUTOHSCROLL` without `ES_CENTER`, so a real edit
    /// control puts the text against the left edge and centres it in the
    /// field's height. The vertical centre is measured on the glyphs' own ink
    /// rather than on the font's cell, which is `CELL` square with the letters
    /// sitting somewhere inside it.
    fn draw_field(
        &self,
        img: &mut Image,
        font: &Font,
        (x, y, _cx, cy): (i32, i32, i32, i32),
        base: (i32, i32),
    ) {
        const PAD: i32 = 3;
        let before: String = self.text[..self.caret].iter().collect();
        let after: String = self.text[self.caret..].iter().collect();
        let whole = format!("{before}{}{after}", self.composing);

        let height = glyph_height(base);
        let (left, ink_top, _, ink_bottom) = self.ink_box(font, &whole, base);
        let top = y + (cy - (ink_bottom - ink_top)).max(0) / 2 - ink_top;
        // Against the left edge, with the first glyph's own bearing taken off
        // so the run starts at the padding rather than a pixel or two in.
        let mut pen = x + PAD - left;

        pen += self.draw_text(img, font, &before, pen, top, base);
        if !self.composing.is_empty() {
            let width = self.draw_text(img, font, &self.composing, pen, top, base);
            // Underline what the IME has not committed, the way an edit
            // control shows a composition.
            fill(img, pen, top + ink_bottom, width, 1, paint::CARET);
            pen += width;
        } else {
            fill(
                img,
                pen,
                top + ink_top,
                1,
                (ink_bottom - ink_top).max(height / 2),
                paint::CARET,
            );
        }
        self.draw_text(img, font, &after, pen, top, base);
    }

    /// Draws a run of text scaled to the dialog's font size, returning its
    /// width.
    fn draw_text(
        &self,
        img: &mut Image,
        font: &Font,
        text: &str,
        x: i32,
        y: i32,
        base: (i32, i32),
    ) -> i32 {
        if text.is_empty() {
            return 0;
        }
        let line = self.render(font, text);
        let height = glyph_height(base);
        let scale = height as f32 / days_font::CELL as f32;
        let width = (line.width as f32 * scale) as i32;
        blit_scaled(img, &line, x, y, width, height);
        self.measure(text, base)
    }

    /// Rasterises a run of text.
    ///
    /// White, so the colour channels carry the raw luminance: the blit takes
    /// that as coverage and applies the real colour.
    fn render(&self, font: &Font, text: &str) -> text::TextImage {
        text::render_line_with(font, text, [0xff, 0xff, 0xff], &|c| {
            text::menu_advance(c, self.english)
        })
    }

    /// The box a run of text's inked pixels occupy once scaled, relative to
    /// the run's drawing origin.
    ///
    /// Centring needs this and not the advance box, on both axes. Horizontally
    /// the line carries a trailing cell so the last glyph's outline is not
    /// clipped, and the advances lean the run left. Vertically the font's cell
    /// is `CELL` square with the glyph sitting somewhere inside it, so
    /// centring the cell leaves the letters high.
    fn ink_box(&self, font: &Font, text: &str, base: (i32, i32)) -> (i32, i32, i32, i32) {
        let line = self.render(font, text);
        let scale = |v: usize| (v as i32) * glyph_height(base) / days_font::CELL as i32;
        let lit = |x: usize, y: usize| line.rgba[(y * line.width + x) * 4] != 0;
        let left = (0..line.width).find(|&x| (0..line.height).any(|y| lit(x, y)));
        let right = (0..line.width)
            .rev()
            .find(|&x| (0..line.height).any(|y| lit(x, y)));
        let top = (0..line.height).find(|&y| (0..line.width).any(|x| lit(x, y)));
        let bottom = (0..line.height)
            .rev()
            .find(|&y| (0..line.width).any(|x| lit(x, y)));
        match (left, right, top, bottom) {
            (Some(l), Some(r), Some(t), Some(b)) => {
                (scale(l), scale(t), scale(r + 1), scale(b + 1))
            }
            // A run with no ink at all -- an empty field, or only spaces.
            _ => (0, 0, self.measure(text, base), glyph_height(base)),
        }
    }

    /// How wide a run of text is once scaled.
    fn measure(&self, text: &str, base: (i32, i32)) -> i32 {
        let units: i32 = text
            .chars()
            .map(|c| text::menu_advance(c, self.english))
            .sum();
        units * glyph_height(base) / days_font::CELL as i32
    }
}

/// A character's cost against the dialog's byte budget.
///
/// An approximation of the ANSI codepage's billing, not an implementation of
/// it: one byte for ASCII, two for everything else. Shift-JIS bills the kana
/// and kanji this game uses at two bytes, which is why the English prompt says
/// 120 and the Japanese one says 60 for the same 120-byte buffer.
pub fn cost(c: char) -> usize {
    if c.is_ascii() {
        1
    } else {
        2
    }
}

/// The caption bar's height, which Windows takes from the system metrics
/// rather than the template. One line of the dialog's font plus its padding is
/// the closest this can get.
fn caption_height(base: (i32, i32)) -> i32 {
    base.1 + 4
}

/// How tall a glyph is drawn, so text fits the template's own rows.
fn glyph_height(base: (i32, i32)) -> i32 {
    base.1
}

/// The dialog base units for a font, as Windows measures them: the average
/// character width and the character height.
///
/// Measured from the font actually being drawn with, since `MS Shell Dlg` is
/// not available and its metrics were the player's machine's anyway.
pub fn base_units(english: bool) -> (i32, i32) {
    // The menu advance is flat for everything but the kerned Latin letters, so
    // the average over the alphabet is what Windows would call the average
    // character width.
    let letters: Vec<char> = ('A'..='Z').chain('a'..='z').collect();
    let total: i32 = letters
        .iter()
        .map(|&c| text::menu_advance(c, english))
        .sum();
    let average = total / letters.len() as i32;
    // Scaled down from the font's 48-pixel cell to something a dialog-sized
    // box can hold: the template is 280 x 62 units, and Windows' own base
    // units for an 8pt shell font are about 6 x 13.
    let scale = 4;
    (
        (average / scale).max(1),
        (days_font::CELL as i32 / scale).max(1),
    )
}

fn fill(img: &mut Image, x: i32, y: i32, w: i32, h: i32, colour: [u8; 4]) {
    for row in y.max(0)..(y + h).min(img.height as i32) {
        for col in x.max(0)..(x + w).min(img.width as i32) {
            let at = ((row as usize) * img.width as usize + col as usize) * 4;
            img.rgba[at..at + 4].copy_from_slice(&colour);
        }
    }
}

fn outline(img: &mut Image, x: i32, y: i32, w: i32, h: i32, colour: [u8; 4]) {
    fill(img, x, y, w, 1, colour);
    fill(img, x, y + h - 1, w, 1, colour);
    fill(img, x, y, 1, h, colour);
    fill(img, x + w - 1, y, 1, h, colour);
}

/// Draws a rendered line into the dialog, scaled down to `w` x `h`.
///
/// Two things differ from the way dialogue is drawn, both because this text is
/// a quarter of the size:
///
/// - **Coverage comes from the luminance plane, not the alpha plane.** The
///   alpha plane is dilated — it is the outline that keeps subtitles legible
///   over video — and at twelve pixels the outline is the whole glyph.
/// - **The downscale averages over the source area** rather than picking one
///   pixel out of sixteen, which is the difference between a readable letter
///   and a handful of specks.
fn blit_scaled(img: &mut Image, line: &text::TextImage, x: i32, y: i32, w: i32, h: i32) {
    if w <= 0 || h <= 0 || line.width == 0 || line.height == 0 {
        return;
    }
    for row in 0..h {
        let dy = y + row;
        if dy < 0 || dy >= img.height as i32 {
            continue;
        }
        let y0 = (row as usize * line.height) / h as usize;
        let y1 = (((row + 1) as usize * line.height) / h as usize).max(y0 + 1);
        for col in 0..w {
            let dx = x + col;
            if dx < 0 || dx >= img.width as i32 {
                continue;
            }
            let x0 = (col as usize * line.width) / w as usize;
            let x1 = (((col + 1) as usize * line.width) / w as usize).max(x0 + 1);

            let (mut sum, mut n) = (0u32, 0u32);
            for sy in y0..y1.min(line.height) {
                for sx in x0..x1.min(line.width) {
                    // The red channel is the luminance: `render_line_with`
                    // writes `colour * luminance / 255`, and the colour here is
                    // near white.
                    sum += u32::from(line.rgba[(sy * line.width + sx) * 4]);
                    n += 1;
                }
            }
            if n == 0 {
                continue;
            }
            let coverage = (sum / n).min(255) as u16;
            if coverage == 0 {
                continue;
            }
            let dst = (dy as usize * img.width as usize + dx as usize) * 4;
            for i in 0..3 {
                let over = u16::from(paint::TEXT[i]) * coverage;
                let under = u16::from(img.rgba[dst + i]) * (255 - coverage);
                img.rgba[dst + i] = ((over + under) / 255) as u8;
            }
            img.rgba[dst + 3] = img.rgba[dst + 3].max(coverage as u8);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blank() -> Comment {
        Comment {
            template: Template {
                id: dialog::COMMENT_EN,
                title: "Enter comment".into(),
                rect: dialog::Rect {
                    x: 0,
                    y: 0,
                    cx: 280,
                    cy: 62,
                },
                font: None,
                items: Vec::new(),
            },
            text: Vec::new(),
            caret: 0,
            focus: Focus::Edit,
            composing: String::new(),
            english: true,
        }
    }

    #[test]
    fn typing_lands_at_the_caret() {
        let mut c = blank();
        c.insert("save");
        assert_eq!(c.text(), "save");
        c.home();
        c.insert("a ");
        assert_eq!(c.text(), "a save");
        c.end();
        c.insert("!");
        assert_eq!(c.text(), "a save!");
    }

    #[test]
    fn backspace_and_delete_work_from_the_caret() {
        let mut c = blank();
        c.insert("abcd");
        c.left();
        c.backspace();
        assert_eq!(c.text(), "abd");
        c.delete();
        assert_eq!(c.text(), "ab");
        // Neither runs off the end.
        c.home();
        c.backspace();
        c.end();
        c.delete();
        assert_eq!(c.text(), "ab");
    }

    /// The dialog procedure reads 120 bytes, which is why the English prompt
    /// says 120 characters and the Japanese one says 60.
    #[test]
    fn the_budget_is_bytes_and_not_characters() {
        let mut c = blank();
        c.insert(&"a".repeat(200));
        assert_eq!(c.text().chars().count(), Comment::BUDGET);

        let mut c = blank();
        c.insert(&"あ".repeat(200));
        assert_eq!(c.text().chars().count(), Comment::BUDGET / 2);
        assert_eq!(c.used(), Comment::BUDGET);
    }

    #[test]
    fn a_character_that_would_not_fit_is_dropped_rather_than_split() {
        let mut c = blank();
        c.insert(&"a".repeat(Comment::BUDGET - 1));
        // One byte left, and a two-byte character does not take it.
        c.insert("あ");
        assert_eq!(c.used(), Comment::BUDGET - 1);
        c.insert("z");
        assert_eq!(c.used(), Comment::BUDGET);
    }

    /// Cancelling is not "save with the old comment": the shipped dialog only
    /// calls `_CommentSet@4` on OK, and that call is what makes the save
    /// happen at all.
    #[test]
    fn escape_cancels_and_return_accepts() {
        let mut c = blank();
        c.insert("mine");
        assert_eq!(c.escape(), Act::Cancel);
        assert_eq!(c.enter(), Act::Accept("mine".into()));
        c.focus = Focus::Cancel;
        assert_eq!(c.enter(), Act::Cancel);
    }

    #[test]
    fn tab_cycles_the_field_and_the_two_buttons() {
        let mut c = blank();
        assert_eq!(c.focus(), Focus::Edit);
        c.tab(false);
        assert_eq!(c.focus(), Focus::Ok);
        c.tab(false);
        assert_eq!(c.focus(), Focus::Cancel);
        c.tab(false);
        assert_eq!(c.focus(), Focus::Edit);
        c.tab(true);
        assert_eq!(c.focus(), Focus::Cancel);
    }

    /// A composition is shown but is not part of the text until the IME
    /// commits it, which arrives as ordinary text input.
    #[test]
    fn an_ime_composition_is_not_committed_text() {
        let mut c = blank();
        c.compose("せか");
        assert_eq!(c.text(), "");
        c.insert("世界");
        assert_eq!(c.text(), "世界");
        assert!(c.composing.is_empty());
    }

    #[test]
    fn a_click_on_a_button_answers_for_it() {
        let mut c = blank();
        c.template.items = vec![
            dialog::Item {
                id: control::OK,
                class: Class::Button,
                rect: dialog::Rect {
                    x: 168,
                    y: 41,
                    cx: 50,
                    cy: 14,
                },
                style: 0,
                text: "OK".into(),
            },
            dialog::Item {
                id: control::CANCEL,
                class: Class::Button,
                rect: dialog::Rect {
                    x: 223,
                    y: 41,
                    cx: 50,
                    cy: 14,
                },
                style: 0,
                text: "Cancel".into(),
            },
        ];
        let base = (6, 13);
        // Where the dialog actually draws them: below the caption bar.
        let caption = caption_height(base);
        let hit = |c: &mut Comment, i: usize| {
            let (x, y, cx, cy) = c.template.items[i].rect.to_pixels(base);
            c.click((x + cx / 2, y + caption + cy / 2), base)
        };
        assert_eq!(hit(&mut c, 0), Act::Accept(String::new()));
        assert_eq!(hit(&mut c, 1), Act::Cancel);
        // Outside every control, nothing happens -- including the strip the
        // caption bar occupies, which is above every one of them.
        assert_eq!(c.click((0, 0), base), Act::None);
        let (x, y, cx, _) = c.template.items[0].rect.to_pixels(base);
        assert_eq!(c.click((x + cx / 2, y), base), Act::None);
    }

    #[test]
    fn ascii_costs_one_byte_and_everything_else_two() {
        assert_eq!(cost('a'), 1);
        assert_eq!(cost('あ'), 2);
        assert_eq!(cost('界'), 2);
    }
}
