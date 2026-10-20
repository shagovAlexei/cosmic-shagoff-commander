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

/// Names of these ids, sorted by name, numbers where unknown.
fn named(names: &HashMap<u32, String>, ids: &[u32]) -> Vec<(u32, String)> {
    // Each id once first: a selection of many files repeats them.
    let ids: std::collections::BTreeSet<u32> = ids.iter().copied().collect();
    let mut v: Vec<(u32, String)> = (ids.into_iter())
        .map(|i| (i, names.get(&i).cloned().unwrap_or_else(|| i.to_string())))
        .collect();
    v.sort_by(|a, b| a.1.cmp(&b.1));
    v
}

impl Owners {
    /// Names of these groups, sorted by name, numbers where unknown.
    pub fn groups(&self, gids: &[u32]) -> Vec<(u32, String)> {
        named(&self.groups, gids)
    }

    /// Names of these users, sorted by name, numbers where unknown.
    pub fn users(&self, uids: &[u32]) -> Vec<(u32, String)> {
        named(&self.users, uids)
    }

    pub fn all_uids(&self) -> Vec<u32> {
        self.users.keys().copied().collect()
    }

    pub fn all_gids(&self) -> Vec<u32> {
        self.groups.keys().copied().collect()
    }
}

/// The user this process runs as (0: root, may chown anything).
pub fn euid() -> u32 {
    rustix::process::geteuid().as_raw()
}

/// The groups this process may give a file it owns (its own and the supplementary ones):
/// without root, chgrp to any other fails.
pub fn my_groups() -> Vec<u32> {
    let mut v: Vec<u32> = (rustix::process::getgroups().unwrap_or_default().into_iter())
        .map(|g| g.as_raw())
        .collect();
    v.push(rustix::process::getegid().as_raw());
    v.sort_unstable();
    v.dedup();
    v
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
        assert_eq!(
            o.users(&[1001, 1000, 1000]),
            [(1001, "1001".into()), (1000, "shag".into())]
        );
        assert_eq!(o.all_gids(), [1000]);
    }
}
