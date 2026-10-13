//! Configuration → "Colors by type…" (TC "Color by file type"): the rules, first match wins.
//! Edits a copy, saved on OK.

use crate::app::Message;
use crate::fl;
use cosmic::iced::widget::{column, row};
use cosmic::iced::{Alignment, Color, Length};
use cosmic::{Element, theme, widget};
use shagoff_core::colors::{self, Paint, Rule, What};

#[derive(Clone, Debug)]
pub struct ColorEdit {
    pub list: Vec<Rule>,
    pub sel: Option<usize>,
}

#[derive(Clone, Debug)]
pub enum ColorMsg {
    Select(usize),
    Mask(String),
    /// Index into `What::ALL`.
    What(usize),
    Color(String),
    /// After the selection: every file, a colour to change.
    Add,
    Delete,
    Up,
    Down,
    /// Back to `colors::defaults()`.
    Reset,
}

impl ColorEdit {
    pub fn new(list: Vec<Rule>) -> Self {
        let sel = (!list.is_empty()).then_some(0);
        Self { list, sel }
    }

    pub fn update(&mut self, m: ColorMsg) {
        let sel = self.sel.filter(|&i| i < self.list.len());
        match (m, sel) {
            (ColorMsg::Select(i), _) => self.sel = (i < self.list.len()).then_some(i),
            (ColorMsg::Mask(s), Some(i)) => self.list[i].mask = s,
            (ColorMsg::What(w), Some(i)) => self.list[i].what = What::ALL[w.min(4)],
            (ColorMsg::Color(s), Some(i)) => self.list[i].color = s,
            (ColorMsg::Add, _) => {
                let at = sel.map_or(self.list.len(), |i| i + 1);
                self.list.insert(
                    at,
                    Rule {
                        mask: "*".into(),
                        what: What::Files,
                        color: "#e5a50a".into(),
                    },
                );
                self.sel = Some(at);
            }
            (ColorMsg::Delete, Some(i)) => {
                self.list.remove(i);
                self.sel = (!self.list.is_empty()).then(|| i.min(self.list.len() - 1));
            }
            (ColorMsg::Up, Some(i)) if i > 0 => {
                self.list.swap(i, i - 1);
                self.sel = Some(i - 1);
            }
            (ColorMsg::Down, Some(i)) if i + 1 < self.list.len() => {
                self.list.swap(i, i + 1);
                self.sel = Some(i + 1);
            }
            (ColorMsg::Reset, _) => *self = Self::new(colors::defaults()),
            _ => {}
        }
    }
}

pub fn what_label(w: What) -> String {
    match w {
        What::Any => fl!("colors-any"),
        What::Files => fl!("colors-files"),
        What::Dirs => fl!("colors-dirs"),
        What::Links => fl!("colors-links"),
        What::Exec => fl!("colors-exec"),
    }
}

/// A square of the rule's colour; the accent's for an empty one, a frame for a bad one.
fn swatch<'a>(color: &str) -> Element<'a, Message> {
    let paint = colors::parse(color);
    widget::container(widget::Space::new())
        .width(Length::Fixed(14.0))
        .height(Length::Fixed(14.0))
        .class(theme::Container::custom(move |t| {
            let fill = match paint {
                Some(Paint::Accent) => Some(Color::from(t.cosmic().accent_text_color())),
                Some(Paint::Rgb([r, g, b])) => Some(Color::from_rgb8(r, g, b)),
                None => None,
            };
            widget::container::Style {
                background: fill.map(Into::into),
                border: cosmic::iced::Border {
                    color: t.cosmic().on_bg_color().into(),
                    width: 1.0,
                    radius: 2.0.into(),
                },
                ..Default::default()
            }
        }))
        .into()
}

pub fn view<'a>(c: &'a ColorEdit, cancel: Element<'a, Message>) -> Element<'a, Message> {
    let msg = Message::Colors;
    let mut list = column![].spacing(1);
    for (i, r) in c.list.iter().enumerate() {
        let mask = if r.mask.trim().is_empty() {
            fl!("colors-every")
        } else {
            r.mask.clone()
        };
        list = list.push(crate::dialogs::menu_row_with(
            Some(swatch(&r.color)),
            None,
            mask,
            what_label(r.what),
            c.sel == Some(i),
            msg(ColorMsg::Select(i)),
        ));
    }
    if c.list.is_empty() {
        list = list.push(widget::text::caption(fl!("colors-empty")));
    }
    let list = widget::container(widget::scrollable(list).height(Length::Fixed(140.0)))
        .padding(4)
        .width(Length::Fill)
        .class(theme::Container::Card);

    let some = c.sel.is_some();
    let btn = |label: String, m: ColorMsg, on: bool| {
        widget::button::standard(label)
            .width(Length::Fixed(150.0))
            .on_press_maybe(on.then(|| msg(m)))
    };
    let arrow = |name: &'static str, m: ColorMsg, on: bool| {
        widget::button::icon(widget::icon::from_name(name).size(16))
            .on_press_maybe(on.then(|| msg(m)))
    };
    let last = c.sel.is_some_and(|i| i + 1 >= c.list.len());
    let buttons = column![
        btn(fl!("colors-add"), ColorMsg::Add, true),
        btn(fl!("hotlist-delete"), ColorMsg::Delete, some),
        btn(fl!("colors-reset"), ColorMsg::Reset, true),
        row![
            arrow("go-up-symbolic", ColorMsg::Up, c.sel.is_some_and(|i| i > 0)),
            arrow("go-down-symbolic", ColorMsg::Down, some && !last),
        ]
        .spacing(6),
    ]
    .spacing(6);

    let rule = c.sel.and_then(|i| c.list.get(i));
    let input = |value: &'a str, on: fn(String) -> ColorMsg| {
        let field = widget::text_input("", value);
        match rule {
            Some(_) => field
                .on_input(move |s| msg(on(s)))
                .on_submit(|_| Message::DialogSubmit),
            None => field,
        }
    };
    let whats: Vec<String> = What::ALL.into_iter().map(what_label).collect();
    let picked = rule.and_then(|r| What::ALL.iter().position(|w| *w == r.what));
    // Two rows, so the dialog's buttons stay inside a 530 px window.
    let labelled = |label: String, w: Element<'a, Message>| {
        column![widget::text::caption(label), w].spacing(4)
    };
    let fields = column![
        row![
            labelled(
                fl!("colors-mask"),
                input(rule.map_or("", |r| r.mask.as_str()), ColorMsg::Mask).into()
            )
            .width(Length::FillPortion(3)),
            labelled(
                fl!("colors-what"),
                widget::dropdown(whats, picked, move |w| msg(ColorMsg::What(w))).into()
            )
            .width(Length::FillPortion(2)),
        ]
        .spacing(12),
        labelled(
            fl!("colors-color"),
            row![
                input(rule.map_or("", |r| r.color.as_str()), ColorMsg::Color),
                swatch(rule.map_or("", |r| r.color.as_str())),
            ]
            .spacing(8)
            .align_y(Alignment::Center)
            .into()
        ),
    ]
    .spacing(8);

    widget::dialog()
        .title(fl!("colors-title"))
        .control(
            column![row![list, buttons].spacing(12), fields]
                .spacing(12)
                .width(Length::Fill),
        )
        .primary_action(widget::button::suggested(fl!("ok")).on_press(Message::DialogSubmit))
        .secondary_action(cancel)
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_edit_move_delete_reset() {
        let mut c = ColorEdit::new(colors::defaults());
        c.update(ColorMsg::Add); // after the links rule
        c.update(ColorMsg::Mask("*.rs".into()));
        c.update(ColorMsg::What(2)); // Dirs
        c.update(ColorMsg::Color("#ff0000".into()));
        assert_eq!(
            c.list[1],
            Rule {
                mask: "*.rs".into(),
                what: What::Dirs,
                color: "#ff0000".into()
            }
        );
        c.update(ColorMsg::Up);
        assert_eq!((c.sel, c.list[0].mask.as_str()), (Some(0), "*.rs"));
        c.update(ColorMsg::Up); // first: stays
        assert_eq!(c.sel, Some(0));
        c.update(ColorMsg::Delete);
        c.update(ColorMsg::Delete);
        assert!(c.list.is_empty() && c.sel.is_none());
        c.update(ColorMsg::Mask("x".into())); // nothing selected: nothing to edit
        c.update(ColorMsg::Reset);
        assert_eq!(c.list, colors::defaults());
    }
}
