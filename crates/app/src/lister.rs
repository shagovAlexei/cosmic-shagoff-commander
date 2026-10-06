//! F3 viewer (TC's Lister) in place of the panels: state and view; logic in `core::lister`.

use crate::app::Message;
use crate::fl;
use crate::keymap::Action;
use cosmic::iced::widget::text::Wrapping;
use cosmic::iced::{ContentFit, Length};
use cosmic::widget::{self, column, row};
use cosmic::{Element, theme};
use shagoff_core::lister::{self, Doc, Encoding, Mode, Text};
use shagoff_core::viewport;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Fixed row height: the scroll math depends on it.
pub const ROW_H: f32 = 20.0;
/// Monospace advance at the default text size.
const MONO_W: f32 = 8.5;
/// ← / →: this many chars.
const STEP_X: f32 = MONO_W * 8.0;
const HEX_COLS: usize = 77;
/// Encodings in the order of the drop-down (code page names, not translated).
pub const ENCODINGS: [(Encoding, &str); 4] = [
    (Encoding::Auto, "UTF-8"),
    (Encoding::Cp1251, "Windows-1251"),
    (Encoding::Koi8r, "KOI8-R"),
    (Encoding::Cp866, "DOS 866"),
];
const ENCODING_NAMES: [&str; 4] = ["UTF-8", "Windows-1251", "KOI8-R", "DOS 866"];

#[derive(Debug)]
pub enum Image {
    Raster(widget::image::Handle),
    Svg(widget::svg::Handle),
}

/// A file read for viewing (built off the UI thread).
#[derive(Debug)]
pub struct Loaded {
    pub doc: Doc,
    /// Text mode as opened: UTF-8 / BOM, not wrapped.
    pub text: Text,
    /// Handles are made once: a new handle per frame would decode the image every frame.
    pub image: Option<Image>,
}

impl Loaded {
    pub fn read(path: &Path, name: &str) -> Result<Self, String> {
        let doc = lister::load(path).map_err(|e| e.to_string())?;
        let text = Text::new(&doc, Encoding::Auto, None);
        let image = lister::is_image(name).then(|| {
            if name.to_ascii_lowercase().ends_with(".svg") {
                Image::Svg(widget::svg::Handle::from_memory(doc.bytes.clone()))
            } else {
                Image::Raster(widget::image::Handle::from_bytes(doc.bytes.clone()))
            }
        });
        Ok(Self { doc, text, image })
    }
}

pub struct Lister {
    /// Pane whose file is shown (N / P walk its list).
    pub side: usize,
    pub name: String,
    /// Results of other loads are dropped.
    pub id: u64,
    /// `None` while reading.
    pub loaded: Option<Arc<Result<Loaded, String>>>,
    pub mode: Mode,
    /// Scroll offset (x, y) and viewport height.
    pub offset: (f32, f32),
    pub height: f32,
    pub width: f32,
    /// A / S / K / 8 and `W`: kept when N / P open the next file.
    pub encoding: Encoding,
    /// The encoding was picked by hand: kept for the next file; a guessed one is guessed again.
    chosen: bool,
    pub wrap: bool,
    /// Text for another encoding or wrapping, rebuilt from the file's bytes; `None` = `Loaded::text`.
    // ponytail: rebuilt on the UI thread (a 32 MB file takes a moment); spawn_blocking if it shows.
    text: Option<Text>,
    /// Row of the last search match (highlighted).
    pub hit: Option<usize>,
    pub query: String,
    /// The F7 field is shown.
    pub searching: bool,
    pub scroll: widget::Id,
    pub input: widget::Id,
    /// Mouse selection (TC Lister): dragged, Shift+click extends, Ctrl+A all, Ctrl+C copies.
    pub sel: Option<lister::Selection>,
    /// The button is down over the text: moves extend the selection.
    pub dragging: bool,
    /// Last pointer position over the text, in viewport pixels; `None` until a move over it
    /// (the move that enters the area is not reported, so a quick press would start at a stale
    /// spot: then the first move while pressed sets where the selection starts).
    pub pointer: Option<(f32, f32)>,
    /// When and on which row the last double click was: a press soon after there selects the row.
    pub double: Option<(std::time::Instant, usize)>,
    /// Ctrl+Q: shown in place of the other pane, keys stay with the panels; what is shown — path,
    /// size, mtime (a file changed on disk is shown again; a dir or `..`: a note instead of
    /// contents). `None`: the full-window F3 viewer.
    pub quick: Option<(PathBuf, u64, std::time::SystemTime)>,
}

impl Lister {
    pub fn new(side: usize, name: String, id: u64) -> Self {
        Self {
            side,
            name,
            id,
            loaded: None,
            mode: Mode::Text,
            offset: (0.0, 0.0),
            height: 400.0,
            width: 800.0,
            encoding: Encoding::Auto,
            chosen: false,
            wrap: false,
            text: None,
            hit: None,
            sel: None,
            dragging: false,
            pointer: None,
            double: None,
            quick: None,
            query: String::new(),
            searching: false,
            scroll: widget::Id::unique(),
            input: widget::Id::unique(),
        }
    }

    /// The next file in the same lister: search text kept, the rest reset.
    pub fn reopen(&mut self, name: String, id: u64) {
        let query = std::mem::take(&mut self.query);
        let (scroll, input, height) = (self.scroll.clone(), self.input.clone(), self.height);
        let (width, chosen, wrap) = (self.width, self.chosen, self.wrap);
        let encoding = if chosen {
            self.encoding
        } else {
            Encoding::Auto
        };
        *self = Self {
            query,
            scroll,
            input,
            height,
            width,
            encoding,
            chosen,
            wrap,
            ..Self::new(self.side, name, id)
        };
    }

    pub fn ok(&self) -> Option<&Loaded> {
        self.loaded.as_deref().and_then(|r| r.as_ref().ok())
    }

    pub fn set_loaded(&mut self, loaded: Arc<Result<Loaded, String>>) {
        if let Ok(l) = loaded.as_ref() {
            self.mode = lister::detect(&self.name, l.doc.text());
            // Not UTF-8: guess the Cyrillic code page, unless one was chosen (A / S / K).
            if self.mode == Mode::Text && !self.chosen {
                self.encoding = lister::guess(&l.doc.bytes);
            }
        }
        self.loaded = Some(loaded);
        self.retext();
    }

    /// Doc and the text mode content in the current encoding / wrapping.
    pub fn text(&self) -> Option<(&Doc, &Text)> {
        let l = self.ok()?;
        Some((&l.doc, self.text.as_ref().unwrap_or(&l.text)))
    }

    /// Chars per wrapped line: the viewport minus padding and the scrollbar.
    fn wrap_cols(&self) -> usize {
        ((self.width - 32.0).max(0.0) / MONO_W) as usize
    }

    fn retext(&mut self) {
        self.text = match self.ok() {
            Some(l) if self.encoding != Encoding::Auto || self.wrap => Some(Text::new(
                &l.doc,
                self.encoding,
                self.wrap.then(|| self.wrap_cols()),
            )),
            _ => None,
        };
        self.hit = None;
        self.sel = None; // rows changed under it
    }

    /// A / S / K / 8: show the text in `enc` (switches hex to text, as TC does).
    pub fn set_encoding(&mut self, enc: Encoding) {
        self.encoding = enc;
        self.chosen = true;
        self.retext();
        self.set_mode(Mode::Text);
    }

    /// `W`: wrap long lines at the window width.
    pub fn toggle_wrap(&mut self) {
        self.wrap = !self.wrap;
        self.retext();
        self.set_mode(Mode::Text);
    }

    /// New viewport size; wrapped text is rewrapped when a char more or less fits.
    pub fn resized(&mut self, width: f32, height: f32) {
        let before = self.wrap_cols();
        (self.width, self.height) = (width, height);
        if self.wrap && self.wrap_cols() != before {
            self.retext();
        }
    }

    /// `4` on a file that is not an image does nothing.
    pub fn set_mode(&mut self, mode: Mode) {
        // While reading: the mode detected on arrival would undo it.
        if self.loaded.is_none() {
            return;
        }
        if mode != Mode::Image || self.ok().is_some_and(|l| l.image.is_some()) {
            self.mode = mode;
            self.hit = None;
            self.sel = None;
        }
    }

    pub fn rows(&self) -> usize {
        match (self.ok(), self.mode) {
            (Some(_), Mode::Text) => self.text().map_or(0, |(_, t)| t.lines.len()),
            (Some(l), Mode::Hex) => lister::hex_rows(l.doc.bytes.len()),
            _ => 0,
        }
    }

    fn top(&self) -> usize {
        (self.offset.1 / ROW_H) as usize
    }

    /// Where a scroll key moves the view to (y), `None` for other keys.
    pub fn scroll_y(&self, action: Action) -> Option<f32> {
        let max = (self.rows() as f32 * ROW_H - self.height).max(0.0);
        let page = viewport::page_rows(ROW_H, self.height) as f32 * ROW_H;
        let y = self.offset.1;
        let to = match action {
            Action::Up => y - ROW_H,
            Action::Down => y + ROW_H,
            Action::PageUp => y - page,
            Action::PageDown => y + page,
            Action::Home => 0.0,
            Action::End => max,
            _ => return None,
        };
        Some(to.clamp(0.0, max))
    }

    pub fn scroll_x(&self, right: bool) -> f32 {
        let x = self.offset.0 + if right { STEP_X } else { -STEP_X };
        x.clamp(0.0, (self.width() - STEP_X).max(0.0))
    }

    fn width(&self) -> f32 {
        let cols = match self.mode {
            Mode::Hex => HEX_COLS,
            _ => self.text().map_or(0, |(_, t)| t.cols),
        };
        cols as f32 * MONO_W + 16.0
    }

    /// Search: from the top visible row (`again`: from the row after / before the last match).
    /// Returns the y to scroll to on a match.
    pub fn find(&mut self, forward: bool, again: bool) -> Option<f32> {
        let start = match (again, self.hit) {
            (true, Some(h)) if forward => h + 1,
            (true, Some(h)) => h.checked_sub(1)?,
            _ => self.top(),
        };
        let (doc, text) = self.text()?;
        let row = match self.mode {
            Mode::Text => {
                lister::find_line(text.bytes(doc), &text.lines, &self.query, start, forward)
            }
            Mode::Hex => lister::find_bytes(&doc.bytes, self.query.as_bytes(), start, forward),
            Mode::Image => None,
        }?;
        self.hit = Some(row);
        // A couple of rows of context above the match.
        Some(row.saturating_sub(2) as f32 * ROW_H)
    }

    /// The text position under the pointer (the view's scroll offset added).
    pub fn pointer_pos(&self) -> Option<lister::Pos> {
        let (x, y) = self.pointer?;
        Some(lister::pos_at(
            x + self.offset.0,
            y + self.offset.1,
            ROW_H,
            MONO_W,
            self.rows(),
        ))
    }

    /// Ctrl+A: every row.
    pub fn select_all(&mut self) {
        let Some((doc, txt)) = self.text() else {
            return;
        };
        let last = self.rows().saturating_sub(1);
        let len = if self.rows() == 0 {
            0
        } else {
            self.row_text(doc, txt, last).chars().count()
        };
        self.sel = Some(lister::Selection {
            anchor: (0, 0),
            head: (last, len),
        });
    }

    /// Ctrl+C: the selected text, rows as shown (a wrapped line's parts each on a line).
    // ponytail: wrapped rows are joined with newlines too; join a line's parts if it matters.
    pub fn selected_text(&self) -> Option<String> {
        let sel = self.sel.filter(|s| !s.is_empty())?;
        let (doc, txt) = self.text()?;
        Some(sel.text(|i| self.row_text(doc, txt, i)))
    }

    /// Double click: the word under the pointer (`whole`: the whole row, for a triple click).
    pub fn select_at_pointer(&mut self, whole: bool) {
        let (Some((row, col)), Some((doc, txt))) = (self.pointer_pos(), self.text()) else {
            return;
        };
        if row >= self.rows() {
            return;
        }
        let line = self.row_text(doc, txt, row);
        let cols = if whole {
            Some(0..line.chars().count())
        } else {
            lister::word_at(&line, col)
        };
        self.sel = cols.map(|c| lister::Selection {
            anchor: (row, c.start),
            head: (row, c.end),
        });
    }

    fn row_text(&self, doc: &Doc, text: &Text, i: usize) -> String {
        match self.mode {
            Mode::Hex => lister::hex_row(&doc.bytes, i),
            _ => lister::line_text(text.bytes(doc), &text.lines[i]),
        }
    }
}

fn mode_button(label: String, mode: Mode, current: Mode) -> Element<'static, Message> {
    let b = if mode == current {
        widget::button::suggested(label)
    } else {
        widget::button::standard(label)
    };
    b.on_press(Message::ListerMode(mode)).into()
}

pub fn view(l: &Lister) -> Element<'_, Message> {
    let mut modes = vec![
        mode_button(fl!("lister-text"), Mode::Text, l.mode),
        mode_button(fl!("lister-hex"), Mode::Hex, l.mode),
    ];
    if l.ok().is_some_and(|x| x.image.is_some()) {
        modes.push(mode_button(fl!("lister-image"), Mode::Image, l.mode));
    }
    let info = match l.loaded.as_deref() {
        None => fl!("lister-loading"),
        Some(Err(e)) => e.clone(),
        Some(Ok(x)) => match l.mode {
            Mode::Text => fl!(
                "lister-info-lines",
                n = l.rows(),
                size = shagoff_core::format::size(x.doc.total)
            ),
            _ => fl!(
                "lister-info",
                size = shagoff_core::format::size(x.doc.total)
            ),
        },
    };
    // Name and info on their own line, buttons wrapping below: one row did not fit a narrow window.
    let title = row::with_children(vec![
        widget::text::heading(l.name.clone())
            .wrapping(Wrapping::WordOrGlyph)
            .width(Length::Fill)
            .into(),
        widget::text::body(info).into(),
    ])
    .spacing(8)
    .align_y(cosmic::iced::Alignment::Center);
    let mut buttons = modes;
    let enc = ENCODINGS.iter().position(|(e, _)| *e == l.encoding);
    buttons.extend([
        widget::dropdown(&ENCODING_NAMES, enc, |i| {
            Message::ListerEncoding(ENCODINGS[i].0)
        })
        .into(),
        if l.wrap {
            widget::button::suggested(fl!("lister-wrap"))
        } else {
            widget::button::standard(fl!("lister-wrap"))
        }
        .on_press(Message::ListerWrap)
        .into(),
    ]);
    // Ctrl+Q: the keys are the panels'; search, N / P and Esc belong to the F3 viewer.
    if l.quick.is_none() {
        buttons.extend([
            widget::button::standard(fl!("lister-find"))
                .on_press(Message::ListerSearch)
                .into(),
            widget::button::standard(fl!("lister-prev"))
                .on_press(Message::ListerStep(false))
                .into(),
            widget::button::standard(fl!("lister-next"))
                .on_press(Message::ListerStep(true))
                .into(),
            widget::button::standard(fl!("lister-close"))
                .on_press(Message::ListerClose)
                .into(),
        ]);
    }
    // Ctrl+Q on a dir or `..`: just the name and the note, no view modes to pick.
    let bar = if l.quick.is_some() && l.ok().is_none() {
        column::with_children(vec![title.into()])
    } else {
        column::with_children(vec![
            title.into(),
            widget::flex_row(buttons).spacing(4).into(),
        ])
    }
    .spacing(4);
    let mut col = column::with_capacity(4).spacing(6).push(bar);
    if l.searching {
        col = col.push(
            widget::text_input(fl!("lister-find-hint"), &l.query)
                .id(l.input.clone())
                .on_input(Message::ListerQuery)
                .on_submit(|_| Message::ListerFind),
        );
    }
    if let Some(x) = l.ok()
        && x.doc.truncated()
    {
        col = col.push(widget::text::caption(fl!(
            "lister-truncated",
            limit = (lister::LIMIT >> 20).to_string(),
            size = shagoff_core::format::size(x.doc.total)
        )));
    }
    col.push(content(l)).padding([4, 8]).into()
}

fn content(l: &Lister) -> Element<'_, Message> {
    let (Some(x), Some((doc, txt))) = (l.ok(), l.text()) else {
        return widget::Space::new().into();
    };
    if l.mode == Mode::Image {
        let img: Element<'_, Message> = match &x.image {
            Some(Image::Raster(h)) => widget::image(h.clone())
                .content_fit(ContentFit::ScaleDown)
                .into(),
            Some(Image::Svg(h)) => widget::svg(h.clone())
                .content_fit(ContentFit::Contain)
                .into(),
            None => widget::Space::new().into(),
        };
        return widget::container(img).center(Length::Fill).into();
    }
    // Only the rows in view are built; spacers keep the scroll height.
    let rows = l.rows();
    let range = viewport::visible_range(rows, ROW_H, l.offset.1, l.height);
    let width = Length::Fixed(l.width());
    let mut list = column::with_capacity(range.len() + 2).push(
        widget::Space::new()
            .width(width)
            .height(Length::Fixed(range.start as f32 * ROW_H)),
    );
    let mono = |s: String| {
        widget::text(s)
            .font(cosmic::font::mono())
            .wrapping(Wrapping::None)
    };
    for i in range.clone() {
        let line = l.row_text(doc, txt, i);
        let picked = l.sel.and_then(|s| s.in_row(i, line.chars().count()));
        let text: Element<'_, Message> = match picked {
            Some(c) => {
                // Char columns to byte offsets.
                let byte = |n: usize| line.char_indices().nth(n).map_or(line.len(), |(b, _)| b);
                let (a, b) = (byte(c.start), byte(c.end));
                row![
                    mono(line[..a].to_string()),
                    widget::container(mono(line[a..b].to_string())).class(hit_style(true)),
                    mono(line[b..].to_string()),
                ]
                .into()
            }
            None => mono(line).into(),
        };
        let hit = l.hit == Some(i);
        list = list.push(
            widget::container(text)
                .width(width)
                .height(Length::Fixed(ROW_H))
                .class(hit_style(hit)),
        );
    }
    list = list.push(
        widget::Space::new()
            .width(Length::Fixed(1.0))
            .height(Length::Fixed((rows - range.end) as f32 * ROW_H)),
    );
    let list = widget::scrollable(list)
        .id(l.scroll.clone())
        .on_scroll(|v| {
            let o = v.absolute_offset();
            Message::ListerScrolled(o.x, o.y, v.bounds().height)
        })
        .direction(cosmic::iced::widget::scrollable::Direction::Both {
            vertical: Default::default(),
            horizontal: Default::default(),
        })
        .width(Length::Fill)
        .height(Length::Fill);
    // on_scroll misses window resizes.
    let list = cosmic::iced::widget::sensor(list)
        .on_show(Message::ListerResized)
        .on_resize(Message::ListerResized);
    // Selecting with the mouse: the pointer is tracked over the text, the button starts / ends.
    widget::mouse_area(list)
        .interaction(cosmic::iced::mouse::Interaction::Text)
        .on_move(|p| Message::ListerPointer(p.x, p.y))
        .on_exit(Message::ListerPointerLeft)
        .on_press(Message::ListerPress)
        .on_release(Message::ListerRelease)
        .on_double_click(Message::ListerDoubleClick)
        .into()
}

fn hit_style(hit: bool) -> theme::Container<'static> {
    theme::Container::custom(move |t| {
        let c = t.cosmic();
        if hit {
            widget::container::Style {
                background: Some(cosmic::iced::Color::from(c.accent_color()).into()),
                text_color: Some(c.on_accent_color().into()),
                ..Default::default()
            }
        } else {
            widget::container::Style::default()
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> Lister {
        let mut l = Lister::new(0, "a.txt".into(), 1);
        let doc = Doc {
            bytes: s.as_bytes().to_vec(),
            decoded: None,
            total: s.len() as u64,
        };
        let text = Text::new(&doc, Encoding::Auto, None);
        l.set_loaded(Arc::new(Ok(Loaded {
            doc,
            text,
            image: None,
        })));
        l
    }

    #[test]
    fn select_all_and_copy_with_the_mouse() {
        let mut l = text("one\ntwo\nthree");
        assert_eq!(l.selected_text(), None);
        l.select_all();
        assert_eq!(l.selected_text().as_deref(), Some("one\ntwo\nthree"));
        // drag from "ne" on row 0 to "tw" on row 1 (pointer in px, 8.5 px chars, 20 px rows)
        l.pointer = Some((8.5, 5.0));
        let start = l.pointer_pos().unwrap();
        l.pointer = Some((17.0, 25.0));
        l.sel = Some(lister::Selection {
            anchor: start,
            head: l.pointer_pos().unwrap(),
        });
        assert_eq!(l.selected_text().as_deref(), Some("ne\ntw"));
        l.toggle_wrap(); // rows rebuilt: the selection would point elsewhere
        assert_eq!(l.sel, None);
    }

    #[test]
    fn double_click_selects_the_word_triple_the_row() {
        let mut l = text("let first_row = 1;");
        l.pointer = Some((8.5 * 6.0, 5.0)); // over "first_row"
        l.select_at_pointer(false);
        assert_eq!(l.selected_text().as_deref(), Some("first_row"));
        l.select_at_pointer(true);
        assert_eq!(l.selected_text().as_deref(), Some("let first_row = 1;"));
    }

    #[test]
    fn scroll_keys_clamp() {
        let mut l = text(&"line\n".repeat(100));
        l.height = 10.0 * ROW_H;
        assert_eq!(l.scroll_y(Action::Up), Some(0.0));
        assert_eq!(l.scroll_y(Action::Down), Some(ROW_H));
        assert_eq!(l.scroll_y(Action::End), Some(90.0 * ROW_H));
        l.offset.1 = 90.0 * ROW_H;
        assert_eq!(l.scroll_y(Action::PageDown), Some(90.0 * ROW_H));
        assert_eq!(l.scroll_y(Action::PageUp), Some(81.0 * ROW_H));
        assert_eq!(l.scroll_y(Action::Copy), None);
    }

    #[test]
    fn find_then_next_and_previous() {
        let mut l = text("a\nfoo\nb\nFOO\nc\nfoo\n");
        l.query = "foo".into();
        assert_eq!(l.find(true, false), Some(0.0)); // row 1, two rows of context → 0
        assert_eq!(l.hit, Some(1));
        l.find(true, true);
        assert_eq!(l.hit, Some(3));
        l.find(true, true);
        assert_eq!(l.hit, Some(5));
        assert_eq!(l.find(true, true), None); // no more: the last match stays
        assert_eq!(l.hit, Some(5));
        l.find(false, true);
        assert_eq!(l.hit, Some(3));
    }

    #[test]
    fn encoding_and_wrap_rebuild_the_text_and_survive_the_next_file() {
        let mut l = text("abcdefghij\n\u{0}");
        l.set_mode(Mode::Hex);
        l.set_encoding(Encoding::Cp1251);
        assert_eq!(l.mode, Mode::Text); // TC: an encoding key shows text
        l.width = 32.0 + 4.0 * MONO_W; // 4 chars
        l.toggle_wrap();
        assert_eq!(l.rows(), 4); // abcd efgh ij + NUL line
        l.resized(32.0 + 5.0 * MONO_W, 100.0);
        assert_eq!(l.rows(), 3); // abcde fghij, NUL
        l.reopen("b.txt".into(), 2);
        assert_eq!((l.encoding, l.wrap), (Encoding::Cp1251, true));
        l.toggle_wrap();
        l.set_encoding(Encoding::Auto);
        assert!(l.text.is_none());
    }

    #[test]
    fn a_cp1251_file_opens_decoded() {
        let mut l = Lister::new(0, "a.txt".into(), 1);
        let doc = Doc {
            bytes: b"\xcf\xf0\xe8\xe2\xe5\xf2".to_vec(), // "Привет" in 1251
            decoded: None,
            total: 6,
        };
        let text = Text::new(&doc, Encoding::Auto, None);
        l.set_loaded(Arc::new(Ok(Loaded {
            doc,
            text,
            image: None,
        })));
        assert_eq!(l.encoding, Encoding::Cp1251);
        let (doc, t) = l.text().unwrap();
        assert_eq!(lister::line_text(t.bytes(doc), &t.lines[0]), "Привет");
        // a guess is not carried to the next file; a choice is
        l.reopen("b.txt".into(), 2);
        assert_eq!(l.encoding, Encoding::Auto);
        l.set_encoding(Encoding::Koi8r);
        l.reopen("c.txt".into(), 3);
        assert_eq!(l.encoding, Encoding::Koi8r);
    }

    #[test]
    fn mode_keys_wait_for_the_file() {
        let mut l = Lister::new(0, "a.bin".into(), 1);
        l.set_mode(Mode::Hex); // still reading: the detected mode would undo it
        assert_eq!(l.mode, Mode::Text);
    }

    #[test]
    fn image_mode_only_for_images() {
        let mut l = text("hi");
        assert_eq!(l.mode, Mode::Text);
        l.set_mode(Mode::Image);
        assert_eq!(l.mode, Mode::Text);
        l.set_mode(Mode::Hex);
        assert_eq!(l.mode, Mode::Hex);
        assert_eq!(l.rows(), 1);
    }
}
