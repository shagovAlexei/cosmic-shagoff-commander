//! The fixed columns of the full view after Name (TC `cm_CustomColumnConfig`, simplified): which
//! of them are shown, kept in the config by `key`, in this order.

/// A column after Name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Col {
    Ext,
    Size,
    Date,
    Attr,
    Owner,
    Group,
}

impl Col {
    pub const ALL: [Col; 6] = [
        Col::Ext,
        Col::Size,
        Col::Date,
        Col::Attr,
        Col::Owner,
        Col::Group,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Col::Ext => "ext",
            Col::Size => "size",
            Col::Date => "date",
            Col::Attr => "attr",
            Col::Owner => "owner",
            Col::Group => "group",
        }
    }

    /// Where its width is kept (`Config.columns`, dragged in the header); `None`: a fixed one.
    pub fn slot(self) -> Option<usize> {
        match self {
            Col::Ext => Some(0),
            Col::Size => Some(1),
            Col::Date => Some(2),
            Col::Attr => Some(3),
            Col::Owner | Col::Group => None,
        }
    }

    /// Width of a column without a slot, px at the default text size.
    pub const FIXED_W: f32 = 90.0;
}

/// The columns named in `keys`, each once, in `Col::ALL` order; unknown names are left out.
pub fn parse(keys: &[String]) -> Vec<Col> {
    Col::ALL
        .into_iter()
        .filter(|c| keys.iter().any(|k| k == c.key()))
        .collect()
}

/// What the config holds when nothing was chosen: the four of TC's default full view.
pub fn default_keys() -> Vec<String> {
    [Col::Ext, Col::Size, Col::Date, Col::Attr]
        .map(|c| c.key().to_string())
        .to_vec()
}

/// `keys` with `col` shown if it was not, else hidden.
pub fn toggled(keys: &[String], col: Col) -> Vec<String> {
    let mut on = parse(keys);
    match on.iter().position(|&c| c == col) {
        Some(i) => {
            on.remove(i);
        }
        None => on.push(col),
    }
    Col::ALL
        .into_iter()
        .filter(|c| on.contains(c))
        .map(|c| c.key().to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn columns_are_parsed_in_order_and_toggled() {
        let keys = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            parse(&keys(&["date", "bogus", "ext", "date"])),
            [Col::Ext, Col::Date]
        );
        assert_eq!(parse(&default_keys()).len(), 4);
        assert_eq!(
            toggled(&keys(&["ext", "size"]), Col::Owner),
            keys(&["ext", "size", "owner"])
        );
        assert_eq!(toggled(&keys(&["ext", "size"]), Col::Ext), keys(&["size"]));
        assert_eq!(toggled(&[], Col::Group), keys(&["group"]));
    }
}
