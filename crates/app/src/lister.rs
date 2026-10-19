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

/// Row height at the default text size: the scroll math depends on it.
const ROW_H: f32 = 20.0;
/// Monospace advance at the default text size.
const MONO_W: f32 = 8.5;
const HEX_COLS: usize = 77;
/// Encodings in the order of the drop-down (code page names, not translated): ours with keys,
/// then the others `lister::guess` can find, and a few to pick by hand.
pub static ENCODINGS: std::sync::LazyLock<Vec<(Encoding, &'static str)>> =
    std::sync::LazyLock::new(|| {
        use shagoff_core::lister::encoding_rs as e;
        let ours = [
            (Encoding::Auto, "UTF-8"),
            (Encoding::Cp1251, "Windows-1251"),
            (Encoding::Koi8r, "KOI8-R"),
            (Encoding::Cp866, "DOS 866"),
        ];
        let others = [
            e::WINDOWS_1252,
            e::WINDOWS_1250,
            e::ISO_8859_2,
            e::WINDOWS_1253,
            e::ISO_8859_7,
            e::WINDOWS_1254,
            e::WINDOWS_1257,
            e::ISO_8859_4,
            e::ISO_8859_13,
            e::WINDOWS_1255,
            e::ISO_8859_8,
            e::WINDOWS_1256,
            e::ISO_8859_6,
            e::WINDOWS_1258,
            e::WINDOWS_874,
            e::KOI8_U,
            e::ISO_8859_5,
            e::SHIFT_JIS,
            e::EUC_JP,
            e::EUC_KR,
            e::GBK,
            e::BIG5,
        ];
        (ours.into_iter())
            .chain(others.into_iter().map(|c| (Encoding::Other(c), c.name())))
            .collect()
    });
static ENCODING_NAMES: std::sync::LazyLock<Vec<&'static str>> =
    std::sync::LazyLock::new(|| ENCODINGS.iter().map(|&(_, n)| n).collect());
/// Arrow keys on a zoomed picture move it this many pixels.
const PAN: f32 = 40.0;

#[derive(Debug)]
pub enum Image {
    /// The handle and the picture's size in pixels (from its header; `None` if unreadable).
    Raster(widget::image::Handle, Option<(u32, u32)>),
    Svg(widget::svg::Handle),
}

/// A file read for viewing (built off the UI thread).
#[derive(Debug)]
pub struct Loaded {
    pub doc: Doc,
    /// Text mode in `shape`: as opened (UTF-8 / BOM, not wrapped), or as the viewer had it when
    /// another window was read.
    pub text: Text,
    /// Encoding and wrap width `text` was built with.
    pub shape: Shape,
    /// Handles are made once: a new handle per frame would decode the image every frame.
    pub image: Option<Image>,
}

/// Text mode's encoding and wrap width (`None`: not wrapped).
pub type Shape = (Encoding, Option<usize>);

impl Loaded {
    pub fn read(path: &Path, name: &str) -> Result<Self, String> {
        let doc = lister::load(path).map_err(|e| e.to_string())?;
        let text = Text::new(&doc, Encoding::Auto, None);
        // Over `LIMIT` only a window is read: cut bytes do not decode (it opens as hex instead).
        let whole = doc.start == 0 && doc.end() == doc.total;
        let image = (whole && lister::is_image(name)).then(|| {
            if name.to_ascii_lowercase().ends_with(".svg") {
                Image::Svg(widget::svg::Handle::from_memory(doc.bytes.clone()))
            } else {
                raster(&doc.bytes)
            }
        });
        Ok(Self {
            doc,
            text,
            shape: (Encoding::Auto, None),
            image,
        })
    }

    /// Another window of a file too big to hold whole (`lister::load_at`), its text built here
    /// as the viewer shows it: not on the UI thread.
    pub fn window(path: &Path, w: Window) -> Result<Self, String> {
        let doc = lister::load_at(path, w.from, w.to, w.line, w.bom).map_err(|e| e.to_string())?;
        let text = Text::new(&doc, w.shape.0, w.shape.1);
        Ok(Self {
            doc,
            text,
            shape: w.shape,
            image: None,
        })
    }
}

/// Why another window of the file is read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slide {
    /// Scrolled to the window's bottom / top: half a window on, the same line kept in view.
    Next,
    Prev,
    /// Home / End: the file's first / last window.
    Home,
    End,
    /// Search went past the window (forward or not): the one after / before it, searched.
    Find(bool),
    /// The search found nothing: back to the window it started in (from this offset).
    Return(u64),
}

/// What the viewer does with another window it got (`Lister::set_window`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Part {
    pub scroll: Option<f32>,
    /// Read another window: the search goes on, or returns to where it started.
    pub read: Option<Slide>,
    /// A search ended: found or not.
    pub found: Option<bool>,
}

/// The part of the file a slide reads (`lister::load_at`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Window {
    pub from: u64,
    pub to: u64,
    /// From a line start (text), else from a hex row.
    pub line: bool,
    pub bom: lister::Bom,
    pub shape: Shape,
}

/// A picture decoded here, off the UI thread: handed to the renderer as pixels, it would
/// otherwise decode it while drawing and the window would stand still (seconds for a 4K JPEG).
/// Turned by its EXIF orientation, as the renderer would. Not decodable here: the bytes, for the
/// renderer to try, with no size known.
fn raster(bytes: &[u8]) -> Image {
    match decode(
        bytes,
        image::Limits::default().max_alloc.unwrap_or(u64::MAX),
    )
    .map(image::DynamicImage::into_rgba8)
    {
        Some(px) => {
            let (w, h) = px.dimensions();
            Image::Raster(
                widget::image::Handle::from_rgba(w, h, px.into_raw()),
                Some((w, h)),
            )
        }
        None => Image::Raster(widget::image::Handle::from_bytes(bytes.to_vec()), None),
    }
}

/// The picture in `bytes`, turned by its EXIF orientation; `None` if not decodable here or
/// needing more than `max_alloc` bytes.
pub(crate) fn decode(bytes: &[u8], max_alloc: u64) -> Option<image::DynamicImage> {
    use image::ImageDecoder;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(max_alloc);
    let mut r = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    r.limits(limits);
    let mut d = r.into_decoder().ok()?;
    let turn = d.orientation().ok();
    let mut img = image::DynamicImage::from_decoder(d).ok()?;
    if let Some(o) = turn {
        img.apply_orientation(o);
    }
    Some(img)
}

pub struct Lister {
    /// Pane whose file is shown (N / P walk its list).
    pub side: usize,
    /// F3 in a window of its own (TC Lister), not in place of the panels.
    pub window: Option<cosmic::iced::window::Id>,
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
    /// Text size (Settings → Fonts); rows and columns grow with it.
    pub size: u16,
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
    /// Picture zoom (+ / -) as a factor of its own size; `None`: fitted to the window.
    pub zoom: Option<f32>,
    /// The picture held in the renderer: the next frame draws it (without it the renderer loads
    /// it on its own and nothing redraws until the pointer moves).
    pub allocation: Option<cosmic::iced::runtime::image::Allocation>,
    /// The file on disk, for reading other windows of a big one; `None`: only the first.
    pub file: Option<PathBuf>,
    /// Another window is being read for this.
    pub slide: Option<Slide>,
    /// Where a search through other windows started: window start, scroll y, the match shown.
    origin: Option<(u64, f32, Option<usize>)>,
}

impl Lister {
    fn scale(&self) -> f32 {
        self.size.max(1) as f32 / crate::config::LISTER_SIZE as f32
    }

    pub fn row_h(&self) -> f32 {
        (ROW_H * self.scale()).round()
    }

    fn mono_w(&self) -> f32 {
        MONO_W * self.scale()
    }

    /// ← / →: this many pixels (8 chars).
    fn step_x(&self) -> f32 {
        self.mono_w() * 8.0
    }

    /// A new size (Settings → Fonts): wrapped lines follow the new column count.
    pub fn set_size(&mut self, size: u16) {
        let before = self.wrap_cols();
        self.size = size;
        if self.wrap && self.wrap_cols() != before {
            self.retext();
        }
    }

    pub fn new(side: usize, name: String, id: u64, size: u16) -> Self {
        Self {
            side,
            window: None,
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
            size,
            text: None,
            hit: None,
            sel: None,
            dragging: false,
            pointer: None,
            double: None,
            quick: None,
            zoom: None,
            allocation: None,
            file: None,
            slide: None,
            origin: None,
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
        let (width, chosen, wrap, window) = (self.width, self.chosen, self.wrap, self.window);
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
            window,
            ..Self::new(self.side, name, id, self.size)
        };
    }

    /// F3 shown in place of both panels (not Ctrl+Q, not in its own window).
    pub fn in_place(&self) -> bool {
        self.quick.is_none() && self.window.is_none()
    }

    /// A picture too big to show (`LIMIT`): shown as hex, the info line says why.
    pub fn too_big(&self) -> bool {
        lister::is_image(&self.name) && self.ok().is_some_and(|l| l.image.is_none())
    }

    pub fn ok(&self) -> Option<&Loaded> {
        self.loaded.as_deref().and_then(|r| r.as_ref().ok())
    }

    pub fn set_loaded(&mut self, loaded: Arc<Result<Loaded, String>>) {
        if let Ok(l) = loaded.as_ref() {
            self.mode = match lister::detect(&self.name, l.doc.text()) {
                Mode::Image if l.image.is_none() => Mode::Hex,
                m => m,
            };
            // Not UTF-8: guess the code page, unless one was chosen (A / S / K, the list).
            if self.mode == Mode::Text && !self.chosen {
                self.encoding = lister::guess(&l.doc.bytes);
            }
        }
        self.loaded = Some(loaded);
        self.zoom = None; // each picture opens fitted
        self.allocation = None;
        self.retext();
    }

    /// The raster picture's handle, to have the renderer take it in at once.
    pub fn raster_handle(&self) -> Option<widget::image::Handle> {
        match self.ok()?.image.as_ref()? {
            Image::Raster(h, _) => Some(h.clone()),
            Image::Svg(_) => None,
        }
    }

    /// The raster picture's own size, if it is one and it was readable.
    pub fn image_size(&self) -> Option<(u32, u32)> {
        match self.ok()?.image.as_ref()? {
            Image::Raster(_, size) => *size,
            Image::Svg(_) => None,
        }
    }

    /// The scale the picture is shown at: its zoom, else the one that fits the window.
    pub fn shown_scale(&self) -> Option<f32> {
        let size = self.image_size()?;
        Some(
            self.zoom
                .unwrap_or_else(|| lister::fit_scale(size, (self.width, self.height))),
        )
    }

    /// The zoomed picture's size on screen; `None` while fitted.
    fn zoomed(&self) -> Option<(f32, f32)> {
        let ((w, h), z) = (self.image_size()?, self.zoom?);
        Some((w as f32 * z, h as f32 * z))
    }

    /// Plus or minus on a picture: the next zoom step from the shown scale. The offset that keeps
    /// the middle of the view on the same spot of the picture; `None` if nothing changed (an SVG,
    /// an unreadable header, the last step).
    pub fn zoom_image(&mut self, up: bool) -> Option<(f32, f32)> {
        let now = self.shown_scale()?;
        let z = lister::zoom_step(now, up)?;
        let was = self.zoom.is_some();
        self.zoom = Some(z);
        let (w, h) = self.zoomed()?;
        let r = z / now;
        let mid = |o: f32, view: f32, size: f32| match was {
            true => ((o + view / 2.0) * r - view / 2.0).clamp(0.0, (size - view).max(0.0)),
            false => ((size - view) / 2.0).max(0.0), // from fitted: the middle
        };
        Some((
            mid(self.offset.0, self.width, w),
            mid(self.offset.1, self.height, h),
        ))
    }

    /// Doc and the text mode content in the current encoding / wrapping.
    pub fn text(&self) -> Option<(&Doc, &Text)> {
        let l = self.ok()?;
        Some((&l.doc, self.text.as_ref().unwrap_or(&l.text)))
    }

    /// Chars per wrapped line: the viewport minus padding and the scrollbar.
    fn wrap_cols(&self) -> usize {
        ((self.width - 32.0).max(0.0) / self.mono_w()) as usize
    }

    fn shape(&self) -> Shape {
        (self.encoding, self.wrap.then(|| self.wrap_cols()))
    }

    fn retext(&mut self) {
        let shape = self.shape();
        self.text = match self.ok() {
            Some(l) if l.shape != shape => Some(Text::new(&l.doc, shape.0, shape.1)),
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
        (self.offset.1 / self.row_h()) as usize
    }

    /// Where a scroll key moves the view to (y), `None` for other keys.
    pub fn scroll_y(&self, action: Action) -> Option<f32> {
        // A zoomed picture pans by PAN px and pages by the window, the text by rows.
        let (step, total, page) = match self.zoomed().filter(|_| self.mode == Mode::Image) {
            Some((_, h)) => (PAN, h, self.height * 0.9),
            None => (
                self.row_h(),
                self.rows() as f32 * self.row_h(),
                viewport::page_rows(self.row_h(), self.height) as f32 * self.row_h(),
            ),
        };
        let max = (total - self.height).max(0.0);
        let y = self.offset.1;
        let to = match action {
            Action::Up => y - step,
            Action::Down => y + step,
            Action::PageUp => y - page,
            Action::PageDown => y + page,
            Action::Home => 0.0,
            Action::End => max,
            _ => return None,
        };
        Some(to.clamp(0.0, max))
    }

    pub fn scroll_x(&self, right: bool) -> f32 {
        if let Some((w, _)) = self.zoomed().filter(|_| self.mode == Mode::Image) {
            let x = self.offset.0 + if right { PAN } else { -PAN };
            return x.clamp(0.0, (w - self.width).max(0.0));
        }
        let x = self.offset.0 + if right { self.step_x() } else { -self.step_x() };
        x.clamp(0.0, (self.width() - self.step_x()).max(0.0))
    }

    fn width(&self) -> f32 {
        let cols = match self.mode {
            Mode::Hex => HEX_COLS,
            _ => self.text().map_or(0, |(_, t)| t.cols),
        };
        cols as f32 * self.mono_w() + 16.0
    }

    /// Search: from the top visible row (`again`: from the row after / before the last match).
    /// Returns the y to scroll to on a match.
    pub fn find(&mut self, forward: bool, again: bool) -> Option<f32> {
        let start = match (again, self.hit) {
            (true, Some(h)) if forward => h + 1,
            (true, Some(h)) => h.checked_sub(1)?,
            _ => self.top(),
        };
        self.find_from(start, forward)
    }

    fn find_from(&mut self, start: usize, forward: bool) -> Option<f32> {
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
        Some(row.saturating_sub(2) as f32 * self.row_h())
    }

    /// What `slide` reads: `None` if the file has no more that way (or is not on disk, or is a
    /// picture).
    // ponytail: a window read in text mode stays when 3 shows hex (addresses not on a 16-byte
    // row) and back (a hex window's first line may start mid-line); the next slide aligns it.
    pub fn window_for(&self, slide: Slide) -> Option<Window> {
        self.file.as_ref()?;
        if self.mode == Mode::Image {
            return None;
        }
        let doc = &self.ok()?.doc;
        let line = self.mode == Mode::Text;
        let (more, back) = (doc.end() < doc.total, doc.start > 0);
        let half = doc.bytes.len() as u64 / 2;
        // Hex search: a match across the edge is in both windows.
        let over = if line {
            0
        } else {
            self.query.len().saturating_sub(1) as u64
        };
        let (from, to) = match slide {
            Slide::Next if more => (doc.start + half.max(1), u64::MAX),
            Slide::Prev if back => (
                doc.start.saturating_sub(lister::LIMIT / 2),
                doc.start + lister::LIMIT / 2,
            ),
            Slide::Home if back => (0, u64::MAX),
            Slide::End if more => (doc.total.saturating_sub(lister::LIMIT), u64::MAX),
            // From the last line, cut by the window's end (the rows already searched are skipped).
            Slide::Find(true) if more && line => {
                (doc.start + lister::last_line_start(doc) as u64, u64::MAX)
            }
            Slide::Find(true) if more => (doc.end() - over.min(half), u64::MAX),
            Slide::Find(false) if back => (
                doc.start.saturating_sub(lister::LIMIT),
                doc.start + over.min(half),
            ),
            Slide::Return(from) => (from, u64::MAX),
            _ => return None,
        };
        Some(Window {
            from,
            to,
            line,
            bom: doc.bom,
            shape: self.shape(),
        })
    }

    /// Scrolled to the edge of a window that is not the file's: the slide to read.
    pub fn edge(&self) -> Option<Slide> {
        if self.slide.is_some() {
            return None;
        }
        let bottom = self.offset.1 + self.height >= (self.rows() as f32 - 1.0) * self.row_h();
        [(bottom, Slide::Next), (self.offset.1 <= 0.0, Slide::Prev)]
            .into_iter()
            .find(|&(at, s)| at && self.window_for(s).is_some())
            .map(|(_, s)| s)
    }

    /// The shown row where file offset `at` (a line start in this window) is.
    // ponytail: scans the window on the UI thread (a few ms per slide in release).
    fn row_of(&self, at: u64) -> usize {
        let Some((doc, text)) = self.text() else {
            return 0;
        };
        let off = at.saturating_sub(doc.start) as usize;
        match self.mode {
            Mode::Hex => off / lister::HEX_WIDTH,
            _ => lister::line_row(text.bytes(doc), &text.lines, lister::newlines(doc, off)),
        }
    }

    /// F7 / F3 found nothing in this window: the slide that searches on, the place it started
    /// kept to come back to.
    pub fn search_on(&mut self, forward: bool) -> Option<Slide> {
        let slide = Slide::Find(forward);
        if self.slide.is_some() {
            return None;
        }
        self.window_for(slide)?;
        self.origin = Some((self.ok()?.doc.start, self.offset.1, self.hit));
        Some(slide)
    }

    /// Another window arrived for `slide` (a reply for an older one is dropped).
    pub fn set_window(&mut self, slide: Slide, loaded: Arc<Result<Loaded, String>>) -> Part {
        let none = Part::default();
        if self.slide != Some(slide) {
            return none;
        }
        self.slide = None;
        let (Some(old), Some(new)) = (
            self.ok().map(|l| l.doc.start),
            loaded.as_ref().as_ref().ok().map(|l| l.doc.start),
        ) else {
            return none;
        };
        // New row = old row + shift (on: this window's rows before the new one; back: the new
        // one's before this one).
        let cut = if new >= old { self.row_of(new) } else { 0 };
        let hit = self.hit;
        self.loaded = Some(loaded);
        self.retext();
        let shift = match new >= old {
            true => -(cut as isize),
            false => self.row_of(old) as isize,
        };
        let rows = self.rows();
        let hit = hit
            .and_then(|h| h.checked_add_signed(shift))
            .filter(|&h| h < rows);
        let max = (rows as f32 * self.row_h() - self.height).max(0.0);
        let at = |y: f32| Some(y.clamp(0.0, max));
        let mut part = Part::default();
        match slide {
            Slide::Next | Slide::Prev => {
                self.hit = hit;
                part.scroll = at(self.offset.1 + shift as f32 * self.row_h());
            }
            Slide::Home => part.scroll = at(0.0),
            Slide::End => part.scroll = at(max),
            Slide::Return(_) => {
                let (_, y, hit) = self.origin.take().unwrap_or_default();
                (self.hit, part.scroll) = (hit, at(y));
            }
            Slide::Find(forward) => {
                // Past the last match, if it is in this window too.
                let start = match (hit, forward) {
                    (Some(h), true) => Some(h + 1),
                    (Some(h), false) => h.checked_sub(1),
                    (None, true) => Some(0),
                    (None, false) => rows.checked_sub(1),
                };
                part.scroll = start.and_then(|s| self.find_from(s, forward));
                if part.scroll.is_some() {
                    self.origin = None;
                    part.found = Some(true);
                } else if self.window_for(slide).is_some() {
                    part.read = Some(slide);
                } else {
                    part.found = Some(false);
                    // Back where the search started.
                    match self.origin {
                        Some((from, ..)) if from != new => part.read = Some(Slide::Return(from)),
                        Some((_, y, h)) => (self.hit, part.scroll) = (h, at(y)),
                        None => {}
                    }
                }
            }
        }
        part
    }

    /// The text position under the pointer (the view's scroll offset added).
    pub fn pointer_pos(&self) -> Option<lister::Pos> {
        let (x, y) = self.pointer?;
        Some(lister::pos_at(
            x + self.offset.0,
            y + self.offset.1,
            self.row_h(),
            self.mono_w(),
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
            Mode::Hex => lister::hex_row(&doc.bytes, i, doc.start),
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
            Mode::Image => {
                let size = shagoff_core::format::size(x.doc.total);
                match (l.image_size(), l.shown_scale()) {
                    (Some((w, h)), Some(z)) => {
                        let p = (z * 100.0).round() as u32;
                        fl!("lister-info-image", w = w, h = h, p = p, size = size)
                    }
                    _ => fl!("lister-info", size = size),
                }
            }
            _ if l.too_big() => fl!(
                "lister-info-too-big",
                size = shagoff_core::format::size(x.doc.total),
                limit = shagoff_core::format::size(lister::LIMIT)
            ),
            _ => fl!(
                "lister-info",
                size = shagoff_core::format::size(x.doc.total)
            ),
        },
    };
    // Name and info on their own line, buttons wrapping below: one row did not fit a narrow window.
    // In its own window the name is in the title bar already.
    let name: Element<'_, Message> = match l.window {
        None => widget::text::heading(l.name.clone())
            .wrapping(Wrapping::WordOrGlyph)
            .width(Length::Fill)
            .into(),
        Some(_) => widget::Space::new().width(Length::Fill).into(),
    };
    let title = row::with_children(vec![name, widget::text::body(info).into()])
        .spacing(8)
        .align_y(cosmic::iced::Alignment::Center);
    let mut buttons = modes;
    if l.mode == Mode::Image && l.zoom.is_some() {
        buttons.push(
            widget::button::standard(fl!("lister-fit"))
                .on_press(Message::ListerFit)
                .into(),
        );
    }
    let enc = ENCODINGS.iter().position(|(e, _)| *e == l.encoding);
    buttons.extend([
        widget::dropdown(ENCODING_NAMES.as_slice(), enc, |i| {
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
        let size = |n: u64| crate::view::human(n);
        col = col.push(widget::text::caption(fl!(
            "lister-truncated",
            from = size(x.doc.start),
            to = size(x.doc.end()),
            size = size(x.doc.total)
        )));
    }
    col.push(content(l)).padding([4, 8]).into()
}

fn content(l: &Lister) -> Element<'_, Message> {
    let (Some(x), Some((doc, txt))) = (l.ok(), l.text()) else {
        return widget::Space::new().into();
    };
    if l.mode == Mode::Image {
        let img: Element<'_, Message> = match (&x.image, l.zoomed()) {
            // Zoomed: its own size times the zoom, scrolled (also by the keys) when bigger.
            (Some(Image::Raster(h, _)), Some((w, ht))) => {
                let img = widget::image(h.clone())
                    .content_fit(ContentFit::Fill)
                    .width(Length::Fixed(w))
                    .height(Length::Fixed(ht));
                if w <= l.width && ht <= l.height {
                    widget::container(img).center(Length::Fill).into()
                } else {
                    widget::scrollable(img)
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
                        .height(Length::Fill)
                        .into()
                }
            }
            (Some(Image::Raster(h, _)), None) => {
                widget::container(widget::image(h.clone()).content_fit(ContentFit::ScaleDown))
                    .center(Length::Fill)
                    .into()
            }
            (Some(Image::Svg(h)), _) => {
                widget::container(widget::svg(h.clone()).content_fit(ContentFit::Contain))
                    .center(Length::Fill)
                    .into()
            }
            (None, _) => widget::Space::new().into(),
        };
        // The window's size: the fit scale and the zoom steps are reckoned from it.
        return cosmic::iced::widget::sensor(img)
            .on_show(Message::ListerResized)
            .on_resize(Message::ListerResized)
            .into();
    }
    // Only the rows in view are built; spacers keep the scroll height.
    let rows = l.rows();
    let range = viewport::visible_range(rows, l.row_h(), l.offset.1, l.height);
    let width = Length::Fixed(l.width());
    let mut list = column::with_capacity(range.len() + 2).push(
        widget::Space::new()
            .width(width)
            .height(Length::Fixed(range.start as f32 * l.row_h())),
    );
    let mono = |s: String| {
        widget::text(s)
            .size(l.size)
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
                .height(Length::Fixed(l.row_h()))
                .class(hit_style(hit)),
        );
    }
    list = list.push(
        widget::Space::new()
            .width(Length::Fixed(1.0))
            .height(Length::Fixed((rows - range.end) as f32 * l.row_h())),
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

    #[test]
    fn plus_minus_zoom_a_picture_from_its_fit_and_fit_again_on_the_next() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("p.bmp");
        image::RgbImage::new(2000, 1000).save(&path).unwrap();
        let mut l = Lister::new(0, "p.bmp".into(), 0, 14);
        l.set_loaded(Arc::new(Loaded::read(&path, "p.bmp")));
        (l.width, l.height) = (1000.0, 800.0); // fits at 0.5
        assert_eq!((l.mode, l.image_size()), (Mode::Image, Some((2000, 1000))));
        // From fitted: 0.75 (1500×750, wider than the view) — the view on its middle.
        assert_eq!(l.zoom_image(true), Some((250.0, 0.0)));
        assert_eq!(l.zoom, Some(0.75));
        assert!(l.zoom_image(false).is_some() && l.zoom_image(false).is_some());
        assert_eq!(l.zoom, Some(0.25));
        l.zoom = Some(8.0);
        assert_eq!(l.zoom_image(true), None); // the last step
        l.set_loaded(Arc::new(Loaded::read(&path, "p.bmp")));
        assert_eq!(l.zoom, None); // the next picture opens fitted
    }

    #[test]
    fn regression_a_picture_over_the_limit_opens_as_hex_and_says_why() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("big.png");
        let f = std::fs::File::create(&path).unwrap();
        f.set_len(lister::LIMIT + 10).unwrap(); // sparse; only its first 32 MB are read
        let mut l = Lister::new(0, "big.png".into(), 0, 14);
        l.set_loaded(Arc::new(Loaded::read(&path, "big.png")));
        // Was: image mode, an empty field (the cut bytes do not decode), nothing said.
        assert_eq!(l.mode, Mode::Hex);
        assert!(l.ok().unwrap().image.is_none());
        assert!(l.too_big());
    }

    fn text(s: &str) -> Lister {
        let mut l = Lister::new(0, "a.txt".into(), 1, crate::config::LISTER_SIZE);
        let doc = Doc {
            bytes: s.as_bytes().to_vec(),
            decoded: None,
            total: s.len() as u64,
            ..Default::default()
        };
        let text = Text::new(&doc, Encoding::Auto, None);
        l.set_loaded(Arc::new(Ok(Loaded {
            doc,
            text,
            shape: (Encoding::Auto, None),
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
        l.set_size(crate::config::LISTER_SIZE * 2); // twice the text: half the columns
        assert_eq!(l.rows(), 6); // ab cd ef gh ij, NUL
        assert_eq!(l.row_h(), 40.0);
        l.set_size(crate::config::LISTER_SIZE);
        l.reopen("b.txt".into(), 2);
        assert_eq!((l.encoding, l.wrap), (Encoding::Cp1251, true));
        l.toggle_wrap();
        l.set_encoding(Encoding::Auto);
        assert!(l.text.is_none());
    }

    #[test]
    fn a_cp1251_file_opens_decoded() {
        let mut l = Lister::new(0, "a.txt".into(), 1, crate::config::LISTER_SIZE);
        let doc = Doc {
            bytes: b"\xcf\xf0\xe8\xe2\xe5\xf2".to_vec(), // "Привет" in 1251
            decoded: None,
            total: 6,
            ..Default::default()
        };
        let text = Text::new(&doc, Encoding::Auto, None);
        l.set_loaded(Arc::new(Ok(Loaded {
            doc,
            text,
            shape: (Encoding::Auto, None),
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

    /// Read the window `slide` wants, as the app does off the UI thread.
    fn slide(l: &mut Lister, s: Slide) -> Option<f32> {
        let w = l.window_for(s)?;
        let loaded = Loaded::window(l.file.as_ref()?, w);
        l.slide = Some(s);
        let part = l.set_window(s, Arc::new(loaded));
        if let Some(y) = part.scroll {
            l.offset.1 = y;
        }
        match part.read {
            Some(next) => slide(l, next),
            None => part.scroll,
        }
    }

    fn top_line(l: &Lister) -> String {
        let (doc, txt) = l.text().unwrap();
        l.row_text(doc, txt, l.top())
    }

    #[test]
    fn a_big_file_is_read_window_by_window_keeping_the_line_in_view() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("big.log");
        let n = (lister::LIMIT as usize / 14) * 3 / 2; // 14-byte lines: 1.5 windows
        let body: String = (0..n).map(|i| format!("line {i:08}\n")).collect();
        std::fs::write(&path, &body).unwrap();
        let mut l = Lister::new(0, "big.log".into(), 1, crate::config::LISTER_SIZE);
        l.set_loaded(Arc::new(Loaded::read(&path, "big.log")));
        l.file = Some(path);
        l.height = 10.0 * ROW_H;
        assert_eq!(l.edge(), None); // at the top of the file's first window
        l.offset.1 = l.scroll_y(Action::End).unwrap();
        assert_eq!(l.edge(), Some(Slide::Next));
        let seen = top_line(&l);
        slide(&mut l, Slide::Next).unwrap();
        assert!(l.ok().unwrap().doc.start > 0);
        assert_eq!(top_line(&l), seen);
        l.offset.1 = 0.0;
        assert_eq!(l.edge(), Some(Slide::Prev));
        let seen = top_line(&l);
        slide(&mut l, Slide::Prev).unwrap();
        assert_eq!(top_line(&l), seen);
        // End: the file's last line at the bottom; Home: its first at the top
        slide(&mut l, Slide::End).unwrap();
        let last = l.rows() - 1;
        let (doc, txt) = l.text().unwrap();
        assert_eq!(l.row_text(doc, txt, last), format!("line {:08}", n - 1));
        assert_eq!(l.window_for(Slide::End), None);
        slide(&mut l, Slide::Home).unwrap();
        assert_eq!(top_line(&l), "line 00000000");
        // search past the first window, then back
        l.query = format!("line {:08}", n - 5);
        assert_eq!(l.find(true, false), None);
        assert!(slide(&mut l, Slide::Find(true)).is_some());
        let (doc, txt) = l.text().unwrap();
        assert_eq!(l.row_text(doc, txt, l.hit.unwrap()), l.query);
        l.query = "line 00000003".into();
        assert_eq!(l.find(false, true), None);
        assert!(slide(&mut l, Slide::Find(false)).is_some());
        assert_eq!(l.hit, Some(3));
        // a match in the window's last, cut line is not found again past it
        let first = Loaded::read(l.file.as_ref().unwrap(), "big.log");
        l.set_loaded(Arc::new(first)); // a whole first window (the search read a shorter one)
        let cut = (lister::LIMIT / 14) as usize; // the row of "li", cut at 32 MB
        l.query = "li".into();
        l.hit = Some(cut - 1);
        l.find(true, true).unwrap();
        assert_eq!(l.hit, Some(cut));
        assert_eq!(l.find(true, true), None);
        let s = l.search_on(true).unwrap();
        slide(&mut l, s).unwrap();
        let (doc, txt) = l.text().unwrap();
        assert_eq!(
            l.row_text(doc, txt, l.hit.unwrap()),
            format!("line {:08}", cut + 1)
        );
        // not found anywhere: back to the window and place the search started from
        let (start, y) = (l.ok().unwrap().doc.start, l.offset.1);
        l.query = "nowhere".into();
        let s = l.search_on(false).unwrap();
        let hit = l.hit;
        assert_eq!(slide(&mut l, s), Some(y));
        assert_eq!(l.hit, hit); // the match shown again
        assert_eq!(l.ok().unwrap().doc.start, start);
        // a reply for a slide given up (another asked for meanwhile) is dropped
        let w = l.window_for(Slide::Home).unwrap();
        let late = Arc::new(Loaded::window(l.file.as_ref().unwrap(), w));
        l.slide = Some(Slide::Next);
        assert_eq!(l.set_window(Slide::Home, late), Part::default());
        assert_eq!(
            (l.slide, l.ok().unwrap().doc.start),
            (Some(Slide::Next), start)
        );
        l.slide = None;
        // hex rows carry the file offset
        l.set_mode(Mode::Hex);
        slide(&mut l, Slide::Prev).unwrap();
        let (doc, txt) = l.text().unwrap();
        assert_eq!(doc.start % 16, 0);
        assert!(
            l.row_text(doc, txt, 0)
                .starts_with(&format!("{:08X}", doc.start))
        );
        // a picture has no windows
        l.mode = Mode::Image;
        assert_eq!(l.window_for(Slide::Home), None);
    }

    #[test]
    fn mode_keys_wait_for_the_file() {
        let mut l = Lister::new(0, "a.bin".into(), 1, crate::config::LISTER_SIZE);
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
