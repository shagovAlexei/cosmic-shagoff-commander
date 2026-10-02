//! TC layout: per pane a path line, column headers, a virtualized file list and a status line.

use crate::app::{App, Message, ROW_H};
use crate::fl;
use cosmic::iced::core::text::{Ellipsize, EllipsizeHeightLimit, Wrapping};
use cosmic::iced::widget::{column, row};
use cosmic::iced::{Alignment, Color, Length};
use cosmic::widget::{self, button, container, mouse_area, scrollable, text};
use cosmic::{Element, theme};
use shagoff_core::format;
use shagoff_core::listing::Entry;
use shagoff_core::panel::PARENT;
use shagoff_core::sort::{Sort, SortKey};
use shagoff_core::viewport;

const W_EXT: f32 = 60.0;
const W_SIZE: f32 = 90.0;
const W_DATE: f32 = 130.0;
const W_ATTR: f32 = 80.0;
const TEXT: u16 = 13;

pub fn view(app: &App) -> Element<'_, Message> {
    row![pane(app, 0), pane(app, 1)]
        .spacing(4)
        .height(Length::Fill)
        .into()
}

fn pane(app: &App, side: usize) -> Element<'_, Message> {
    let p = &app.panes[side];
    let active = app.active == side;
    let entries = p.panel.entries();
    let cursor = p.panel.cursor();

    let path = container(text(p.panel.cwd().display().to_string()).size(TEXT))
        .padding([2, 6])
        .width(Length::Fill)
        .class(bar_style(active));

    let range = viewport::visible_range(entries.len(), ROW_H, p.offset, p.height);
    let mut list = column![widget::Space::new().height(range.start as f32 * ROW_H)];
    for i in range.clone() {
        list = list.push(file_row(app, side, i, &entries[i], i == cursor, active));
    }
    list = list.push(widget::Space::new().height((entries.len() - range.end) as f32 * ROW_H));
    let list = scrollable(list)
        .id(p.scroll_id.clone())
        .on_scroll(move |v| Message::Scrolled(side, v.absolute_offset().y, v.bounds().height))
        .height(Length::Fill);

    let status: Element<_> = match &p.error {
        Some(e) => text(e.clone())
            .size(TEXT)
            .class(theme::Text::Custom(|t| cosmic::iced::widget::text::Style {
                color: Some(t.cosmic().destructive_color().into()),
                ..Default::default()
            }))
            .into(),
        None => {
            let t = p.panel.totals();
            text(fl!(
                "status",
                sel_bytes = "0",
                bytes = format::size(t.bytes),
                sel_files = "0",
                files = t.files.to_string(),
                sel_dirs = "0",
                dirs = t.dirs.to_string()
            ))
            .size(TEXT)
            .into()
        }
    };

    column![
        path,
        header(side, p.panel.sort()),
        list,
        container(status).padding([2, 6])
    ]
    .width(Length::Fill)
    .into()
}

/// Column headers on the same grid as `file_row` (padding 6, spacing 6), so labels sit over their columns.
fn header(side: usize, sort: Sort) -> Element<'static, Message> {
    let cell = |label: String, key: SortKey, width: Length, align: Alignment| {
        let arrow = match (sort.key == key, sort.asc) {
            (true, true) => " ▲",
            (true, false) => " ▼",
            _ => "",
        };
        mouse_area(cell(format!("{label}{arrow}")).width(width).align_x(align))
            .on_press(Message::Header(side, key))
    };
    let start = Alignment::Start;
    container(
        row![
            cell(fl!("col-name"), SortKey::Name, Length::Fill, start),
            cell(fl!("col-ext"), SortKey::Ext, Length::Fixed(W_EXT), start),
            cell(
                fl!("col-size"),
                SortKey::Size,
                Length::Fixed(W_SIZE),
                Alignment::End
            ),
            cell(fl!("col-date"), SortKey::Date, Length::Fixed(W_DATE), start),
            self::cell(fl!("col-attr")).width(Length::Fixed(W_ATTR)),
        ]
        .spacing(6),
    )
    .padding([2, 6])
    .into()
}

fn file_row<'a>(
    app: &'a App,
    side: usize,
    i: usize,
    e: &'a Entry,
    is_cursor: bool,
    active: bool,
) -> Element<'a, Message> {
    let is_parent = e.name == PARENT;
    let size = if e.is_dir() {
        "<DIR>".to_string()
    } else {
        format::size(e.size)
    };
    let (date, attrs) = if is_parent {
        (String::new(), String::new())
    } else {
        (format::date(e.mtime, &app.tz), format::perms(e.mode))
    };
    let cells = row![
        cell(format::display_name(e)).width(Length::Fill),
        cell(e.ext.clone()).width(Length::Fixed(W_EXT)),
        cell(size)
            .width(Length::Fixed(W_SIZE))
            .align_x(Alignment::End),
        cell(date).width(Length::Fixed(W_DATE)),
        cell(attrs).width(Length::Fixed(W_ATTR)),
    ]
    .spacing(6)
    .align_y(Alignment::Center);
    let row = container(cells)
        .padding([0, 6])
        .height(Length::Fixed(ROW_H))
        .width(Length::Fill)
        .clip(true)
        .class(cursor_style(is_cursor, active));
    mouse_area(row)
        .on_press(Message::Click(side, i))
        .on_double_click(Message::DoubleClick(side, i))
        .into()
}

/// One-line table cell: long text is cut with "…" instead of wrapping.
fn cell(s: String) -> widget::Text<'static, cosmic::Theme> {
    text(s)
        .size(TEXT)
        .wrapping(Wrapping::None)
        .ellipsize(Ellipsize::End(EllipsizeHeightLimit::Lines(1)))
}

pub fn fkey_bar() -> Element<'static, Message> {
    let disabled = |label: String| button::standard(label).width(Length::Fill);
    row![
        disabled(fl!("fkey-view")),
        disabled(fl!("fkey-edit")),
        disabled(fl!("fkey-copy")),
        disabled(fl!("fkey-move")),
        disabled(fl!("fkey-mkdir")),
        disabled(fl!("fkey-delete")),
        button::standard(fl!("fkey-exit"))
            .on_press(Message::Exit)
            .width(Length::Fill),
    ]
    .spacing(4)
    .padding(4)
    .into()
}

fn bar_style(active: bool) -> theme::Container<'static> {
    theme::Container::custom(move |t| {
        let c = t.cosmic();
        if active {
            container::Style {
                background: Some(Color::from(c.accent_color()).into()),
                text_color: Some(c.on_accent_color().into()),
                ..Default::default()
            }
        } else {
            container::Style::default()
        }
    })
}

fn cursor_style(is_cursor: bool, active: bool) -> theme::Container<'static> {
    theme::Container::custom(move |t| {
        if !is_cursor {
            return container::Style::default();
        }
        let c = t.cosmic();
        let mut bg = Color::from(c.accent_color());
        if active {
            container::Style {
                background: Some(bg.into()),
                text_color: Some(c.on_accent_color().into()),
                ..Default::default()
            }
        } else {
            bg.a = 0.35;
            container::Style {
                background: Some(bg.into()),
                ..Default::default()
            }
        }
    })
}
