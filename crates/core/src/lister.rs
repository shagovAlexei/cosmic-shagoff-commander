//! F3 viewer (TC's Lister): mode detection, line index, hex rows, search.

use std::io::{self, Read};
use std::ops::Range;
use std::path::Path;

/// A window (`Doc`) holds at most this much of a file.
pub const LIMIT: u64 = 32 << 20;
/// Longer lines are split: one huge line (minified JS) would stall rendering.
pub const MAX_COLS: usize = 4096;
pub const HEX_WIDTH: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Text,
    Hex,
    Image,
}

/// Byte order mark of the file: windows past the first are decoded the same way.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Bom {
    #[default]
    None,
    Utf8,
    Le,
    Be,
}

impl Bom {
    fn of(bytes: &[u8]) -> Self {
        match bytes {
            [0xEF, 0xBB, 0xBF, ..] => Bom::Utf8,
            [0xFF, 0xFE, ..] => Bom::Le,
            [0xFE, 0xFF, ..] => Bom::Be,
            _ => Bom::None,
        }
    }

    /// Bytes per code unit: a line ends with a `\n` unit on a unit boundary.
    fn unit(self) -> usize {
        match self {
            Bom::Le | Bom::Be => 2,
            _ => 1,
        }
    }
}

/// A window of the file: at most `LIMIT` bytes from `start` (the whole of a smaller file).
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Doc {
    /// The window's bytes as they are in the file (hex shows these).
    pub bytes: Vec<u8>,
    /// The same as UTF-8 when the file has a BOM (UTF-16 or UTF-8), for text mode.
    pub decoded: Option<Vec<u8>>,
    /// Size of the whole file.
    pub total: u64,
    /// Offset of `bytes[0]` in the file.
    pub start: u64,
    pub bom: Bom,
}

impl Doc {
    /// Offset just past the window.
    pub fn end(&self) -> u64 {
        self.start + self.bytes.len() as u64
    }

    /// Only a part of the file is held.
    pub fn truncated(&self) -> bool {
        self.start > 0 || self.end() < self.total
    }

    /// Bytes for text mode and its line index.
    pub fn text(&self) -> &[u8] {
        self.decoded.as_deref().unwrap_or(&self.bytes)
    }
}

pub fn load(path: &Path) -> io::Result<Doc> {
    load_at(path, 0, u64::MAX, true, Bom::None)
}

/// The window of `path` up to `to` (at most `LIMIT` bytes) from `from`: with `line`, from the
/// first line start at or after it (or `from`, if no line ends in the window), else from `from`
/// rounded down to a hex row. `bom`: the file's, as its first window found it.
pub fn load_at(path: &Path, from: u64, to: u64, line: bool, bom: Bom) -> io::Result<Doc> {
    use std::io::{Seek, SeekFrom};
    // A fifo would block `open` forever, a device never ends: regular files only.
    let meta = std::fs::metadata(path)?;
    if !meta.is_file() {
        let msg = format!("{}: not a regular file", path.display());
        return Err(io::Error::new(io::ErrorKind::InvalidInput, msg));
    }
    let unit = bom.unit() as u64;
    let from = from - from % if line { unit } else { HEX_WIDTH as u64 };
    // One unit before `from` too: a line end there makes `from` a line start.
    let back = if line && from > 0 { unit } else { 0 };
    let mut file = std::fs::File::open(path)?;
    file.seek(SeekFrom::Start(from - back))?;
    let mut bytes = Vec::new();
    let len = to.saturating_sub(from).min(LIMIT);
    file.take(len + back).read_to_end(&mut bytes)?;
    let skip = match back {
        0 => 0,
        b => line_ends(&bytes, bom).next().unwrap_or(b as usize),
    };
    // The file got shorter than the window asked for (a log rotated meanwhile).
    if bytes.len() <= back as usize && from > 0 {
        let msg = format!("{}: shorter than before", path.display());
        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, msg));
    }
    bytes.drain(..skip.min(bytes.len()));
    let start = from - back + skip as u64;
    let (bom, decoded) = match start {
        0 => (Bom::of(&bytes), decode(&bytes)),
        _ => (bom, decode_from(&bytes, bom)),
    };
    // /proc files report size 0 but have content.
    let total = meta.len().max(start + bytes.len() as u64);
    Ok(Doc {
        bytes,
        decoded,
        total,
        start,
        bom,
    })
}

/// Offsets just past each line end (`\n`, or its UTF-16 unit) in a window's raw bytes.
fn line_ends(bytes: &[u8], bom: Bom) -> impl Iterator<Item = usize> + '_ {
    let u = bom.unit();
    (bytes.chunks_exact(u).enumerate())
        .filter(move |(_, c)| match bom {
            Bom::Le => *c == b"\n\0",
            Bom::Be => *c == b"\0\n",
            _ => c[0] == b'\n',
        })
        .map(move |(i, _)| (i + 1) * u)
}

/// Line ends in the window's raw bytes before `upto`: the file's lines it holds before there.
pub fn newlines(doc: &Doc, upto: usize) -> usize {
    line_ends(&doc.bytes[..upto.min(doc.bytes.len())], doc.bom).count()
}

/// Where the window's last line starts (the window's length if no line ends in it).
pub fn last_line_start(doc: &Doc) -> usize {
    match line_ends(&doc.bytes, doc.bom).last() {
        Some(p) if p < doc.bytes.len() => p,
        _ => doc.bytes.len(),
    }
}

/// The first shown row of the window's `k`-th line (0-based): rows of wrapped or cut lines
/// counted. `lines` past the text's end.
pub fn line_row(text: &[u8], lines: &[Range<usize>], k: usize) -> usize {
    if k == 0 {
        return 0;
    }
    match (text.iter().enumerate())
        .filter(|&(_, &b)| b == b'\n')
        .nth(k - 1)
    {
        Some((p, _)) => lines.partition_point(|r| r.start <= p),
        None => lines.len(),
    }
}

fn utf16(units: &[u8], be: bool) -> Vec<u8> {
    let (pairs, rest) = units.as_chunks::<2>();
    let units = pairs.iter().map(|&pair| {
        if be {
            u16::from_be_bytes(pair)
        } else {
            u16::from_le_bytes(pair)
        }
    });
    let mut s: String = char::decode_utf16(units)
        .map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER))
        .collect();
    if !rest.is_empty() {
        s.push(char::REPLACEMENT_CHARACTER);
    }
    s.into_bytes()
}

/// A window past the BOM: UTF-16 decoded; UTF-8 (BOM or not) is the text as it is.
fn decode_from(bytes: &[u8], bom: Bom) -> Option<Vec<u8>> {
    match bom {
        Bom::Le => Some(utf16(bytes, false)),
        Bom::Be => Some(utf16(bytes, true)),
        _ => None,
    }
}

/// UTF-16 with a BOM → UTF-8; a UTF-8 BOM dropped. `None`: no BOM, the bytes are the text.
pub fn decode(bytes: &[u8]) -> Option<Vec<u8>> {
    match Bom::of(bytes) {
        Bom::Utf8 => Some(bytes[3..].to_vec()),
        Bom::None => None,
        b => decode_from(&bytes[2..], b),
    }
}

/// The codecs `Encoding::Other` holds, for the UI to name them without a dependency of its own.
pub use encoding_rs;

/// Text mode encoding (TC: A = ANSI, S = DOS; K for KOI8-R; 8 back to UTF-8 / BOM).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Encoding {
    /// UTF-8, or UTF-16 / UTF-8 by BOM.
    #[default]
    Auto,
    Cp1251,
    Koi8r,
    Cp866,
    /// Any other single- or multi-byte code page (Western, Greek, Japanese…), as found by `guess`
    /// or picked in the list.
    Other(&'static encoding_rs::Encoding),
}

impl Encoding {
    /// The `encoding_rs` codec of a code page; `None` for `Auto` (UTF-8 / BOM).
    fn codec(self) -> Option<&'static encoding_rs::Encoding> {
        Some(match self {
            Encoding::Auto => return None,
            Encoding::Cp1251 => encoding_rs::WINDOWS_1251,
            Encoding::Koi8r => encoding_rs::KOI8_R,
            Encoding::Cp866 => encoding_rs::IBM866,
            Encoding::Other(e) => e,
        })
    }

    /// Ours for the Cyrillic pages (they have keys), `Other` for the rest, `Auto` for UTF-8.
    pub fn of(e: &'static encoding_rs::Encoding) -> Self {
        match e {
            e if e == encoding_rs::UTF_8 => Encoding::Auto,
            e if e == encoding_rs::WINDOWS_1251 => Encoding::Cp1251,
            e if e == encoding_rs::KOI8_R => Encoding::Koi8r,
            e if e == encoding_rs::IBM866 => Encoding::Cp866,
            e => Encoding::Other(e),
        }
    }
}

/// The text as UTF-8 in `enc`; `None` for `Auto` without a BOM (the bytes are the text).
pub fn decode_as(bytes: &[u8], enc: Encoding) -> Option<Vec<u8>> {
    match enc.codec() {
        None => decode(bytes),
        Some(c) => Some(
            c.decode_without_bom_handling(bytes)
                .0
                .into_owned()
                .into_bytes(),
        ),
    }
}

/// Encoding a file most likely is in, from its first 64 KiB:
/// - valid UTF-8 (or a BOM), or UTF-8 with a few broken bytes → `Auto`;
/// - the Cyrillic page (1251 / KOI8-R / 866) in which they read as the most lowercase Russian
///   letters (running text is mostly lowercase; in the wrong page those bytes turn into capitals
///   or box drawing) — unless `chardetng` (Firefox's detector) sees another script in them: Greek
///   lowercase, say, sits on the bytes of Russian lowercase in 1251. Its Latin guesses do not
///   count against Russian: on a word or two it says Windows-1252;
/// - otherwise what `chardetng` makes of them.
pub fn guess(bytes: &[u8]) -> Encoding {
    use encoding_rs as e;
    let sample = &bytes[..bytes.len().min(64 << 10)];
    let valid = match std::str::from_utf8(sample) {
        Ok(_) => true,
        // Only a char cut at the end of the sample.
        Err(e) => e.error_len().is_none(),
    };
    if valid || decode(bytes).is_some() {
        return Encoding::Auto;
    }
    // UTF-8 with a stray byte: more real multi-byte chars than broken ones (legacy text hardly
    // ever forms valid UTF-8 sequences by chance).
    let lossy = String::from_utf8_lossy(sample);
    let broken = lossy.chars().filter(|&c| c == '\u{fffd}').count();
    let fine = lossy
        .chars()
        .filter(|&c| !c.is_ascii() && c != '\u{fffd}')
        .count();
    if fine > broken {
        return Encoding::Auto;
    }
    // Lowercase Russian letters right after another Russian letter: words come in runs, while
    // Latin-1 accents (`café`) are single high bytes between ASCII letters.
    let cyr = |c: char| matches!(c, 'а'..='я' | 'А'..='Я' | 'ё' | 'Ё');
    let score = |enc| {
        let text = decode_as(sample, enc).unwrap_or_default();
        let text = String::from_utf8_lossy(&text);
        let mut prev = ' ';
        let mut n = 0;
        for c in text.chars() {
            if cyr(prev) && matches!(c, 'а'..='я' | 'ё') {
                n += 1;
            }
            prev = c;
        }
        n
    };
    let high = sample.iter().filter(|&&b| b >= 0x80).count();
    // Cp1251 last: `max_by_key` keeps the last of equal scores, and it is the likeliest.
    let russian = [Encoding::Cp866, Encoding::Koi8r, Encoding::Cp1251]
        .into_iter()
        .map(|e| (score(e), e))
        .filter(|&(n, _)| n > 0 && n * 3 >= high)
        .max_by_key(|&(n, _)| n)
        .map(|(_, e)| e);
    let mut d = chardetng::EncodingDetector::new(chardetng::Iso2022JpDetection::Deny);
    d.feed(sample, sample.len() == bytes.len());
    let found = d.guess(None, chardetng::Utf8Detection::Allow);
    // What `chardetng` may say of Russian text: ours tells these apart better (it calls KOI8-R
    // KOI8-U), and on a word or two it says Latin.
    let ours_or_latin = [
        e::WINDOWS_1251,
        e::KOI8_R,
        e::KOI8_U,
        e::IBM866,
        e::WINDOWS_1252,
        e::WINDOWS_1250,
        e::ISO_8859_2,
        e::WINDOWS_1254,
        e::WINDOWS_1257,
        e::ISO_8859_4,
        e::WINDOWS_1258,
    ];
    // KOI8-U differs from KOI8-R only in these: Ukrainian і ї є ґ (KOI8-R has box drawing there).
    let ukrainian = || {
        (sample.iter()).any(|b| matches!(b, 0xa4 | 0xa6 | 0xa7 | 0xad | 0xb4 | 0xb6 | 0xb7 | 0xbd))
    };
    match russian {
        Some(Encoding::Koi8r) if found == e::KOI8_U && ukrainian() => Encoding::Other(e::KOI8_U),
        Some(r) if ours_or_latin.contains(&found) => r,
        _ if found == e::KOI8_U && !ukrainian() => Encoding::Koi8r,
        _ => Encoding::of(found),
    }
}

/// Text mode content: UTF-8 bytes in the chosen encoding, its line index and widest line.
#[derive(Debug, PartialEq, Eq)]
pub struct Text {
    /// `None`: the file's own bytes are the text.
    pub decoded: Option<Vec<u8>>,
    pub lines: Vec<Range<usize>>,
    /// Widest line in monospace cells (the scrolled width).
    pub cols: usize,
}

impl Text {
    /// `wrap`: lines wrapped at words to this many cells (TC `W`); `None`: cut at `MAX_COLS`.
    pub fn new(doc: &Doc, enc: Encoding, wrap: Option<usize>) -> Self {
        let decoded = match enc {
            Encoding::Auto => doc.decoded.clone(),
            _ => decode_as(&doc.bytes, enc),
        };
        let bytes = decoded.as_deref().unwrap_or(&doc.bytes);
        let lines = match wrap {
            Some(w) => lines(bytes, w.clamp(1, MAX_COLS), true),
            None => lines(bytes, MAX_COLS, false),
        };
        let cols = lines.iter().map(|r| width(bytes, r)).max().unwrap_or(0);
        Self {
            decoded,
            lines,
            cols,
        }
    }

    pub fn bytes<'a>(&'a self, doc: &'a Doc) -> &'a [u8] {
        self.decoded.as_deref().unwrap_or(&doc.bytes)
    }
}

const IMAGES: [&str; 8] = ["png", "jpg", "jpeg", "gif", "bmp", "webp", "ico", "svg"];

/// Bigger files get their type's icon in the Thumbnails view: decoding them takes long.
pub const THUMB_MAX_BYTES: u64 = 64 << 20;

/// Whether `name` (`size` bytes) gets a thumbnail made (svg: not decodable there, it gets its icon).
pub fn thumb_wanted(name: &str, size: u64) -> bool {
    size <= THUMB_MAX_BYTES && is_image(name) && !name.to_ascii_lowercase().ends_with(".svg")
}

pub fn is_image(name: &str) -> bool {
    name.rsplit_once('.')
        .is_some_and(|(_, ext)| IMAGES.contains(&ext.to_ascii_lowercase().as_str()))
}

/// Picture zoom steps (TC Lister: + / -), as a factor of the image's own size.
pub const ZOOMS: [f32; 11] = [0.1, 0.25, 0.5, 0.75, 1.0, 1.5, 2.0, 3.0, 4.0, 6.0, 8.0];

/// The scale a picture of `size` pixels is shown at to fit `view`: never above its own size.
pub fn fit_scale((w, h): (u32, u32), (vw, vh): (f32, f32)) -> f32 {
    if w == 0 || h == 0 {
        return 1.0;
    }
    (vw / w as f32).min(vh / h as f32).clamp(0.01, 1.0)
}

/// The next zoom step above (`up`) or below `now`; `None` past the last one.
pub fn zoom_step(now: f32, up: bool) -> Option<f32> {
    // A little slack: `now` may be a step itself, or a fit scale a hair off one.
    match up {
        true => ZOOMS.into_iter().find(|&z| z > now * 1.001),
        false => ZOOMS.into_iter().rev().find(|&z| z < now * 0.999),
    }
}

/// Mode a file opens in: images by extension, a NUL in the first 8 KiB → hex, else text.
pub fn detect(name: &str, bytes: &[u8]) -> Mode {
    if is_image(name) {
        Mode::Image
    } else if bytes[..bytes.len().min(8192)].contains(&0) {
        Mode::Hex
    } else {
        Mode::Text
    }
}

/// Byte ranges of the lines (without `\n` / `\r\n`), lines wider than `max_cols` cells split:
/// with `words`, after the last whitespace that fits (a word wider than that: where it fills the
/// line); without, where the line is full.
pub fn lines(bytes: &[u8], max_cols: usize, words: bool) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut push = |s: usize, mut e: usize| {
        if e > s && bytes[e - 1] == b'\r' {
            e -= 1;
        }
        split_long(bytes, s..e, max_cols, words, &mut out);
    };
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'\n' {
            push(start, i);
            start = i + 1;
        }
    }
    if start < bytes.len() {
        push(start, bytes.len());
    }
    out
}

/// One line being cut into pieces of at most `max` cells.
struct Pieces<'a> {
    out: &'a mut Vec<Range<usize>>,
    max: usize,
    words: bool,
    from: usize,
    cells: usize,
    /// The piece has more than whitespace (its indent is not a place to break).
    text: bool,
    /// Just after the last whitespace past some text, and the piece's cells up to there.
    brk: Option<(usize, usize)>,
}

impl Pieces<'_> {
    /// A char of `w` cells and `len` bytes at `i`.
    fn put(&mut self, i: usize, len: usize, w: usize, space: bool) {
        if self.cells + w > self.max && self.cells > 0 {
            if let Some((b, bc)) = self.brk.take() {
                self.out.push(self.from..b);
                self.from = b;
                self.cells -= bc;
            }
            // Still too wide (one long word, or no words): cut it here.
            if self.cells + w > self.max && self.cells > 0 {
                self.out.push(self.from..i);
                (self.from, self.cells, self.text) = (i, 0, false);
            }
        }
        self.cells += w;
        if !space {
            self.text = true;
        } else if self.words && self.text {
            self.brk = Some((i + len, self.cells));
        }
    }
}

/// Cut `line` into pieces of at most `max` cells, at char starts (UTF-8 continuation bytes never
/// start a piece).
fn split_long(
    bytes: &[u8],
    line: Range<usize>,
    max: usize,
    words: bool,
    out: &mut Vec<Range<usize>>,
) {
    // Same cells as `line_text` / `width` draw: a tab is four, wide chars two, each invalid
    // UTF-8 sequence one `�`.
    let mut p = Pieces {
        out,
        max,
        words,
        from: line.start,
        cells: 0,
        text: false,
        brk: None,
    };
    let mut at = line.start;
    for chunk in bytes[line.clone()].utf8_chunks() {
        for (i, c) in chunk.valid().char_indices() {
            let w = match c {
                '\t' => 4,
                c if wide(c) => 2,
                _ => 1,
            };
            // Any whitespace but the no-break spaces (U+3000 between Japanese phrases too).
            let space = c.is_whitespace() && !matches!(c, '\u{a0}' | '\u{2007}' | '\u{202f}');
            p.put(at + i, c.len_utf8(), w, space);
        }
        at += chunk.valid().len();
        if !chunk.invalid().is_empty() {
            p.put(at, chunk.invalid().len(), 1, false);
            at += chunk.invalid().len();
        }
    }
    p.out.push(p.from..line.end);
}

/// A line as shown: invalid UTF-8 as `�`, tabs as four spaces.
pub fn line_text(bytes: &[u8], r: &Range<usize>) -> String {
    String::from_utf8_lossy(&bytes[r.clone()]).replace('\t', "    ")
}

/// Width of a line as shown, in monospace cells: a tab is four, CJK and emoji two.
pub fn width(bytes: &[u8], r: &Range<usize>) -> usize {
    let b = &bytes[r.clone()];
    if b.is_ascii() {
        return b.iter().map(|&c| if c == b'\t' { 4 } else { 1 }).sum();
    }
    String::from_utf8_lossy(b)
        .chars()
        .map(|c| match c {
            '\t' => 4,
            c if wide(c) => 2,
            _ => 1,
        })
        .sum()
}

// ponytail: the main East Asian Wide blocks and emoji, not the full Unicode width table.
fn wide(c: char) -> bool {
    matches!(c as u32,
        0x1100..=0x115F
        | 0x2E80..=0xA4CF
        | 0xAC00..=0xD7A3
        | 0xF900..=0xFAFF
        | 0xFE30..=0xFE4F
        | 0xFF00..=0xFF60
        | 0xFFE0..=0xFFE6
        | 0x1F300..=0x1FAFF
        | 0x20000..=0x3FFFD)
}

pub fn hex_rows(len: usize) -> usize {
    len.div_ceil(HEX_WIDTH)
}

/// `00000010  48 65 6C 6C 6F 20 77 6F  72 6C 64 0A 00 00 00 00  Hello world.....`;
/// `base`: the window's offset in the file, for the address.
pub fn hex_row(bytes: &[u8], row: usize, base: u64) -> String {
    let start = row * HEX_WIDTH;
    let chunk = &bytes[start.min(bytes.len())..(start + HEX_WIDTH).min(bytes.len())];
    let mut s = format!("{:08X} ", base + start as u64);
    for i in 0..HEX_WIDTH {
        if i == HEX_WIDTH / 2 {
            s.push(' ');
        }
        match chunk.get(i) {
            Some(b) => s.push_str(&format!(" {b:02X}")),
            None => s.push_str("   "),
        }
    }
    s.push_str("  ");
    s.extend(chunk.iter().map(|&b| {
        if (0x20..0x7F).contains(&b) {
            b as char
        } else {
            '.'
        }
    }));
    s
}

/// First line from `start` (inclusive, going down or up) that contains `query`, ignoring case.
pub fn find_line(
    bytes: &[u8],
    lines: &[Range<usize>],
    query: &str,
    start: usize,
    forward: bool,
) -> Option<usize> {
    if query.is_empty() || lines.is_empty() {
        return None;
    }
    let q = query.to_lowercase();
    let hit = |&i: &usize| line_text(bytes, &lines[i]).to_lowercase().contains(&q);
    if forward {
        (start..lines.len()).find(hit)
    } else {
        (0..=start.min(lines.len() - 1)).rev().find(hit)
    }
}

/// Hex row of the first match of `needle` that starts in row `start` or later (or, going up, in
/// row `start` or earlier).
pub fn find_bytes(bytes: &[u8], needle: &[u8], start: usize, forward: bool) -> Option<usize> {
    if needle.is_empty() || needle.len() > bytes.len() {
        return None;
    }
    let mut at = bytes.windows(needle.len()).enumerate();
    let pos = if forward {
        at.find(|(i, w)| *i >= start * HEX_WIDTH && *w == needle)
    } else {
        at.rfind(|(i, w)| *i < (start + 1) * HEX_WIDTH && *w == needle)
    };
    pos.map(|(i, _)| i / HEX_WIDTH)
}

/// A place in the shown rows: (row, column in chars).
pub type Pos = (usize, usize);

/// Mouse selection in the viewer: from where the button went down to where it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    pub anchor: Pos,
    pub head: Pos,
}

impl Selection {
    pub fn new(at: Pos) -> Self {
        Self {
            anchor: at,
            head: at,
        }
    }

    /// (start, end), start first.
    pub fn ordered(&self) -> (Pos, Pos) {
        if self.anchor <= self.head {
            (self.anchor, self.head)
        } else {
            (self.head, self.anchor)
        }
    }

    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }

    /// The chars of `row` (`len` of them) that are selected, if any.
    pub fn in_row(&self, row: usize, len: usize) -> Option<Range<usize>> {
        let ((r0, c0), (r1, c1)) = self.ordered();
        if row < r0 || row > r1 {
            return None;
        }
        let start = if row == r0 { c0.min(len) } else { 0 };
        let end = if row == r1 { c1.min(len) } else { len };
        (start < end).then_some(start..end)
    }

    /// The selected text, rows joined by `\n` (`text(i)`: row `i` as shown).
    pub fn text(&self, text: impl Fn(usize) -> String) -> String {
        let ((r0, _), (r1, _)) = self.ordered();
        (r0..=r1)
            .map(|r| {
                let line = text(r);
                let n = line.chars().count();
                match self.in_row(r, n) {
                    Some(c) => line.chars().skip(c.start).take(c.len()).collect(),
                    None => String::new(),
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Double click: the word around char `col` of `line` (letters, digits, `_`), else the one char
/// there; `None` past the end.
pub fn word_at(line: &str, col: usize) -> Option<Range<usize>> {
    let chars: Vec<char> = line.chars().collect();
    let at = *chars.get(col)?;
    let word = |c: &char| c.is_alphanumeric() || *c == '_';
    if !word(&at) {
        return Some(col..col + 1);
    }
    let start = chars[..col]
        .iter()
        .rposition(|c| !word(c))
        .map_or(0, |i| i + 1);
    let end = chars[col..]
        .iter()
        .position(|c| !word(c))
        .map_or(chars.len(), |i| col + i);
    Some(start..end)
}

/// The position under (x, y) in content pixels: rows of `row_h`, chars of `char_w` (monospace),
/// between chars rounding to the nearer edge; the row is kept within `rows`.
pub fn pos_at(x: f32, y: f32, row_h: f32, char_w: f32, rows: usize) -> Pos {
    let row = ((y.max(0.0) / row_h) as usize).min(rows.saturating_sub(1));
    let col = (x.max(0.0) / char_w).round() as usize;
    (row, col)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thumbs_for_raster_pictures_not_too_big() {
        assert!(thumb_wanted("a.JPG", 10) && !thumb_wanted("a.svg", 10));
        assert!(!thumb_wanted("a.txt", 10) && !thumb_wanted("a.png", THUMB_MAX_BYTES + 1));
    }

    #[test]
    fn zoom_steps_from_fit_and_between_steps() {
        assert_eq!(fit_scale((2000, 1000), (1000.0, 1000.0)), 0.5);
        assert_eq!(fit_scale((100, 100), (1000.0, 800.0)), 1.0); // small: its own size
        assert_eq!(fit_scale((0, 10), (100.0, 100.0)), 1.0);
        assert_eq!(zoom_step(0.5, true), Some(0.75));
        assert_eq!(zoom_step(0.5, false), Some(0.25));
        assert_eq!(zoom_step(0.6, true), Some(0.75)); // from a fit scale
        assert_eq!(zoom_step(0.6, false), Some(0.5));
        assert_eq!(zoom_step(8.0, true), None);
        assert_eq!(zoom_step(0.1, false), None);
    }

    #[test]
    fn single_byte_code_pages() {
        let utf = |enc, b: &[u8]| String::from_utf8(decode_as(b, enc).unwrap()).unwrap();
        // "Привет" in each
        assert_eq!(
            utf(Encoding::Cp1251, b"\xcf\xf0\xe8\xe2\xe5\xf2 1"),
            "Привет 1"
        );
        assert_eq!(utf(Encoding::Koi8r, b"\xf0\xd2\xc9\xd7\xc5\xd4"), "Привет");
        assert_eq!(utf(Encoding::Cp866, b"\x8f\xe0\xa8\xa2\xa5\xe2"), "Привет");
        assert_eq!(utf(Encoding::Cp1251, b"\xa8\xb8\xb9"), "Ёё№");
        assert_eq!(decode_as(b"plain", Encoding::Auto), None);
        let latin = Encoding::Other(encoding_rs::WINDOWS_1252);
        assert_eq!(utf(latin, b"caf\xe9 \x80"), "café €");
    }

    #[test]
    fn guess_picks_the_code_page_of_russian_text() {
        let text = "Съешь же ещё этих мягких французских булок, да выпей чаю";
        let enc = |codec: &[u8]| guess(codec);
        assert_eq!(enc(text.as_bytes()), Encoding::Auto);
        assert_eq!(enc(b"plain ascii"), Encoding::Auto);
        // the same sentence in each page (bytes from Python's codecs)
        let cp1251 = encode(encoding_rs::WINDOWS_1251, text);
        let koi8 = encode(encoding_rs::KOI8_R, text);
        let cp866 = encode(encoding_rs::IBM866, text);
        assert_eq!(enc(&cp1251), Encoding::Cp1251);
        assert_eq!(enc(&koi8), Encoding::Koi8r);
        assert_eq!(enc(&cp866), Encoding::Cp866);
        assert_eq!(enc(b"\xff\xfe\x00\x00"), Encoding::Auto); // UTF-16 BOM
        // ties go to 1251; a word or two stays Russian (`chardetng` says Latin there)
        assert_eq!(enc(b"\xe4\xe0"), Encoding::Cp1251); // "да"
        assert_eq!(enc(b"\xcf\xf0\xe8\xe2\xe5\xf2"), Encoding::Cp1251); // "Привет"
        // ISO-8859-5 is Cyrillic, but not one of ours: not forced into one
        let iso = encode(encoding_rs::ISO_8859_5, text);
        assert_eq!(enc(&iso), Encoding::Other(encoding_rs::ISO_8859_5));
        // UTF-8 with one stray Latin-1 byte stays UTF-8
        let mut utf = "Größe, schön, Straße, Ärger, Öl".as_bytes().to_vec();
        utf.extend(b" caf\xe9");
        assert_eq!(enc(&utf), Encoding::Auto);
        // a UTF-8 char cut at the end of the sample is still UTF-8
        assert_eq!(enc(&"я".as_bytes()[..1]), Encoding::Auto);
    }

    /// Test helper: `text` in a code page.
    fn encode(codec: &'static encoding_rs::Encoding, text: &str) -> Vec<u8> {
        codec.encode(text).0.into_owned()
    }

    #[test]
    fn guess_finds_other_code_pages_too() {
        let other = |codec, text: &str| {
            assert_eq!(
                guess(&encode(codec, text)),
                Encoding::Other(codec),
                "{text}"
            );
        };
        other(
            encoding_rs::WINDOWS_1252,
            "Le cœur a ses raisons que la raison ne connaît point. Où est la bibliothèque? \
             Ich möchte gern ein Stück Kuchen, bitte. Größe, Straße, schön.",
        );
        other(
            encoding_rs::WINDOWS_1250,
            "Příliš žluťoučký kůň úpěl ďábelské ódy. Zażółć gęślą jaźń, pchnąć w tę łódź jeża.",
        );
        other(
            encoding_rs::SHIFT_JIS,
            "吾輩は猫である。名前はまだ無い。どこで生れたかとんと見当がつかぬ。",
        );
        other(
            encoding_rs::KOI8_U,
            "Їжак і єнот ґанок. Чуєш їх, доцю, га? Кумедна ж ти, прощайся без ґольфів!",
        );
        other(
            encoding_rs::WINDOWS_1253,
            "Ξεσκεπάζω την ψυχοφθόρα βδελυγμία. Η γλώσσα μου έδωσαν ελληνική.",
        );
    }

    #[test]
    fn text_wraps_and_switches_encoding() {
        let doc = Doc {
            bytes: b"abcdefg\n\xcf\xf0".to_vec(),
            decoded: None,
            total: 10,
            ..Default::default()
        };
        let t = Text::new(&doc, Encoding::Auto, None);
        assert_eq!((t.lines.len(), t.cols), (2, 7));
        let t = Text::new(&doc, Encoding::Auto, Some(3));
        let shown: Vec<String> = t
            .lines
            .iter()
            .map(|r| line_text(t.bytes(&doc), r))
            .collect();
        assert_eq!(shown, ["abc", "def", "g", "\u{fffd}\u{fffd}"]);
        let t = Text::new(&doc, Encoding::Cp1251, None);
        assert_eq!(line_text(t.bytes(&doc), &t.lines[1]), "Пр");
        // tabs are four cells, a broken byte one `�`: wrapped by what is drawn
        let tabs = Doc {
            bytes: b"\ta\tb\xffcd".to_vec(),
            decoded: None,
            total: 7,
            ..Default::default()
        };
        let t = Text::new(&tabs, Encoding::Auto, Some(5));
        let shown: Vec<String> = t.lines.iter().map(|r| line_text(&tabs.bytes, r)).collect();
        assert_eq!(shown, ["    a", "    b", "\u{fffd}cd"]); // an indent is no place to break
        assert!(t.cols <= 5);
        // a zero-width viewport must not loop forever
        assert_eq!(Text::new(&doc, Encoding::Auto, Some(0)).lines.len(), 9);
    }

    #[test]
    fn detect_picks_mode() {
        assert_eq!(detect("a.PNG", b"text"), Mode::Image);
        assert_eq!(detect("a.svg", b"<svg/>"), Mode::Image);
        assert_eq!(detect("a.bin", b"ab\0cd"), Mode::Hex);
        assert_eq!(detect("a.txt", "привет".as_bytes()), Mode::Text);
        assert_eq!(detect("png", b""), Mode::Text); // no extension
    }

    #[test]
    fn wrap_breaks_after_spaces_and_cuts_only_words_too_long() {
        let shown = |b: &[u8], max| -> Vec<String> {
            lines(b, max, true)
                .iter()
                .map(|r| line_text(b, r))
                .collect()
        };
        assert_eq!(shown(b"hello world foo", 8), ["hello ", "world ", "foo"]);
        assert_eq!(shown(b"abcdefghij k", 4), ["abcd", "efgh", "ij k"]);
        assert_eq!(shown(b"abcdefghij kl", 4), ["abcd", "efgh", "ij ", "kl"]);
        assert_eq!(
            shown("съешь же ещё".as_bytes(), 5),
            ["съешь", " же ", "ещё"]
        );
        assert_eq!(shown(b"a\tb c", 5), ["a    ", "b c"]); // a tab is four cells
        assert_eq!(shown(b"short", 8), ["short"]);
        // an indent is not a break: no row of only spaces
        assert_eq!(shown(b"    abcdefgh", 6), ["    ab", "cdefgh"]);
        // the ideographic space breaks Japanese phrases
        assert_eq!(
            shown("吾輩は\u{3000}猫である".as_bytes(), 8),
            ["吾輩は\u{3000}", "猫である"]
        );
        // without `words`: cut where full, as when not wrapped
        let hard: Vec<_> = lines(b"ab cd ef", 4, false)
            .iter()
            .map(|r| line_text(b"ab cd ef", r))
            .collect();
        assert_eq!(hard, ["ab c", "d ef"]);
    }

    #[test]
    fn lines_split_on_lf_and_crlf() {
        let b = b"one\r\ntwo\n\nlast";
        let l = lines(b, MAX_COLS, false);
        let text: Vec<String> = l.iter().map(|r| line_text(b, r)).collect();
        assert_eq!(text, ["one", "two", "", "last"]);
        assert_eq!(lines(b"a\n", MAX_COLS, false).len(), 1);
        assert!(lines(b"", MAX_COLS, false).is_empty());
    }

    #[test]
    fn long_line_split_at_char_boundary() {
        let s = "я".repeat(MAX_COLS + 3);
        let l = lines(s.as_bytes(), MAX_COLS, false);
        assert_eq!(l.len(), 2);
        assert_eq!(line_text(s.as_bytes(), &l[0]).chars().count(), MAX_COLS);
        assert_eq!(line_text(s.as_bytes(), &l[1]), "яяя");
    }

    #[test]
    fn line_text_expands_tabs_and_keeps_bad_bytes() {
        let b = b"a\tb\xFF";
        assert_eq!(line_text(b, &(0..b.len())), "a    b\u{FFFD}");
    }

    #[test]
    fn width_counts_chars_and_tabs() {
        let b = "a\tя".as_bytes();
        assert_eq!(
            width(b, &(0..b.len())),
            line_text(b, &(0..b.len())).chars().count()
        );
    }

    #[test]
    fn decode_utf16_and_bom() {
        let mut le = vec![0xFF, 0xFE];
        le.extend("Привет".encode_utf16().flat_map(u16::to_le_bytes));
        assert_eq!(decode(&le).unwrap(), "Привет".as_bytes());
        let mut be = vec![0xFE, 0xFF];
        be.extend("ok".encode_utf16().flat_map(u16::to_be_bytes));
        assert_eq!(decode(&be).unwrap(), b"ok");
        assert_eq!(decode(b"\xEF\xBB\xBFhi").unwrap(), b"hi");
        assert_eq!(decode(b"\xFF"), None); // not UTF-16: the bytes as they are
        // A cut-off last unit is shown, not dropped.
        assert_eq!(decode(b"\xFF\xFEo\0k").unwrap(), "o\u{FFFD}".as_bytes());
    }

    #[test]
    fn hex_shows_the_raw_bytes_text_the_decoded_ones() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("u16.txt");
        let mut le = vec![0xFF, 0xFE];
        le.extend("Привет".encode_utf16().flat_map(u16::to_le_bytes));
        std::fs::write(&p, &le).unwrap();
        let d = load(&p).unwrap();
        assert_eq!(d.bytes, le);
        assert_eq!(d.text(), "Привет".as_bytes());
        assert_eq!(detect("u16.txt", d.text()), Mode::Text);
    }

    #[test]
    fn wide_chars_count_double() {
        let b = "a日本🙂".as_bytes();
        assert_eq!(width(b, &(0..b.len())), 7);
    }

    #[test]
    fn hex_row_full_and_partial() {
        let b = b"Hello world\n\0\0\0\0Hi";
        assert_eq!(hex_rows(b.len()), 2);
        assert_eq!(
            hex_row(b, 0, 0),
            "00000000  48 65 6C 6C 6F 20 77 6F  72 6C 64 0A 00 00 00 00  Hello world....."
        );
        assert_eq!(
            hex_row(b, 1, 0),
            format!("00000010  48 69{}  Hi", " ".repeat(3 * 14 + 1))
        );
        assert!(hex_row(b, 1, 0x1_0000_0000).starts_with("100000010  48 69"));
    }

    #[test]
    fn windows_start_at_a_line_and_keep_the_bom() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("a.txt");
        std::fs::write(&p, "one\ntwo\nthree\n").unwrap();
        let w = load_at(&p, 2, u64::MAX, true, Bom::None).unwrap();
        assert_eq!(
            (w.start, w.bytes.as_slice(), w.total),
            (4, &b"two\nthree\n"[..], 14)
        );
        assert!(w.truncated());
        // `from` already a line start: kept
        assert_eq!(load_at(&p, 4, u64::MAX, true, Bom::None).unwrap().start, 4);
        // up to `to`
        assert_eq!(
            load_at(&p, 0, 8, true, Bom::None).unwrap().bytes,
            b"one\ntwo\n"
        );
        // the file got shorter than the window asked for
        assert!(load_at(&p, 100, u64::MAX, true, Bom::None).is_err());
        // hex: from a row start, no line wanted
        assert_eq!(load_at(&p, 5, u64::MAX, false, Bom::None).unwrap().start, 0);
        // no line end after `from`: from there
        assert_eq!(load_at(&p, 9, 12, true, Bom::None).unwrap().start, 9);
        // UTF-16 LE: lines end with "\n\0" on even offsets; later windows decoded as the first
        let le: Vec<u8> = (b"\xff\xfe".iter().copied())
            .chain("а\nб\nв".encode_utf16().flat_map(u16::to_le_bytes))
            .collect();
        std::fs::write(&p, &le).unwrap();
        let first = load(&p).unwrap();
        assert_eq!(first.bom, Bom::Le);
        let w = load_at(&p, 3, u64::MAX, true, first.bom).unwrap();
        assert_eq!((w.start, w.text()), (6, "б\nв".as_bytes()));
        assert_eq!((newlines(&first, 6), last_line_start(&first)), (1, 10));
    }

    #[test]
    fn line_row_counts_wrapped_rows() {
        let doc = Doc {
            bytes: b"abcdef\n\nxy\n".to_vec(),
            ..Default::default()
        };
        let t = Text::new(&doc, Encoding::Auto, Some(3)); // abc def | (empty) | xy
        let row = |k| line_row(t.bytes(&doc), &t.lines, k);
        assert_eq!((row(0), row(1), row(2), row(3)), (0, 2, 3, 4));
        assert_eq!((newlines(&doc, 8), last_line_start(&doc)), (2, 11));
        let one = Doc {
            bytes: b"no end".to_vec(),
            ..Default::default()
        };
        assert_eq!(last_line_start(&one), 6);
    }

    #[test]
    fn find_line_ignores_case_both_ways() {
        let b = "альфа\nБета\nгамма\nбета".as_bytes();
        let l = lines(b, MAX_COLS, false);
        assert_eq!(find_line(b, &l, "бЕТ", 0, true), Some(1));
        assert_eq!(find_line(b, &l, "бет", 2, true), Some(3));
        assert_eq!(find_line(b, &l, "бет", 2, false), Some(1));
        assert_eq!(find_line(b, &l, "бет", 0, false), None);
        assert_eq!(find_line(b, &l, "дельта", 0, true), None);
        assert_eq!(find_line(b, &l, "", 0, true), None);
    }

    #[test]
    fn find_bytes_by_row() {
        let mut b = vec![0u8; 40];
        b[3..5].copy_from_slice(b"ab");
        b[35..37].copy_from_slice(b"ab");
        assert_eq!(find_bytes(&b, b"ab", 0, true), Some(0));
        assert_eq!(find_bytes(&b, b"ab", 1, true), Some(2));
        assert_eq!(find_bytes(&b, b"ab", 1, false), Some(0));
        assert_eq!(find_bytes(&b, b"ab", 2, false), Some(2)); // a match in the start row counts
        assert_eq!(find_bytes(&b, b"zz", 0, true), None);
    }

    #[test]
    fn load_stops_at_limit() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("big");
        let f = std::fs::File::create(&p).unwrap();
        f.set_len(LIMIT + 10).unwrap(); // sparse
        let d = load(&p).unwrap();
        assert_eq!(d.bytes.len() as u64, LIMIT);
        assert_eq!(d.total, LIMIT + 10);
        assert!(d.truncated());
    }

    #[test]
    fn regression_fifo_is_refused_not_read() {
        let d = tempfile::tempdir().unwrap();
        let fifo = d.path().join("p");
        rustix::fs::mknodat(
            rustix::fs::CWD,
            &fifo,
            rustix::fs::FileType::Fifo,
            rustix::fs::Mode::from_raw_mode(0o600),
            0,
        )
        .unwrap();
        // Run in a thread: the bug is a read that never returns.
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || tx.send(load(&fifo).is_err()).unwrap());
        let refused = rx.recv_timeout(std::time::Duration::from_secs(2));
        assert_eq!(refused, Ok(true));
    }

    #[test]
    fn selection_rows_text_and_pointer() {
        let rows = ["hello world", "second", "third line"];
        let row = |i: usize| rows[i].to_string();
        let s = Selection {
            anchor: (2, 5), // dragged upwards: the end comes first
            head: (0, 6),
        };
        assert_eq!(s.ordered(), ((0, 6), (2, 5)));
        assert_eq!(s.in_row(0, 11), Some(6..11));
        assert_eq!(s.in_row(1, 6), Some(0..6));
        assert_eq!(s.in_row(2, 10), Some(0..5));
        assert_eq!(s.in_row(3, 4), None);
        assert_eq!(s.text(row), "world\nsecond\nthird");
        assert!(Selection::new((1, 1)).is_empty());
        assert_eq!(Selection::new((0, 0)).in_row(0, 5), None);
        // past the end of a line: up to its end
        let s = Selection {
            anchor: (1, 2),
            head: (1, 40),
        };
        assert_eq!(s.text(row), "cond");
        // Cyrillic: chars, not bytes
        let s = Selection {
            anchor: (0, 1),
            head: (0, 3),
        };
        assert_eq!(s.text(|_| "привет".to_string()), "ри");
        assert_eq!(pos_at(17.0, 45.0, 20.0, 8.5, 10), (2, 2));
        assert_eq!(pos_at(-5.0, 9999.0, 20.0, 8.5, 10), (9, 0));
    }

    #[test]
    fn double_click_word() {
        let l = "let first_row = скан(42);";
        assert_eq!(word_at(l, 6), Some(4..13)); // first_row
        assert_eq!(word_at(l, 17), Some(16..20)); // Cyrillic, in chars
        assert_eq!(word_at(l, 3), Some(3..4)); // a space: just it
        assert_eq!(word_at(l, 0), Some(0..3));
        assert_eq!(word_at(l, 99), None);
    }
}
