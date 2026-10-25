//! A flat list with "submenu starts" / "submenu ends" markers as a tree (Start menu, TC's
//! `-Name` … `--` lines of `usermenu`).

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    Item,
    Open,
    Close,
}

/// A node: the index of its line in the flat list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Node {
    Item(usize),
    /// The `Open` line and what is inside it.
    Sub(usize, Vec<Node>),
}

/// The tree of `marks`: a submenu not closed ends with the list, a `Close` with none open is dropped.
pub fn nest(marks: &[Mark]) -> Vec<Node> {
    let mut stack: Vec<(usize, Vec<Node>)> = vec![(usize::MAX, Vec::new())];
    for (i, m) in marks.iter().enumerate() {
        match m {
            Mark::Item => stack.last_mut().unwrap().1.push(Node::Item(i)),
            Mark::Open => stack.push((i, Vec::new())),
            Mark::Close if stack.len() > 1 => {
                let (at, sub) = stack.pop().unwrap();
                stack.last_mut().unwrap().1.push(Node::Sub(at, sub));
            }
            Mark::Close => {}
        }
    }
    while stack.len() > 1 {
        let (at, sub) = stack.pop().unwrap();
        stack.last_mut().unwrap().1.push(Node::Sub(at, sub));
    }
    stack.pop().unwrap().1
}

/// The other bound of the submenu whose `Open` or `Close` is line `i` (`None`: an item, or a bound
/// with no partner).
pub fn partner(marks: &[Mark], i: usize) -> Option<usize> {
    let mut open = Vec::new();
    for (j, m) in marks.iter().enumerate() {
        match m {
            Mark::Open => open.push(j),
            Mark::Close => match open.pop() {
                Some(o) if o == i => return Some(j),
                Some(o) if j == i => return Some(o),
                _ => {}
            },
            Mark::Item => {}
        }
    }
    None
}

/// How deep line `i` sits (an `Open` / `Close` line at its submenu's own level), for the editor.
pub fn depth(marks: &[Mark], i: usize) -> usize {
    let mut d = 0usize;
    for m in &marks[..i] {
        match m {
            Mark::Open => d += 1,
            Mark::Close => d = d.saturating_sub(1),
            Mark::Item => {}
        }
    }
    match marks[i] {
        Mark::Close => d.saturating_sub(1),
        _ => d,
    }
}

#[cfg(test)]
mod tests {
    use super::Mark::*;
    use super::*;

    #[test]
    fn submenus_nest() {
        let m = [Item, Open, Item, Open, Item, Close, Close, Item];
        assert_eq!(
            nest(&m),
            vec![
                Node::Item(0),
                Node::Sub(1, vec![Node::Item(2), Node::Sub(3, vec![Node::Item(4)])]),
                Node::Item(7),
            ]
        );
        assert_eq!(
            (0..m.len()).map(|i| depth(&m, i)).collect::<Vec<_>>(),
            [0, 0, 1, 1, 2, 1, 0, 0]
        );
    }

    #[test]
    fn a_bound_knows_its_partner() {
        let m = [Open, Item, Open, Close, Close, Close];
        assert_eq!(partner(&m, 0), Some(4));
        assert_eq!(partner(&m, 4), Some(0));
        assert_eq!(partner(&m, 3), Some(2));
        assert_eq!(partner(&m, 1), None);
        assert_eq!(partner(&m, 5), None); // a stray end
        assert_eq!(partner(&[Open, Item], 0), None);
    }

    #[test]
    fn unbalanced_marks_still_make_a_tree() {
        assert_eq!(nest(&[Close, Item]), vec![Node::Item(1)]);
        assert_eq!(nest(&[Open, Item]), vec![Node::Sub(0, vec![Node::Item(1)])]);
        assert_eq!(depth(&[Close, Item], 1), 0);
    }
}
