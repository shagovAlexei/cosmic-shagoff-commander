//! F3 viewer (TC's Lister) in place of the panels: state and view; logic in `core::lister`.

use crate::app::Message;
use crate::fl;
use crate::keymap::Action;
use cosmic::iced::widget::text::Wrapping;
use cosmic::iced::{ContentFit, Length};
use cosmic::widget::{self, column, row};
use cosmic::{Element, theme};
use shagoff_core::lister::{self, Doc, Mode};
use shagoff_core::viewport;
use std::ops::Range;
use std::path::Path;
use std::sync::Arc;

/// Fixed row height: the scroll math depends on it.
pub const ROW_H: f32 = 20.0;
/// Monospace advance at the default text size.
const MONO_W: f32 = 8.5;
/// ← / →: this many chars.
const STEP_X: f32 = MONO_W * 8.0;
const HEX_COLS: usize = 77;

#[derive(Debug)]
pub enum Image {
    Raster(widget::image::Handle),
    Svg(widget::svg::Handle),
}

/// A file read for viewing (built off the UI thread).
#[derive(Debug)]
pub struct Loaded {
    pub doc: Doc,
    pub lines: Vec<Range<usize>>,
    /// Longest line, in chars: the width of the scrolled text.
    pub cols: usize,
    /// Handles are made once: a new handle per frame would decode the image every frame.
    pub image: Option<Image>,
}

impl Loaded {
    pub fn read(path: &Path, name: &str) -> Result<Self, String> {
        let doc = lister::load(path).map_err(|e| e.to_string())?;
        let lines = lister::lines(&doc.bytes);
        let cols = lines
            .iter()
            .map(|r| lister::width(&doc.bytes, r))
            .max()
            .unwrap_or(0);
        let image = lister::is_image(name).then(|| {
            if name.to_ascii_lowercase().ends_with(".svg") {
                Image::Svg(widget::svg::Handle::from_memory(doc.bytes.clone()))
            } else {
                Image::Raster(widget::image::Handle::from_bytes(doc.bytes.clone()))
            }
        });
        Ok(Self {
            doc,
            lines,
            cols,
            image,
        })
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
        *self = Self {
            query,
            scroll,
            input,
            height,
            ..Self::new(self.side, name, id)
        };
    }

    pub fn ok(&self) -> Option<&Loaded> {
        self.loaded.as_deref().and_then(|r| r.as_ref().ok())
    }

    pub fn set_loaded(&mut self, loaded: Arc<Result<Loaded, String>>) {
        if let Ok(l) = loaded.as_ref() {
            self.mode = lister::detect(&self.name, &l.doc.bytes);
        }
        self.loaded = Some(loaded);
    }

    /// `4` on a file that is not an image does nothing.
    pub fn set_mode(&mut self, mode: Mode) {
        if mode != Mode::Image || self.ok().is_some_and(|l| l.image.is_some()) {
            self.mode = mode;
            self.hit = None;
        }
    }

    pub fn rows(&self) -> usize {
        match (self.ok(), self.mode) {
            (Some(l), Mode::Text) => l.lines.len(),
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
            _ => self.ok().map_or(0, |l| l.cols),
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
        let l = self.ok()?;
        let row = match self.mode {
            Mode::Text => lister::find_line(&l.doc.bytes, &l.lines, &self.query, start, forward),
            Mode::Hex => lister::find_bytes(&l.doc.bytes, self.query.as_bytes(), start, forward),
            Mode::Image => None,
        }?;
        self.hit = Some(row);
        // A couple of rows of context above the match.
        Some(row.saturating_sub(2) as f32 * ROW_H)
    }

    fn row_text(&self, l: &Loaded, i: usize) -> String {
        match self.mode {
            Mode::Hex => lister::hex_row(&l.doc.bytes, i),
            _ => lister::line_text(&l.doc.bytes, &l.lines[i]),
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
                n = x.lines.len(),
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
    let Some(x) = l.ok() else {
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
        let text = widget::text(l.row_text(x, i))
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
        .on_show(|s| Message::ListerResized(s.height))
        .on_resize(|s| Message::ListerResized(s.height))
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
            total: s.len() as u64,
        };
        let lines = lister::lines(&doc.bytes);
        l.set_loaded(Arc::new(Ok(Loaded {
            doc,
            lines,
            cols: 10,
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
