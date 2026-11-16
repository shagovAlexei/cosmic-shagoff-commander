//! F3 code colours (231): a line split into keywords, strings, comments and numbers by the
//! file's language (its extension). Per line, no grammar: enough to read code by, no more.
//! ponytail: a block comment is coloured only on the line it opens on (rows are drawn one by
//! one, from anywhere in the file); carry its state if that ever bothers anyone.

use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Keyword,
    Str,
    Comment,
    Number,
}

pub struct Lang {
    /// Line comment openers.
    line: &'static [&'static str],
    /// `/* … */` comments.
    block: bool,
    quotes: &'static [u8],
    keywords: &'static [&'static str],
}

#[rustfmt::skip]
const C_KEYWORDS: &[&str] = &[
    "abstract", "async", "await", "break", "case", "catch", "class", "const", "continue",
    "default", "defer", "delete", "do", "else", "enum", "export", "extends", "false", "final",
    "finally", "for", "func", "function", "go", "goto", "if", "implements", "import", "in",
    "instanceof", "interface", "internal", "let", "map", "namespace", "new", "null", "nullptr",
    "override", "package", "private", "protected", "public", "return", "sizeof", "static",
    "struct", "super", "switch", "this", "throw", "throws", "true", "try", "type", "typedef",
    "typeof", "union", "using", "var", "virtual", "void", "volatile", "while", "yield",
    "#include", "#define", "#ifdef", "#ifndef", "#endif", "#if", "#else", "fun", "val", "when",
    "object", "data", "chan", "select", "range", "undefined", "of", "as", "is",
];
#[rustfmt::skip]
const RUST_KEYWORDS: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum",
    "extern", "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move",
    "mut", "pub", "ref", "return", "self", "Self", "static", "struct", "super", "trait", "true",
    "type", "unsafe", "use", "where", "while", "Some", "None", "Ok", "Err",
];
#[rustfmt::skip]
const SCRIPT_KEYWORDS: &[&str] = &[
    "and", "as", "assert", "async", "await", "begin", "break", "case", "class", "def", "del",
    "do", "done", "elif", "else", "elsif", "end", "esac", "except", "export", "false", "False",
    "fi", "finally", "for", "from", "function", "global", "if", "import", "in", "is", "lambda",
    "local", "module", "nil", "None", "nonlocal", "not", "or", "pass", "raise", "require",
    "return", "self", "then", "true", "True", "try", "unless", "until", "while", "with",
    "yield", "echo", "set", "unset", "readonly", "shift", "source",
];
#[rustfmt::skip]
const SQL_KEYWORDS: &[&str] = &[
    "select", "from", "where", "insert", "into", "values", "update", "delete", "create",
    "table", "drop", "alter", "and", "or", "not", "null", "join", "left", "right", "inner",
    "on", "group", "by", "order", "having", "limit", "as", "primary", "key", "index", "SELECT",
    "FROM", "WHERE", "INSERT", "INTO", "VALUES", "UPDATE", "DELETE", "CREATE", "TABLE", "DROP",
    "ALTER", "AND", "OR", "NOT", "NULL", "JOIN", "LEFT", "RIGHT", "INNER", "ON", "GROUP", "BY",
    "ORDER", "HAVING", "LIMIT", "AS", "PRIMARY", "KEY", "INDEX",
];
#[rustfmt::skip]
const LUA_KEYWORDS: &[&str] = &[
    "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "if", "in",
    "local", "nil", "not", "or", "repeat", "return", "then", "true", "until", "while",
];
const DATA_KEYWORDS: &[&str] = &["true", "false", "null", "yes", "no", "on", "off"];

const RUST: Lang = Lang {
    line: &["//"],
    block: true,
    quotes: b"\"", // ' opens a lifetime as often as a char
    keywords: RUST_KEYWORDS,
};
const C: Lang = Lang {
    line: &["//"],
    block: true,
    quotes: b"\"'`",
    keywords: C_KEYWORDS,
};
const SCRIPT: Lang = Lang {
    line: &["#"],
    block: false,
    quotes: b"\"'",
    keywords: SCRIPT_KEYWORDS,
};
const SQL: Lang = Lang {
    line: &["--"],
    block: true,
    quotes: b"'\"",
    keywords: SQL_KEYWORDS,
};
const LUA: Lang = Lang {
    line: &["--"],
    block: false,
    quotes: b"\"'",
    keywords: LUA_KEYWORDS,
};
const DATA: Lang = Lang {
    line: &["#", ";"],
    block: false,
    quotes: b"\"'",
    keywords: DATA_KEYWORDS,
};
const CSS: Lang = Lang {
    line: &[],
    block: true,
    quotes: b"\"'",
    keywords: &[],
};

/// The language of a file named `name`, if we colour it.
pub fn lang(name: &str) -> Option<&'static Lang> {
    let lower = name.to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "makefile" | "dockerfile" | "pkgbuild" | "justfile"
    ) {
        return Some(&SCRIPT);
    }
    let ext = lower.rsplit_once('.')?.1;
    Some(match ext {
        "rs" => &RUST,
        "c" | "h" | "cc" | "cpp" | "cxx" | "hpp" | "hh" | "cs" | "java" | "js" | "mjs" | "cjs"
        | "ts" | "tsx" | "jsx" | "go" | "kt" | "kts" | "swift" | "dart" | "scala" | "php"
        | "groovy" | "gradle" | "zig" | "v" | "qml" => &C,
        "py" | "pyw" | "sh" | "bash" | "zsh" | "fish" | "rb" | "pl" | "pm" | "r" | "cmake"
        | "nix" | "ps1" | "tcl" | "awk" | "mk" => &SCRIPT,
        "sql" | "hs" => &SQL,
        "lua" => &LUA,
        "toml" | "yaml" | "yml" | "ini" | "conf" | "cfg" | "json" | "jsonc" | "desktop"
        | "service" | "env" | "properties" => &DATA,
        "css" | "scss" | "less" => &CSS,
        _ => return None,
    })
}

fn ident(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// The coloured byte ranges of `line` (ascending, apart); the rest is plain.
pub fn spans(line: &str, lang: &Lang) -> Vec<(Range<usize>, Kind)> {
    let b = line.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let rest = &line[i..];
        if lang.line.iter().any(|m| rest.starts_with(m)) {
            out.push((i..b.len(), Kind::Comment));
            break;
        }
        if lang.block && rest.starts_with("/*") {
            let end = rest[2..].find("*/").map_or(b.len(), |e| i + 2 + e + 2);
            out.push((i..end, Kind::Comment));
            i = end;
            continue;
        }
        let c = b[i];
        if lang.quotes.contains(&c) {
            let mut j = i + 1;
            while j < b.len() && b[j] != c {
                j += if b[j] == b'\\' { 2 } else { 1 };
            }
            let end = (j + 1).min(b.len());
            out.push((i..end, Kind::Str));
            i = end;
        } else if c.is_ascii_digit() {
            let end = (i..b.len())
                .find(|&j| !(ident(b[j]) || b[j] == b'.'))
                .unwrap_or(b.len());
            out.push((i..end, Kind::Number));
            i = end;
        } else if ident(c) || c == b'#' {
            let end = (i + 1..b.len()).find(|&j| !ident(b[j])).unwrap_or(b.len());
            if lang.keywords.contains(&&line[i..end]) {
                out.push((i..end, Kind::Keyword));
            }
            i = end;
        } else {
            i += 1; // ASCII punctuation, or a byte of a non-ASCII char: never a cut inside one
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shown(line: &str, name: &str) -> Vec<(&'static str, String)> {
        let l = lang(name).unwrap();
        spans(line, l)
            .into_iter()
            .map(|(r, k)| {
                let k = match k {
                    Kind::Keyword => "k",
                    Kind::Str => "s",
                    Kind::Comment => "c",
                    Kind::Number => "n",
                };
                (k, line[r].to_string())
            })
            .collect()
    }

    #[test]
    fn languages_by_extension() {
        assert!(lang("main.RS").is_some());
        assert!(lang("Makefile").is_some());
        assert!(lang("notes.txt").is_none());
        assert!(lang("noext").is_none());
    }

    #[test]
    fn rust_line() {
        assert_eq!(
            shown(r#"let s = "a \" b"; // x 1"#, "a.rs"),
            [
                ("k", "let".into()),
                ("s", r#""a \" b""#.into()),
                ("c", "// x 1".into())
            ]
        );
        // A lifetime is not a string; a number inside a name is not a number.
        assert_eq!(
            shown("fn f<'a>(x2: u8) { 0x1F }", "a.rs"),
            [("k", "fn".into()), ("n", "0x1F".into())]
        );
    }

    #[test]
    fn script_and_block_comments() {
        assert_eq!(
            shown("if x: return 'привет' # да", "a.py"),
            [
                ("k", "if".into()),
                ("k", "return".into()),
                ("s", "'привет'".into()),
                ("c", "# да".into())
            ]
        );
        assert_eq!(
            shown("a /* b */ c /* open", "a.c"),
            [("c", "/* b */".into()), ("c", "/* open".into())]
        );
        assert_eq!(
            shown("x = \"unclosed", "a.toml"),
            [("s", "\"unclosed".into())]
        );
        assert_eq!(shown("#include <x>", "a.c"), [("k", "#include".into())]);
    }
}
