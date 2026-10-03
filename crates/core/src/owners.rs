//! User and group names for the status bar, from `/etc/passwd` and `/etc/group`.

use std::collections::HashMap;

#[derive(Clone, Debug, Default)]
pub struct Owners {
    users: HashMap<u32, String>,
    groups: HashMap<u32, String>,
}

/// `name:x:id:…` lines → id → name (passwd and group share the layout).
pub fn parse(text: &str) -> HashMap<u32, String> {
    text.lines()
        .filter_map(|l| {
            let mut f = l.split(':');
            let name = f.next()?;
            let id = f.nth(1)?.parse().ok()?;
            Some((id, name.to_string()))
        })
        .collect()
}

impl Owners {
    /// Read once at start. ponytail: local files only; LDAP/NIS users show as numbers.
    pub fn load() -> Self {
        let read = |p| {
            std::fs::read_to_string(p)
                .map(|t| parse(&t))
                .unwrap_or_default()
        };
        Owners {
            users: read("/etc/passwd"),
            groups: read("/etc/group"),
        }
    }

    /// `user:group`, numbers where the name is unknown.
    pub fn name(&self, uid: u32, gid: u32) -> String {
        let user = self
            .users
            .get(&uid)
            .cloned()
            .unwrap_or_else(|| uid.to_string());
        let group = self
            .groups
            .get(&gid)
            .cloned()
            .unwrap_or_else(|| gid.to_string());
        format!("{user}:{group}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_passwd() {
        let m = parse(
            "root:x:0:0:root:/root:/bin/bash\n# comment\nbad line\nshag:x:1000:1000::/home/shag:/bin/zsh\n",
        );
        assert_eq!(m.get(&0).map(String::as_str), Some("root"));
        assert_eq!(m.get(&1000).map(String::as_str), Some("shag"));
        assert_eq!(m.len(), 2);
    }

    #[test]
    fn names_fall_back_to_numbers() {
        let o = Owners {
            users: parse("shag:x:1000:1000::/:/bin/sh\n"),
            groups: parse("shag:x:1000:\n"),
        };
        assert_eq!(o.name(1000, 1000), "shag:shag");
        assert_eq!(o.name(1001, 5), "1001:5");
    }
}
