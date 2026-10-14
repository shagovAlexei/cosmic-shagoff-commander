//! Configuration → "Colors by type…" (TC "Color by file type"): the rules, first match wins.
//! Edits a copy, saved on OK.

use crate::app::Message;
use crate::fl;
use cosmic::iced::widget::{column, row};
use cosmic::iced::{Alignment, Color, Length};
use cosmic::{Element, theme, widget};
use shagoff_core::colors::{self, MarkColors, Paint, Rule, What};

#[derive(Clone, Debug)]
pub struct ColorEdit {
    pub list: Vec<Rule>,
    pub sel: Option<usize>,
    pub marks: MarkColors,
    /// The colour field edits marked rows' or the cursor's colours, not a rule's (`sel` is `None`).
    pub special: Option<Special>,
    /// The colour field shows names ("Orange") instead of codes (`#ff7800`).
    pub names: bool,
    /// The colour field and the palette edit the row's background, not the name's colour.
    pub bg: bool,
    /// The palette is open (it takes the rule list's room, so it is folded by default).
    pub palette: bool,
    /// The colour field's text: the selected rule's colour (or background), as a name or a code.
    pub field: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Special {
    /// Text and background.
    Marked,
    /// Background only: its text is black or white by it.
    Cursor,
}

#[derive(Clone, Debug)]
pub enum ColorMsg {
    Select(usize),
    Mask(String),
    Size(String),
    Age(String),
    Attr(String),
    /// Edit marked rows' / the cursor's colours.
    Special(Special),
    /// Index into `What::ALL`.
    What(usize),
    /// Typed into the colour field: a code, or a palette colour's name.
    Color(String),
    /// A palette swatch: its code.
    Pick(&'static str),
    /// The field shows names / codes.
    Names(bool),
    /// Open / fold the palette.
    Palette,
    /// The field and the palette edit the background / the name's colour.
    Bg(bool),
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
        let mut c = Self {
            list,
            sel,
            marks: colors::default_marks(),
            special: None,
            names: true,
            bg: false,
            palette: false,
            field: String::new(),
        };
        c.show();
        c
    }

    /// The colour being edited (see `color_mut`).
    pub fn edited(&self) -> &str {
        let (bg, m) = (self.bg, &self.marks);
        match self.special {
            Some(Special::Cursor) => &m.cursor,
            Some(Special::Marked) => {
                if bg {
                    &m.bg
                } else {
                    &m.color
                }
            }
            None => self
                .sel
                .and_then(|i| self.list.get(i))
                .map_or("", |r| if bg { &r.bg } else { &r.color }),
        }
    }

    /// An empty colour means none of ours: a background, the marks', the cursor's ("None"), not
    /// the accent of a name.
    pub fn empty_is_none(&self) -> bool {
        self.bg || self.special.is_some()
    }

    /// Back to the rules from the marks / the cursor: the cursor's forced background goes too.
    fn leave_special(&mut self) {
        if self.special.take() == Some(Special::Cursor) {
            self.bg = false;
        }
    }

    /// The colour being edited: the marks' or the cursor's, else the selected rule's; the
    /// background or the text's.
    fn color_mut(&mut self) -> Option<&mut String> {
        let bg = self.bg;
        let m = &mut self.marks;
        match self.special {
            Some(Special::Cursor) => Some(&mut m.cursor),
            Some(Special::Marked) => Some(if bg { &mut m.bg } else { &mut m.color }),
            None => {
                let r = self.sel.and_then(|i| self.list.get_mut(i))?;
                Some(if bg { &mut r.bg } else { &mut r.color })
            }
        }
    }

    /// The field from what is edited, in the chosen form (a colour without a name: its code).
    fn show(&mut self) {
        let c = self.edited().to_string();
        self.field = match self.names {
            true => color_name(&c, self.empty_is_none()).unwrap_or(c),
            false => c,
        };
    }

    pub fn update(&mut self, m: ColorMsg) {
        let sel = self.sel.filter(|&i| i < self.list.len());
        let typed = matches!(m, ColorMsg::Color(_));
        self.apply(m, sel);
        // Typing keeps what was typed; anything else shows the rule's colour afresh.
        if !typed {
            self.show();
        }
    }

    fn apply(&mut self, m: ColorMsg, sel: Option<usize>) {
        match (m, sel) {
            (ColorMsg::Select(i), _) => {
                self.sel = (i < self.list.len()).then_some(i);
                self.leave_special();
            }
            (ColorMsg::Special(s), _) => {
                self.leave_special();
                self.special = Some(s);
                self.sel = None;
                if s == Special::Cursor {
                    self.bg = true;
                }
            }
            (ColorMsg::Attr(s), Some(i)) => self.list[i].attr = s,
            (ColorMsg::Mask(s), Some(i)) => self.list[i].mask = s,
            (ColorMsg::Size(s), Some(i)) => self.list[i].size = s,
            (ColorMsg::Age(s), Some(i)) => self.list[i].age = s,
            (ColorMsg::What(w), Some(i)) => {
                if let Some(&what) = What::ALL.get(w) {
                    self.list[i].what = what;
                }
            }
            (ColorMsg::Color(s), _) => {
                let code = color_code(&s, self.empty_is_none()).unwrap_or_else(|| s.clone());
                if let Some(c) = self.color_mut() {
                    *c = code;
                }
                self.field = s;
            }
            (ColorMsg::Pick(code), _) => {
                if let Some(c) = self.color_mut() {
                    *c = code.into();
                }
            }
            (ColorMsg::Names(on), _) => self.names = on,
            // The cursor has a background only.
            (ColorMsg::Bg(on), _) => self.bg = on || self.special == Some(Special::Cursor),
            (ColorMsg::Palette, _) => self.palette = !self.palette,
            (ColorMsg::Add, _) => {
                let at = sel.map_or(self.list.len(), |i| i + 1);
                self.leave_special();
                self.list.insert(
                    at,
                    Rule {
                        mask: "*".into(),
                        what: What::Files,
                        color: "#e5a50a".into(),
                        ..Default::default()
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
            (ColorMsg::Reset, _) => {
                self.leave_special();
                let (names, bg, palette) = (self.names, self.bg, self.palette);
                *self = Self::new(colors::defaults());
                (self.names, self.bg, self.palette) = (names, bg, palette);
            }
            _ => {}
        }
    }
}

/// A palette colour's name ("Orange" for `#ff7800`; an empty one: the accent, or "None" for a
/// background); others: none.
pub fn color_name(code: &str, bg: bool) -> Option<String> {
    Some(match code.trim().to_ascii_lowercase().as_str() {
        "" if bg => fl!("colors-none"),
        "" => fl!("colors-accent"),
        "#99c1f1" => fl!("color-blue-light"),
        "#3584e4" => fl!("color-blue"),
        "#1a5fb4" => fl!("color-blue-dark"),
        "#99e3f0" => fl!("color-cyan-light"),
        "#33c7de" => fl!("color-cyan"),
        "#1c8fa8" => fl!("color-cyan-dark"),
        "#8ff0a4" => fl!("color-green-light"),
        "#33d17a" => fl!("color-green"),
        "#26a269" => fl!("color-green-dark"),
        "#f9f06b" => fl!("color-yellow-light"),
        "#f6d32d" => fl!("color-yellow"),
        "#e5a50a" => fl!("color-yellow-dark"),
        "#ffbe6f" => fl!("color-orange-light"),
        "#ff7800" => fl!("color-orange"),
        "#c64600" => fl!("color-orange-dark"),
        "#f66151" => fl!("color-red-light"),
        "#e01b24" => fl!("color-red"),
        "#a51d2d" => fl!("color-red-dark"),
        "#dc8add" => fl!("color-purple-light"),
        "#9141ac" => fl!("color-purple"),
        "#613583" => fl!("color-purple-dark"),
        "#cdab8f" => fl!("color-brown-light"),
        "#986a44" => fl!("color-brown"),
        "#63452c" => fl!("color-brown-dark"),
        "#ffffff" => fl!("color-white"),
        "#deddda" => fl!("color-grey-light"),
        "#9a9996" => fl!("color-grey"),
        "#3d3846" => fl!("color-grey-dark"),
        "#000000" => fl!("color-black"),
        _ => return None,
    })
}

/// A palette colour's code by its name, any case ("оранжевый" → `#ff7800`).
fn color_code(name: &str, bg: bool) -> Option<String> {
    let name = name.trim().to_lowercase();
    PALETTE
        .iter()
        .flatten()
        // Either name of the empty one ("Accent" / "None") clears the field, in both modes.
        .find(|c| {
            [color_name(c, bg), color_name(c, !bg)]
                .into_iter()
                .flatten()
                .any(|n| n.to_lowercase() == name)
        })
        .map(|c| c.to_string())
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

/// Clicked into the colour field, by rows: light, mid, dark shades of GNOME's palette (blue,
/// cyan, green, yellow, orange, red, purple, brown, grey); the first column is the theme's accent
/// (an empty field; no background when editing that), a light grey and black.
const PALETTE: [[&str; 10]; 3] = [
    [
        "", "#99c1f1", "#99e3f0", "#8ff0a4", "#f9f06b", "#ffbe6f", "#f66151", "#dc8add", "#cdab8f",
        "#ffffff",
    ],
    [
        "#deddda", "#3584e4", "#33c7de", "#33d17a", "#f6d32d", "#ff7800", "#e01b24", "#9141ac",
        "#986a44", "#9a9996",
    ],
    [
        "#000000", "#1a5fb4", "#1c8fa8", "#26a269", "#e5a50a", "#c64600", "#a51d2d", "#613583",
        "#63452c", "#3d3846",
    ],
];

/// A square of the rule's colour; the accent's for an empty one (a background: a frame), a frame
/// for a bad one.
fn swatch<'a>(color: &str, bg: bool) -> Element<'a, Message> {
    swatch_sized(color, bg, 14.0)
}

fn swatch_sized<'a>(color: &str, bg: bool, size: f32) -> Element<'a, Message> {
    let paint = colors::parse(color).filter(|p| !(bg && *p == Paint::Accent));
    widget::container(widget::Space::new())
        .width(Length::Fixed(size))
        .height(Length::Fixed(size))
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

/// One click per colour; the first is the accent / no background (an empty field).
fn palette<'a>(on: bool, bg: bool) -> Element<'a, Message> {
    let mut grid = column![].spacing(2);
    for line in PALETTE {
        let mut row = row![].spacing(4);
        for color in line {
            let name = color_name(color, bg).unwrap_or_default();
            let tip = if color.is_empty() {
                name
            } else {
                format!("{name} {color}")
            };
            let b = widget::button::custom(swatch_sized(color, bg, 18.0))
                .padding(3)
                .class(theme::Button::Icon)
                .on_press_maybe(on.then(|| Message::Colors(ColorMsg::Pick(color))));
            row = row.push(widget::tooltip(
                b,
                widget::text(tip),
                widget::tooltip::Position::Bottom,
            ));
        }
        grid = grid.push(row);
    }
    grid.into()
}

pub fn view<'a>(c: &'a ColorEdit, cancel: Element<'a, Message>) -> Element<'a, Message> {
    let msg = Message::Colors;
    let mut list = column![].spacing(1);
    for (i, r) in c.list.iter().enumerate() {
        let mut mask = if r.mask.trim().is_empty() {
            fl!("colors-every")
        } else {
            r.mask.clone()
        };
        if !r.valid() {
            mask = format!("⚠ {mask}");
        }
        // The kind, then the conditions: "Files  >1M  <7 d  h".
        let mut what = what_label(r.what);
        for (v, unit) in [
            (&r.size, String::new()),
            (&r.age, fl!("colors-days-short")),
            (&r.attr, String::new()),
        ] {
            if !v.trim().is_empty() {
                what = format!("{what}  {} {unit}", v.trim())
                    .trim_end()
                    .to_string();
            }
        }
        // The name's colour (not an empty one on a background: the row's text then), the background.
        let mut swatches = row![].spacing(2);
        let has_bg = !r.bg.trim().is_empty();
        if !(has_bg && r.color.trim().is_empty()) {
            swatches = swatches.push(swatch(&r.color, false));
        }
        if has_bg {
            swatches = swatches.push(swatch(&r.bg, true));
        }
        list = list.push(crate::dialogs::menu_row_with(
            Some(swatches.into()),
            None,
            mask,
            what,
            c.sel == Some(i),
            msg(ColorMsg::Select(i)),
        ));
    }
    if c.list.is_empty() {
        list = list.push(widget::text::caption(fl!("colors-empty")));
    }
    let list = widget::container(
        widget::scrollable(list).height(Length::Fixed(match c.palette {
            true => 96.0,
            false => 176.0,
        })),
    )
    .padding(4)
    .width(Length::Fill)
    .class(theme::Container::Card);

    let some = c.sel.is_some();
    let btn = |label: String, m: ColorMsg, on: bool| {
        widget::button::standard(label)
            .width(Length::Fixed(190.0))
            .on_press_maybe(on.then(|| msg(m)))
    };
    let arrow = |name: &'static str, m: ColorMsg, on: bool| {
        widget::button::icon(widget::icon::from_name(name).size(16))
            .on_press_maybe(on.then(|| msg(m)))
    };
    let last = c.sel.is_some_and(|i| i + 1 >= c.list.len());
    // Marked rows / the cursor: their swatches, lit while the colour field edits them.
    let special = |label: String, s: Special, colors: &[(&str, bool)]| -> Element<'a, Message> {
        // A colour that does not parse is the theme's: say so, as on a rule.
        let label = match colors.iter().all(|(c, _)| colors::parse(c).is_some()) {
            true => label,
            false => format!("⚠ {label}"),
        };
        let mut swatches = row![].spacing(2);
        for &(color, bg) in colors {
            swatches = swatches.push(swatch(color, bg));
        }
        let content = row![widget::text::body(label).width(Length::Fill), swatches]
            .spacing(6)
            .align_y(Alignment::Center);
        let b = match c.special == Some(s) {
            true => widget::button::custom(content).class(theme::Button::Suggested),
            false => widget::button::custom(content).class(theme::Button::Standard),
        };
        b.padding([4, 12])
            .width(Length::Fixed(190.0))
            .on_press(msg(ColorMsg::Special(s)))
            .into()
    };
    let m = &c.marks;
    let buttons = column![
        btn(fl!("colors-add"), ColorMsg::Add, true),
        row![
            btn(fl!("hotlist-delete"), ColorMsg::Delete, some).width(Length::Fixed(110.0)),
            arrow("go-up-symbolic", ColorMsg::Up, c.sel.is_some_and(|i| i > 0)),
            arrow("go-down-symbolic", ColorMsg::Down, some && !last),
        ]
        .spacing(6)
        .align_y(Alignment::Center),
        special(
            fl!("colors-marked"),
            Special::Marked,
            &[(&m.color, true), (&m.bg, true)]
        ),
        special(fl!("colors-cursor"), Special::Cursor, &[(&m.cursor, true)]),
    ]
    .spacing(6);

    let rule = c.sel.and_then(|i| c.list.get(i));
    let hinted = |hint: &'static str, value: &'a str, on: fn(String) -> ColorMsg| {
        let field = widget::text_input(hint, value);
        match rule {
            Some(_) => field
                .on_input(move |s| msg(on(s)))
                .on_submit(|_| Message::DialogSubmit),
            None => field,
        }
    };
    let input = |value: &'a str, on: fn(String) -> ColorMsg| hinted("", value, on);
    // The colour field edits a rule's colour, or the marks' / the cursor's.
    let editable = rule.is_some() || c.special.is_some();
    let none = c.empty_is_none();
    // Text / Background: which colour the field and the palette edit; the edited one is lit.
    let target = |label: String, bg: bool| -> Element<'a, Message> {
        let b = match c.bg == bg {
            true => widget::button::suggested(label),
            false => widget::button::standard(label),
        };
        // The cursor has a background only.
        b.on_press_maybe((bg || c.special != Some(Special::Cursor)).then(|| msg(ColorMsg::Bg(bg))))
            .into()
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
            labelled(
                fl!("colors-size"),
                hinted(">1M", rule.map_or("", |r| r.size.as_str()), ColorMsg::Size).into()
            )
            .width(Length::FillPortion(1)),
            labelled(
                fl!("colors-age"),
                hinted("<7", rule.map_or("", |r| r.age.as_str()), ColorMsg::Age).into()
            )
            .width(Length::FillPortion(1)),
            labelled(
                fl!("colors-attr"),
                hinted(
                    "h r s",
                    rule.map_or("", |r| r.attr.as_str()),
                    ColorMsg::Attr
                )
                .into()
            )
            .width(Length::FillPortion(1)),
        ]
        .spacing(12),
        labelled(
            fl!("colors-color"),
            column![
                row![
                    target(fl!("colors-text"), false),
                    target(fl!("colors-bg"), true),
                    {
                        let field = widget::text_input("", &c.field);
                        match editable {
                            true => field
                                .on_input(|s| Message::Colors(ColorMsg::Color(s)))
                                .on_submit(|_| Message::DialogSubmit),
                            false => field,
                        }
                    },
                    // The colour, and the palette under it: open / fold.
                    widget::button::custom(
                        row![
                            swatch(c.edited(), none),
                            widget::icon::from_name(match c.palette {
                                true => "pan-up-symbolic",
                                false => "pan-down-symbolic",
                            })
                            .size(16),
                        ]
                        .spacing(2)
                        .align_y(Alignment::Center)
                    )
                    .padding(4)
                    .class(theme::Button::Icon)
                    .on_press(msg(ColorMsg::Palette)),
                    widget::toggler(c.names)
                        .label(fl!("colors-names"))
                        .on_toggle(move |on| msg(ColorMsg::Names(on))),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            ]
            .push_maybe(c.palette.then(|| palette(editable, none)))
            .spacing(6)
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
        .tertiary_action(widget::button::text(fl!("colors-reset")).on_press(msg(ColorMsg::Reset)))
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
                color: "#ff0000".into(),
                ..Default::default()
            }
        );
        c.update(ColorMsg::Up);
        assert_eq!((c.sel, c.list[0].mask.as_str()), (Some(0), "*.rs"));
        c.update(ColorMsg::Up); // first: stays
        assert_eq!(c.sel, Some(0));
        for _ in 0..3 {
            c.update(ColorMsg::Delete);
        }
        assert!(c.list.is_empty() && c.sel.is_none());
        c.update(ColorMsg::Mask("x".into())); // nothing selected: nothing to edit
        c.update(ColorMsg::Reset);
        assert_eq!(c.list, colors::defaults());
    }

    #[test]
    fn the_colour_field_shows_and_takes_names_or_codes() {
        let mut c = ColorEdit::new(colors::defaults()); // links: cyan, shown by name
        assert_eq!(c.field, color_name("#33c7de", false).unwrap());
        c.update(ColorMsg::Names(false));
        assert_eq!(c.field, "#33c7de");
        c.update(ColorMsg::Names(true));
        let orange = color_name("#ff7800", false).unwrap().to_uppercase();
        c.update(ColorMsg::Color(orange.clone())); // a name, any case: stored as its code
        assert_eq!(
            (c.list[0].color.as_str(), c.field.as_str()),
            ("#ff7800", orange.as_str())
        );
        c.update(ColorMsg::Color("#123456".into())); // a code with no name stays a code
        assert_eq!(c.list[0].color, "#123456");
        c.update(ColorMsg::Select(0));
        assert_eq!(c.field, "#123456");
        c.update(ColorMsg::Pick("#e01b24"));
        assert_eq!(c.field, color_name("#e01b24", false).unwrap());
        c.update(ColorMsg::Names(false));
        assert_eq!(c.field, "#e01b24");
    }

    #[test]
    fn the_background_is_edited_apart() {
        let mut c = ColorEdit::new(colors::defaults());
        c.update(ColorMsg::Bg(true));
        assert_eq!(c.field, fl!("colors-none")); // no background yet
        c.update(ColorMsg::Pick("#99c1f1"));
        c.update(ColorMsg::Size(">1M".into()));
        c.update(ColorMsg::Age("<7".into()));
        let r = &c.list[0];
        assert_eq!((r.bg.as_str(), r.color.as_str()), ("#99c1f1", "#33c7de"));
        assert_eq!((r.size.as_str(), r.age.as_str()), (">1M", "<7"));
        c.update(ColorMsg::Color(fl!("colors-none"))); // the name of "none" clears it
        assert_eq!(c.list[0].bg, "");
        c.update(ColorMsg::Pick("#99c1f1"));
        c.update(ColorMsg::Color(fl!("colors-accent"))); // so does the accent's
        assert_eq!(c.list[0].bg, "");
        c.update(ColorMsg::Bg(false));
        assert_eq!(c.field, color_name("#33c7de", false).unwrap());
    }

    #[test]
    fn marks_and_cursor_are_edited_apart_from_rules() {
        let mut c = ColorEdit::new(colors::defaults());
        c.update(ColorMsg::Special(Special::Marked));
        assert_eq!(c.sel, None);
        c.update(ColorMsg::Pick("#f6d32d")); // text
        c.update(ColorMsg::Bg(true));
        c.update(ColorMsg::Color("#000000".into()));
        c.update(ColorMsg::Special(Special::Cursor));
        assert!(c.bg); // the cursor: a background only
        c.update(ColorMsg::Bg(false));
        assert!(c.bg);
        c.update(ColorMsg::Pick("#1a5fb4"));
        let want = MarkColors {
            color: "#f6d32d".into(),
            bg: "#000000".into(),
            cursor: "#1a5fb4".into(),
        };
        assert_eq!(c.marks, want);
        assert_eq!(c.list, colors::defaults()); // rules untouched
        c.update(ColorMsg::Select(1));
        assert_eq!(c.special, None);
        assert!(!c.bg); // the cursor's forced background stays with it
        c.update(ColorMsg::Attr("h".into()));
        assert_eq!(c.list[1].attr, "h");
        c.update(ColorMsg::Reset);
        assert_eq!(c.marks, colors::default_marks());
    }
}
