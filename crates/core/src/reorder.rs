//! Reordering a list by dragging an item with the mouse (Ctrl+D «Настроить…», the toolbar).
//! The UI reports which item the pointer is over and when the left button goes down / up.

#[derive(Clone, Debug, Default)]
pub struct Reorder {
    /// The item under the pointer, and the one held by the left button.
    pub hover: Option<usize>,
    pub grab: Option<usize>,
    /// Where the held item just came from: items of other sizes shift after a move, and the one
    /// sliding under the still pointer must not move it back (it would flicker).
    vacated: Option<usize>,
}

impl Reorder {
    /// The pointer is over `at`: a held item moves there. Returns where it went.
    pub fn hover<T>(&mut self, at: Option<usize>, list: &mut Vec<T>) -> Option<usize> {
        self.hover = at;
        if let (Some(from), Some(to)) = (self.grab, at)
            && from != to
            && from < list.len()
            && to < list.len()
            && self.vacated != Some(to)
        {
            let e = list.remove(from);
            list.insert(to, e);
            self.grab = Some(to);
            self.vacated = Some(from);
            return Some(to);
        }
        if at != self.vacated {
            self.vacated = None;
        }
        None
    }

    /// The left button went down: the item under the pointer (if any) is held.
    pub fn grab(&mut self) {
        self.grab = self.hover;
        self.vacated = None;
    }

    pub fn release(&mut self) {
        self.grab = None;
    }

    /// The pointer left `i`: forgotten only if still the hovered one (the enter of a neighbour
    /// and the leave come in either order).
    pub fn left(&mut self, i: usize) {
        if self.hover == Some(i) {
            self.hover = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_held_item_follows_the_pointer() {
        let mut l = vec!["a", "b", "c"];
        let mut r = Reorder::default();
        r.hover(Some(0), &mut l);
        r.grab();
        assert_eq!(r.hover(Some(2), &mut l), Some(2)); // over "c": "a" moves there
        assert_eq!(l, ["b", "c", "a"]);
        r.release();
        assert_eq!(r.hover(Some(0), &mut l), None); // released: hovering moves nothing
        // Moved onto an item of another size: the one sliding under the pointer does not move
        // it back.
        r.grab();
        r.hover(Some(1), &mut l);
        assert_eq!(l, ["c", "b", "a"]);
        r.hover(Some(0), &mut l);
        assert_eq!(l, ["c", "b", "a"]);
        r.release();
        // Pressed outside the items: nothing held.
        r.hover(Some(2), &mut l);
        r.left(2);
        assert_eq!(r.hover, None);
        // Item 1 entered before item 2 says it was left: the late leave keeps item 1.
        r.hover(Some(1), &mut l);
        r.left(2);
        assert_eq!(r.hover, Some(1));
        r.left(1);
        r.grab();
        r.hover(Some(2), &mut l);
        assert_eq!(l, ["c", "b", "a"]);
        // The list shrank under a held item: nothing moves, nothing panics.
        r.hover(Some(0), &mut l);
        r.grab();
        l.clear();
        assert_eq!(r.hover(Some(0), &mut l), None);
    }
}
