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
    /// The first `LIMIT` bytes, UTF-16 already turned into UTF-8.
    pub bytes: Vec<u8>,
    /// Size of the whole file.
    pub total: u64,
}

impl Doc {
    pub fn truncated(&self) -> bool {
        self.total > LIMIT
    }
}

pub fn load(path: &Path) -> io::Result<Doc> {
    let file = std::fs::File::open(path)?;
    let total = file.metadata()?.len();
    let mut bytes = Vec::new();
    file.take(LIMIT).read_to_end(&mut bytes)?;
    Ok(Doc {
        bytes: decode(bytes),
        total,
    })
}

/// UTF-16 with a BOM → UTF-8; a UTF-8 BOM is dropped. Anything else is kept as is.
pub fn decode(bytes: Vec<u8>) -> Vec<u8> {
    let utf16 = |be: bool| {
        let units = bytes[2..].chunks_exact(2).map(|c| {
            let pair = [c[0], c[1]];
            if be {
                u16::from_be_bytes(pair)
            } else {
                u16::from_le_bytes(pair)
            }
        });
        char::decode_utf16(units)
            .map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER))
            .collect::<String>()
            .into_bytes()
    };
    match bytes.get(..3) {
        Some([0xEF, 0xBB, 0xBF]) => bytes[3..].to_vec(),
        _ => match bytes.get(..2) {
            Some([0xFF, 0xFE]) => utf16(false),
            Some([0xFE, 0xFF]) => utf16(true),
            _ => bytes,
        },
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

/// Byte ranges of the lines (without `\n` / `\r\n`), lines over `MAX_COLS` chars split.
pub fn lines(bytes: &[u8]) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut push = |s: usize, mut e: usize| {
        if e > s && bytes[e - 1] == b'\r' {
            e -= 1;
        }
        split_long(bytes, s, e, &mut out);
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

/// Cut `s..e` every `MAX_COLS` chars, at char starts (UTF-8 continuation bytes never start a piece).
fn split_long(bytes: &[u8], s: usize, e: usize, out: &mut Vec<Range<usize>>) {
    let mut from = s;
    let mut chars = 0;
    for i in s..e {
        if bytes[i] & 0xC0 != 0x80 {
            if chars == MAX_COLS {
                out.push(from..i);
                from = i;
                chars = 0;
            }
            chars += 1;
        }
    }
    out.push(from..e);
}

/// A line as shown: invalid UTF-8 as `�`, tabs as four spaces.
pub fn line_text(bytes: &[u8], r: &Range<usize>) -> String {
    String::from_utf8_lossy(&bytes[r.clone()]).replace('\t', "    ")
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
        let l = lines(b);
        let text: Vec<String> = l.iter().map(|r| line_text(b, r)).collect();
        assert_eq!(text, ["one", "two", "", "last"]);
        assert_eq!(lines(b"a\n").len(), 1);
        assert!(lines(b"").is_empty());
    }

    #[test]
    fn long_line_split_at_char_boundary() {
        let s = "я".repeat(MAX_COLS + 3);
        let l = lines(s.as_bytes());
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
    fn decode_utf16_and_bom() {
        let mut le = vec![0xFF, 0xFE];
        le.extend("Привет".encode_utf16().flat_map(u16::to_le_bytes));
        assert_eq!(decode(le), "Привет".as_bytes());
        let mut be = vec![0xFE, 0xFF];
        be.extend("ok".encode_utf16().flat_map(u16::to_be_bytes));
        assert_eq!(decode(be), b"ok");
        assert_eq!(decode(b"\xEF\xBB\xBFhi".to_vec()), b"hi");
        assert_eq!(decode(b"\xFF".to_vec()), b"\xFF");
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
        let l = lines(b);
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
}
