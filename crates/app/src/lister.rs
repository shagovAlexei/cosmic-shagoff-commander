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
use std::path::Path;
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
            wrap: false,
            text: None,
            hit: None,
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
        let (width, encoding, wrap) = (self.width, self.encoding, self.wrap);
        *self = Self {
            query,
            scroll,
            input,
            height,
            width,
            encoding,
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
    }

    /// A / S / K / 8: show the text in `enc` (switches hex to text, as TC does).
    pub fn set_encoding(&mut self, enc: Encoding) {
        self.encoding = enc;
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
    let bar = column::with_children(vec![
        title.into(),
        widget::flex_row(buttons).spacing(4).into(),
    ])
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
    for i in range.clone() {
        let text = widget::text(l.row_text(doc, txt, i))
            .font(cosmic::font::mono())
            .wrapping(Wrapping::None);
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
    cosmic::iced::widget::sensor(list)
        .on_show(Message::ListerResized)
        .on_resize(Message::ListerResized)
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
