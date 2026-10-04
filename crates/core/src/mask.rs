//! TC file mask: `*.rs;*.toml|*.bak` — patterns split by `;` or spaces (`"a b*"` quoted), exclusions after `|`, `*` and `?`, case-insensitive.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mask {
    include: Vec<String>,
    exclude: Vec<String>,
}

impl Mask {
    pub fn parse(s: &str) -> Self {
        // The first `|` outside quotes starts the exclusions.
        let mut quoted = false;
        let bar = s.char_indices().find(|&(_, c)| {
            quoted ^= c == '"';
            c == '|' && !quoted
        });
        let (inc, exc) = bar.map_or((s, ""), |(i, _)| (&s[..i], &s[i + 1..]));
        let split = |part: &str| -> Vec<String> {
            tokens(part)
                .into_iter()
                .map(|p| p.to_lowercase())
                // TC: `*.*` means every file, including names without a dot
                .map(|p| if p == "*.*" { "*".to_string() } else { p })
                .collect()
        };
        let mut include = split(inc);
        if include.is_empty() {
            include.push("*".into());
        }
        Self {
            include,
            exclude: split(exc),
        }
    }

    pub fn matches(&self, name: &str) -> bool {
        let name = name.to_lowercase();
        self.include.iter().any(|p| glob(p, &name)) && !self.exclude.iter().any(|p| glob(p, &name))
    }
}

/// Patterns split by `;` or whitespace, as in TC; `"..."` keeps spaces inside one pattern.
fn tokens(s: &str) -> Vec<String> {
    let (mut out, mut cur, mut quoted) = (Vec::new(), String::new(), false);
    for c in s.chars() {
        match c {
            '"' => quoted = !quoted,
            ';' if !quoted => out.push(std::mem::take(&mut cur)),
            c if c.is_whitespace() && !quoted => out.push(std::mem::take(&mut cur)),
            c => cur.push(c),
        }
    }
    out.push(cur);
    out.retain(|t| !t.is_empty());
    out
}

/// `*` = any run, `?` = one char. Greedy with backtracking to the last `*`.
pub(crate) fn glob(pattern: &str, name: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let n: Vec<char> = name.chars().collect();
    let (mut pi, mut ni) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while ni < n.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == n[ni]) {
            pi += 1;
            ni += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some((pi, ni));
            pi += 1;
        } else if let Some((sp, sn)) = star {
            pi = sp + 1;
            ni = sn + 1;
            star = Some((sp, sn + 1));
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|&c| c == '*')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regression_separators_inside_quotes_are_literal() {
        assert!(m("\"a;b*\"", "a;b.txt"));
        assert!(!m("\"a;b*\"", "b.txt"));
        assert!(m("\"x|y\"", "x|y"));
        assert!(!m("*|\"x|y\"", "x|y"));
        assert!(m("*|\"x|y\"", "x"));
    }

    fn m(mask: &str, name: &str) -> bool {
        Mask::parse(mask).matches(name)
    }

    #[test]
    fn space_separates_patterns_like_tc() {
        assert!(m("*.rs *.toml", "a.rs"));
        assert!(m("*.rs *.toml", "Cargo.toml"));
        assert!(!m("*.rs *.toml", "a.md"));
        assert!(m("*.rs; *.toml|*.bak tmp*", "a.rs"));
        assert!(!m("*.* |*.bak tmp*", "tmp1"));
    }

    #[test]
    fn quotes_keep_spaces_in_a_pattern() {
        assert!(m("\"my file*\"", "my file.txt"));
        assert!(!m("\"my file*\"", "my"));
        assert!(!m("\"my file*\"", "file.txt"));
    }

    #[test]
    fn star_and_question() {
        assert!(m("*.rs", "main.rs"));
        assert!(!m("*.rs", "main.rsx"));
        assert!(m("a?c", "abc"));
        assert!(!m("a?c", "ac"));
        assert!(m("*", "anything"));
        assert!(m("a*b*c", "aXXbYYc"));
        assert!(!m("a*b*c", "aXXbYY"));
    }

    #[test]
    fn case_insensitive() {
        assert!(m("*.RS", "Main.rs"));
        assert!(m("readme*", "README.md"));
    }

    #[test]
    fn several_patterns_and_exclusions() {
        assert!(m("*.rs;*.toml", "Cargo.toml"));
        assert!(!m("*.rs;*.toml", "Cargo.lock"));
        assert!(m("*|*.bak", "a.txt"));
        assert!(!m("*|*.bak", "a.bak"));
        assert!(!m("*.rs|test*", "test_main.rs"));
    }

    #[test]
    fn star_dot_star_matches_names_without_dot() {
        assert!(m("*.*", "Makefile"));
        assert!(m("*.*", "a.b"));
    }

    #[test]
    fn empty_mask_means_all() {
        assert!(m("", "x"));
        assert!(m("  ", "x"));
        assert!(m("|*.bak", "x.txt"));
    }
}
