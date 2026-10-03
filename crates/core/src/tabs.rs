//! Ordered tabs with exactly one active; never empty.

#[derive(Debug, Clone)]
pub struct Tabs<T> {
    items: Vec<T>,
    active: usize,
}

impl<T> Tabs<T> {
    pub fn new(first: T) -> Self {
        Self {
            items: vec![first],
            active: 0,
        }
    }

    pub fn items(&self) -> &[T] {
        &self.items
    }

    pub fn items_mut(&mut self) -> &mut [T] {
        &mut self.items
    }

    pub fn active_index(&self) -> usize {
        self.active
    }

    pub fn active(&self) -> &T {
        &self.items[self.active]
    }

    pub fn active_mut(&mut self) -> &mut T {
        &mut self.items[self.active]
    }

    /// Insert right after the active tab and activate it.
    pub fn open_after(&mut self, item: T) {
        self.active += 1;
        self.items.insert(self.active, item);
    }

    /// Close tab `i`; refuses the last one. Closing the active tab activates its right neighbour (left if none).
    pub fn close(&mut self, i: usize) -> bool {
        self.take(i).is_some()
    }

    /// Remove tab `i` and hand it over; refuses the last one. The active tab moves as with `close`.
    pub fn take(&mut self, i: usize) -> Option<T> {
        if self.items.len() == 1 || i >= self.items.len() {
            return None;
        }
        let item = self.items.remove(i);
        if i < self.active || self.active == self.items.len() {
            self.active -= 1;
        }
        Some(item)
    }

    /// Keep the tabs `keep` says yes to, and always the active one.
    pub fn retain(&mut self, mut keep: impl FnMut(&T) -> bool) {
        let mut i = 0;
        let active = self.active;
        let mut new_active = 0;
        self.items.retain(|t| {
            let kept = i == active || keep(t);
            if kept && i < active {
                new_active += 1;
            }
            i += 1;
            kept
        });
        self.active = new_active;
    }

    pub fn select(&mut self, i: usize) {
        if i < self.items.len() {
            self.active = i;
        }
    }

    pub fn next(&mut self) {
        self.active = (self.active + 1) % self.items.len();
    }

    pub fn prev(&mut self) {
        self.active = (self.active + self.items.len() - 1) % self.items.len();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tabs 'a', 'b', ... (n of them) with index `active` selected.
    fn tabs(n: usize, active: usize) -> Tabs<char> {
        let mut t = Tabs::new('a');
        for c in (1..n).map(|i| (b'a' + i as u8) as char) {
            t.items.push(c);
        }
        t.active = active;
        t
    }

    #[test]
    fn open_after_inserts_next_to_active_and_activates() {
        let mut t = tabs(3, 0); // [a] b c
        t.open_after('x');
        assert_eq!(t.items(), ['a', 'x', 'b', 'c']);
        assert_eq!(*t.active(), 'x');
    }

    #[test]
    fn close_refuses_last() {
        let mut t = Tabs::new('a');
        assert!(!t.close(0));
        assert_eq!(t.items(), ['a']);
    }

    #[test]
    fn close_out_of_range_is_noop() {
        let mut t = tabs(2, 0);
        assert!(!t.close(5));
        assert_eq!(t.items().len(), 2);
    }

    #[test]
    fn close_active_middle_selects_right() {
        let mut t = tabs(3, 1); // a [b] c
        assert!(t.close(1));
        assert_eq!(t.items(), ['a', 'c']);
        assert_eq!(*t.active(), 'c');
    }

    #[test]
    fn close_active_rightmost_selects_left() {
        let mut t = tabs(3, 2); // a b [c]
        assert!(t.close(2));
        assert_eq!(*t.active(), 'b');
    }

    #[test]
    fn close_left_of_active_keeps_active_tab() {
        let mut t = tabs(3, 2); // a b [c]
        assert!(t.close(0));
        assert_eq!(*t.active(), 'c');
    }

    #[test]
    fn close_right_of_active_keeps_active_tab() {
        let mut t = tabs(3, 0); // [a] b c
        assert!(t.close(2));
        assert_eq!(*t.active(), 'a');
    }

    #[test]
    fn select_ignores_out_of_range() {
        let mut t = tabs(3, 0);
        t.select(2);
        assert_eq!(*t.active(), 'c');
        t.select(9);
        assert_eq!(*t.active(), 'c');
    }

    #[test]
    fn next_prev_wrap() {
        let mut t = tabs(3, 2);
        t.next();
        assert_eq!(*t.active(), 'a');
        t.prev();
        assert_eq!(*t.active(), 'c');
        let mut one = Tabs::new('a');
        one.next();
        one.prev();
        assert_eq!(*one.active(), 'a');
    }

    #[test]
    fn take_hands_over_and_refuses_last() {
        let mut t = tabs(3, 2); // a b [c]
        assert_eq!(t.take(0), Some('a'));
        assert_eq!(t.items(), ['b', 'c']);
        assert_eq!(*t.active(), 'c');
        assert_eq!(t.take(1), Some('c'));
        assert_eq!(*t.active(), 'b');
        assert_eq!(t.take(0), None);
        assert_eq!(t.take(7), None);
    }

    #[test]
    fn retain_keeps_the_active_one() {
        let mut t = tabs(5, 2); // a b [c] d e
        t.retain(|c| *c == 'b' || *c == 'e');
        assert_eq!(t.items(), ['b', 'c', 'e']);
        assert_eq!(*t.active(), 'c');
        t.retain(|_| false);
        assert_eq!(t.items(), ['c']);
        assert_eq!(t.active_index(), 0);
    }
}
