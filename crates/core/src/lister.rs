//! F3 viewer (TC's Lister): mode detection, line index, hex rows, search.

use std::io::{self, Read};
use std::ops::Range;
use std::path::Path;

/// Read at most this much of a file.
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

#[derive(Debug, PartialEq, Eq)]
pub struct Doc {
    /// The first `LIMIT` bytes as they are in the file (hex shows these).
    pub bytes: Vec<u8>,
    /// The same as UTF-8 when the file has a BOM (UTF-16 or UTF-8), for text mode.
    pub decoded: Option<Vec<u8>>,
    /// Size of the whole file.
    pub total: u64,
}

impl Doc {
    pub fn truncated(&self) -> bool {
        self.total > LIMIT
    }

    /// Bytes for text mode and its line index.
    pub fn text(&self) -> &[u8] {
        self.decoded.as_deref().unwrap_or(&self.bytes)
    }
}

pub fn load(path: &Path) -> io::Result<Doc> {
    // A fifo would block `open` forever, a device never ends: regular files only.
    let meta = std::fs::metadata(path)?;
    if !meta.is_file() {
        let msg = format!("{}: not a regular file", path.display());
        return Err(io::Error::new(io::ErrorKind::InvalidInput, msg));
    }
    let file = std::fs::File::open(path)?;
    let mut bytes = Vec::new();
    file.take(LIMIT).read_to_end(&mut bytes)?;
    // /proc files report size 0 but have content.
    let total = meta.len().max(bytes.len() as u64);
    Ok(Doc {
        decoded: decode(&bytes),
        bytes,
        total,
    })
}

/// UTF-16 with a BOM → UTF-8; a UTF-8 BOM dropped. `None`: no BOM, the bytes are the text.
pub fn decode(bytes: &[u8]) -> Option<Vec<u8>> {
    let utf16 = |be: bool| {
        let (pairs, rest) = bytes[2..].as_chunks::<2>();
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
    };
    match bytes {
        [0xEF, 0xBB, 0xBF, rest @ ..] => Some(rest.to_vec()),
        [0xFF, 0xFE, ..] => Some(utf16(false)),
        [0xFE, 0xFF, ..] => Some(utf16(true)),
        _ => None,
    }
}

/// Text mode encoding (TC: A = ANSI, S = DOS; K for KOI8-R; 8 back to UTF-8 / BOM).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Encoding {
    /// UTF-8, or UTF-16 / UTF-8 by BOM.
    #[default]
    Auto,
    Cp1251,
    Koi8r,
    Cp866,
}

// Bytes 0x80..=0xFF of each single-byte code page (generated from Python's codecs; 0x98 in cp1251
// is unassigned → U+FFFD).
const CP1251: &str = "ЂЃ‚ѓ„…†‡€‰Љ‹ЊЌЋЏђ‘’“”•–—�™љ›њќћџ\u{a0}ЎўЈ¤Ґ¦§Ё©Є«¬\u{ad}®Ї°±Ііґµ¶·ё№є»јЅѕїАБВГДЕЖЗИЙКЛМНОПРСТУФХЦЧШЩЪЫЬЭЮЯабвгдежзийклмнопрстуфхцчшщъыьэюя";
const KOI8R: &str = "─│┌┐└┘├┤┬┴┼▀▄█▌▐░▒▓⌠■∙√≈≤≥\u{a0}⌡°²·÷═║╒ё╓╔╕╖╗╘╙╚╛╜╝╞╟╠╡Ё╢╣╤╥╦╧╨╩╪╫╬©юабцдефгхийклмнопярстужвьызшэщчъЮАБЦДЕФГХИЙКЛМНОПЯРСТУЖВЬЫЗШЭЩЧЪ";
const CP866: &str = "АБВГДЕЖЗИЙКЛМНОПРСТУФХЦЧШЩЪЫЬЭЮЯабвгдежзийклмноп░▒▓│┤╡╢╖╕╣║╗╝╜╛┐└┴┬├─┼╞╟╚╔╩╦╠═╬╧╨╤╥╙╘╒╓╫╪┘┌█▄▌▐▀рстуфхцчшщъыьэюяЁёЄєЇїЎў°∙·√№¤■\u{a0}";

/// The text as UTF-8 in `enc`; `None` for `Auto` without a BOM (the bytes are the text).
pub fn decode_as(bytes: &[u8], enc: Encoding) -> Option<Vec<u8>> {
    let table = match enc {
        Encoding::Auto => return decode(bytes),
        Encoding::Cp1251 => CP1251,
        Encoding::Koi8r => KOI8R,
        Encoding::Cp866 => CP866,
    };
    let high: Vec<char> = table.chars().collect();
    let s: String = bytes
        .iter()
        .map(|&b| {
            if b < 0x80 {
                b as char
            } else {
                high[b as usize - 0x80]
            }
        })
        .collect();
    Some(s.into_bytes())
}

/// Encoding a file most likely is in: valid UTF-8 (or a BOM) → `Auto`; otherwise the Cyrillic code
/// page in which the first 64 KiB read as the most lowercase Russian letters (running text is
/// mostly lowercase; in the wrong page those bytes turn into capitals or box drawing).
pub fn guess(bytes: &[u8]) -> Encoding {
    let sample = &bytes[..bytes.len().min(64 << 10)];
    let valid = match std::str::from_utf8(sample) {
        Ok(_) => true,
        // Only a char cut at the end of the sample.
        Err(e) => e.error_len().is_none(),
    };
    if valid || decode(bytes).is_some() {
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
    [Encoding::Cp866, Encoding::Koi8r, Encoding::Cp1251]
        .into_iter()
        .map(|e| (score(e), e))
        .filter(|&(n, _)| n > 0 && n * 3 >= high)
        .max_by_key(|&(n, _)| n)
        .map_or(Encoding::Auto, |(_, e)| e)
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
    /// `wrap`: cut lines every this many chars (TC `W`), else at `MAX_COLS`.
    pub fn new(doc: &Doc, enc: Encoding, wrap: Option<usize>) -> Self {
        let decoded = match enc {
            Encoding::Auto => doc.decoded.clone(),
            _ => decode_as(&doc.bytes, enc),
        };
        let bytes = decoded.as_deref().unwrap_or(&doc.bytes);
        let lines = lines(bytes, wrap.unwrap_or(MAX_COLS).clamp(1, MAX_COLS));
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

pub fn is_image(name: &str) -> bool {
    name.rsplit_once('.')
        .is_some_and(|(_, ext)| IMAGES.contains(&ext.to_ascii_lowercase().as_str()))
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

/// Byte ranges of the lines (without `\n` / `\r\n`), lines wider than `max_cols` cells split.
// ponytail: wrap cuts at the cell limit, not at word boundaries.
pub fn lines(bytes: &[u8], max_cols: usize) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut push = |s: usize, mut e: usize| {
        if e > s && bytes[e - 1] == b'\r' {
            e -= 1;
        }
        split_long(bytes, s, e, max_cols, &mut out);
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

/// Cut `s..e` every `max` chars, at char starts (UTF-8 continuation bytes never start a piece).
fn split_long(bytes: &[u8], s: usize, e: usize, max: usize, out: &mut Vec<Range<usize>>) {
    // Same cells as `line_text` / `width` draw: a tab is four, wide chars two, each invalid
    // UTF-8 sequence one `�`.
    let mut from = s;
    let mut cells = 0;
    let mut at = s;
    let mut put = |i: usize, w: usize, from: &mut usize, cells: &mut usize| {
        if *cells + w > max && *cells > 0 {
            out.push(*from..i);
            *from = i;
            *cells = 0;
        }
        *cells += w;
    };
    for chunk in bytes[s..e].utf8_chunks() {
        for (i, c) in chunk.valid().char_indices() {
            let w = match c {
                '\t' => 4,
                c if wide(c) => 2,
                _ => 1,
            };
            put(at + i, w, &mut from, &mut cells);
        }
        at += chunk.valid().len();
        if !chunk.invalid().is_empty() {
            put(at, 1, &mut from, &mut cells);
            at += chunk.invalid().len();
        }
    }
    out.push(from..e);
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

/// `00000010  48 65 6C 6C 6F 20 77 6F  72 6C 64 0A 00 00 00 00  Hello world.....`
pub fn hex_row(bytes: &[u8], row: usize) -> String {
    let start = row * HEX_WIDTH;
    let chunk = &bytes[start.min(bytes.len())..(start + HEX_WIDTH).min(bytes.len())];
    let mut s = format!("{start:08X} ");
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

#[cfg(test)]
mod tests {
    use super::*;

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
        for t in [CP1251, KOI8R, CP866] {
            assert_eq!(t.chars().count(), 128);
        }
    }

    #[test]
    fn guess_picks_the_code_page_of_russian_text() {
        let text = "Съешь же ещё этих мягких французских булок, да выпей чаю";
        let enc = |codec: &[u8]| guess(codec);
        assert_eq!(enc(text.as_bytes()), Encoding::Auto);
        assert_eq!(enc(b"plain ascii"), Encoding::Auto);
        // the same sentence in each page (bytes from Python's codecs)
        let cp1251: Vec<u8> = text.chars().map(|c| encode(CP1251, c)).collect();
        let koi8: Vec<u8> = text.chars().map(|c| encode(KOI8R, c)).collect();
        let cp866: Vec<u8> = text.chars().map(|c| encode(CP866, c)).collect();
        assert_eq!(enc(&cp1251), Encoding::Cp1251);
        assert_eq!(enc(&koi8), Encoding::Koi8r);
        assert_eq!(enc(&cp866), Encoding::Cp866);
        assert_eq!(enc(b"\xff\xfe\x00\x00"), Encoding::Auto); // UTF-16 BOM
        // ties go to 1251; Western accents stay as they were
        assert_eq!(enc(b"\xe4\xe0"), Encoding::Cp1251); // "да"
        assert_eq!(enc(b"caf\xe9 na\xefve G\xf6\xdfe"), Encoding::Auto);
        // a UTF-8 char cut at the end of the sample is still UTF-8
        assert_eq!(enc(&"я".as_bytes()[..1]), Encoding::Auto);
    }

    /// Test helper: the byte of `c` in a code page table.
    fn encode(table: &str, c: char) -> u8 {
        if c.is_ascii() {
            return c as u8;
        }
        0x80 + table.chars().position(|t| t == c).expect("char in page") as u8
    }

    #[test]
    fn text_wraps_and_switches_encoding() {
        let doc = Doc {
            bytes: b"abcdefg\n\xcf\xf0".to_vec(),
            decoded: None,
            total: 10,
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
        };
        let t = Text::new(&tabs, Encoding::Auto, Some(5));
        let shown: Vec<String> = t.lines.iter().map(|r| line_text(&tabs.bytes, r)).collect();
        assert_eq!(shown, ["    a", "    b", "\u{fffd}cd"]);
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
    fn lines_split_on_lf_and_crlf() {
        let b = b"one\r\ntwo\n\nlast";
        let l = lines(b, MAX_COLS);
        let text: Vec<String> = l.iter().map(|r| line_text(b, r)).collect();
        assert_eq!(text, ["one", "two", "", "last"]);
        assert_eq!(lines(b"a\n", MAX_COLS).len(), 1);
        assert!(lines(b"", MAX_COLS).is_empty());
    }

    #[test]
    fn long_line_split_at_char_boundary() {
        let s = "я".repeat(MAX_COLS + 3);
        let l = lines(s.as_bytes(), MAX_COLS);
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
            hex_row(b, 0),
            "00000000  48 65 6C 6C 6F 20 77 6F  72 6C 64 0A 00 00 00 00  Hello world....."
        );
        assert_eq!(
            hex_row(b, 1),
            format!("00000010  48 69{}  Hi", " ".repeat(3 * 14 + 1))
        );
    }

    #[test]
    fn find_line_ignores_case_both_ways() {
        let b = "альфа\nБета\nгамма\nбета".as_bytes();
        let l = lines(b, MAX_COLS);
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
}
