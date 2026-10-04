//! TC layout: per pane a path line, column headers and a virtualized file list; one status line
//! (each pane's half), the command line and the F-keys in the footer.

use crate::app::{App, Message, ROW_H, StatusKind, Tab};
use crate::fl;
use crate::keymap::Action;
use cosmic::iced::core::text::{Ellipsize, EllipsizeHeightLimit, Wrapping};
use cosmic::iced::widget::{column, row};
use cosmic::iced::{Alignment, Color, Length};
use cosmic::widget::{self, button, container, mouse_area, scrollable, text};
use cosmic::{Element, theme};
use shagoff_core::drives;
use shagoff_core::format;
use shagoff_core::listing::Entry;
use shagoff_core::panel::PARENT;
use shagoff_core::sort::{Sort, SortKey};
use shagoff_core::tabs::Tabs;
use shagoff_core::viewport;

const W_EXT: f32 = 60.0;
const W_SIZE: f32 = 90.0;
const W_DATE: f32 = 130.0;
const W_ATTR: f32 = 80.0;
const TEXT: u16 = 13;

pub fn view(app: &App) -> Element<'_, Message> {
    if let Some(l) = &app.lister {
        return crate::lister::view(l);
    }
    row![pane(app, 0), pane(app, 1)]
        .spacing(4)
        .height(Length::Fill)
        .into()
}

fn pane(app: &App, side: usize) -> Element<'_, Message> {
    let tabs = &app.panes[side];
    let p = tabs.active();
    let active = app.active == side;

    let title = match &p.results {
        Some(_) => fl!("find-results", dir = p.panel.cwd().display().to_string()),
        None => p.panel.cwd().display().to_string(),
    };
    let path = container(text(title).size(TEXT))
        .padding([2, 6])
        .width(Length::Fill)
        .class(bar_style(active));

    let list = if p.brief {
        brief_list(side, p, active)
    } else {
        full_list(app, side, p, active)
    };
    // on_scroll misses window resizes: the sensor reports the list's real size.
    let list = cosmic::iced::widget::sensor(list)
        .on_show(move |size| Message::Resized(side, size))
        .on_resize(move |size| Message::Resized(side, size));

    let mut col = column![drive_bar(app, side)];
    if tabs.items().len() > 1 {
        col = col.push(tab_bar(side, tabs, active));
    }
    col.push(path)
        .push(header(side, p.panel.sort()))
        .push(list)
        .width(Length::Fill)
        .into()
}

/// Full view: virtualized rows in a vertical scrollable.
fn full_list<'a>(app: &'a App, side: usize, p: &'a Tab, active: bool) -> Element<'a, Message> {
    let (entries, cursor) = (p.panel.entries(), p.panel.cursor());
    let range = viewport::visible_range(entries.len(), ROW_H, p.offset, p.height);
    let mut list = column![widget::Space::new().height(range.start as f32 * ROW_H)];
    for i in range.clone() {
        list = list.push(file_row(
            app,
            side,
            i,
            &entries[i],
            i == cursor,
            active,
            p.panel.is_marked(&entries[i]),
        ));
    }
    list = list.push(widget::Space::new().height((entries.len() - range.end) as f32 * ROW_H));
    scrollable(list)
        .id(app.scroll_ids[side].clone())
        .on_scroll(move |v| Message::Scrolled(side, v.absolute_offset().y, v.bounds().height))
        .height(Length::Fill)
        .into()
}

/// Brief view (TC): names only, top to bottom then left to right; scrolled by whole columns.
fn brief_list<'a>(side: usize, p: &'a Tab, active: bool) -> Element<'a, Message> {
    let (entries, cursor) = (p.panel.entries(), p.panel.cursor());
    let (rows, cols) = p.brief_grid();
    let mut grid = row![].spacing(2);
    for c in p.col..p.col + cols {
        let mut list = column![];
        for (i, e) in entries.iter().enumerate().skip(c * rows).take(rows) {
            let label = if e.is_dir() {
                format::display_name(e)
            } else {
                e.name.clone()
            };
            let cell = container(name_cell(e, label))
                .padding([0, 6])
                .height(Length::Fixed(ROW_H))
                .width(Length::Fill)
                .clip(true)
                .class(cursor_style(i == cursor, active, p.panel.is_marked(e)));
            list = list.push(
                mouse_area(cell)
                    .on_press(Message::Click(side, i))
                    .on_double_click(Message::DoubleClick(side, i)),
            );
        }
        grid = grid.push(container(list).width(Length::FillPortion(1)).clip(true));
    }
    mouse_area(container(grid).width(Length::Fill).height(Length::Fill))
        .on_scroll(move |d| Message::BriefWheel(side, d))
        .into()
}

/// Theme icon by type, then the name (cut with "…").
fn name_cell(e: &Entry, label: String) -> Element<'static, Message> {
    let (name, generic) = format::icon_name(e);
    let icon = widget::icon::from_name(name)
        .fallback(Some(widget::icon::IconFallback::Names(vec![
            generic.into(),
        ])))
        .size(16)
        .icon();
    row![icon, cell(label)]
        .spacing(4)
        .align_y(Alignment::Center)
        .into()
}

/// Drive buttons (the one holding cwd highlighted) and free space on the current disk.
// ponytail: many drives overflow the row; wrap in a horizontal scrollable when they stop fitting.
fn drive_bar(app: &App, side: usize) -> Element<'_, Message> {
    let current = drives::containing(&app.drives, app.panes[side].active().panel.cwd());
    let mut bar = row![].spacing(2).align_y(Alignment::Center);
    for (i, d) in app.drives.iter().enumerate() {
        let b = if Some(i) == current {
            button::suggested(d.label.clone())
        } else {
            button::text(d.label.clone())
        };
        bar = bar.push(b.on_press(Message::Drive(side, d.path.clone())));
    }
    bar = bar.push(widget::Space::new().width(Length::Fill));
    if let Some((free, total)) = app.space[side] {
        bar = bar.push(text(fl!("disk-free", free = human(free), total = human(total))).size(TEXT));
    }
    container(bar).padding([2, 6]).into()
}

/// `12,3 ГБ`: units and decimal separator come from the locale's ftl.
fn human(n: u64) -> String {
    let units = fl!("size-units");
    let units: [&str; 6] = units
        .split_whitespace()
        .collect::<Vec<_>>()
        .try_into()
        .unwrap_or(["B", "KB", "MB", "GB", "TB", "PB"]);
    let sep = fl!("decimal-sep").chars().next().unwrap_or('.');
    format::human(n, &units, sep)
}

/// Shown only with 2+ tabs; the active tab is styled like the cursor row.
fn tab_bar(side: usize, tabs: &Tabs<Tab>, pane_active: bool) -> Element<'_, Message> {
    let mut bar = row![].spacing(2);
    for (i, t) in tabs.items().iter().enumerate() {
        let label = container(cell(t.title()))
            .padding([2, 8])
            .max_width(160.0)
            .class(cursor_style(i == tabs.active_index(), pane_active, false));
        bar = bar.push(
            mouse_area(label)
                .on_press(Message::SelectTab(side, i))
                .on_middle_press(Message::CloseTabAt(side, i))
                .on_double_click(Message::CloseTabAt(side, i)),
        );
    }
    bar.into()
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
    is_marked: bool,
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
        container(name_cell(e, format::display_name(e))).width(Length::Fill),
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
        .class(cursor_style(is_cursor, active, is_marked));
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

/// A pane's part of the status line: quick search / filter field, read error, or totals.
fn pane_status(app: &App, side: usize) -> Element<'_, Message> {
    let p = app.panes[side].active();
    match (&app.search, &p.error) {
        (Some(s), _) if s.side == side => {
            let label = if s.filter {
                fl!("filter-label")
            } else {
                fl!("search-label")
            };
            row![
                text(label).size(TEXT),
                widget::text_input("", &s.text)
                    .id(app.input_id.clone())
                    .on_input(Message::SearchInput)
                    .on_submit(|_| Message::SearchSubmit)
                    .size(TEXT)
            ]
            .spacing(6)
            .align_y(Alignment::Center)
            .into()
        }
        (_, Some(e)) => text(e.clone())
            .size(TEXT)
            .wrapping(Wrapping::None)
            .ellipsize(Ellipsize::End(EllipsizeHeightLimit::Lines(1)))
            .class(theme::Text::Custom(|t| cosmic::iced::widget::text::Style {
                color: Some(t.cosmic().destructive_color().into()),
                ..Default::default()
            }))
            .into(),
        (_, None) => {
            let (t, m) = (p.panel.totals(), p.panel.marked_totals());
            let totals = fl!(
                "status",
                sel_bytes = format::size(m.bytes),
                bytes = format::size(t.bytes),
                sel_files = m.files.to_string(),
                files = t.files.to_string(),
                sel_dirs = m.dirs.to_string(),
                dirs = t.dirs.to_string()
            );
            let line = match p.panel.filter() {
                Some(f) => format!("{}  {totals}", fl!("filter-status", pattern = f)),
                None => totals,
            };
            text(line)
                .size(TEXT)
                .wrapping(Wrapping::None)
                .ellipsize(Ellipsize::End(EllipsizeHeightLimit::Lines(1)))
                .into()
        }
    }
}

/// One status line above the F-keys: each pane's totals under that pane; in the active pane's
/// half also the last message or, without one, the entry under the cursor.
pub fn footer(app: &App) -> Element<'_, Message> {
    // The viewer has no panels to total and its own buttons.
    if app.lister.is_some() {
        return container(window_status(app)).padding([2, 8]).into();
    }
    let half = |side: usize| -> Element<'_, Message> {
        let totals = container(pane_status(app, side))
            .width(Length::FillPortion(3))
            .clip(true);
        // Nothing to show (no message, cursor on ".."): the totals take the whole half.
        let extra = app.status.is_some()
            || app.panes[side]
                .active()
                .panel
                .current()
                .is_some_and(|e| e.name != shagoff_core::panel::PARENT);
        let line = if side == app.active && extra {
            row![
                totals,
                container(window_status(app))
                    .width(Length::FillPortion(2))
                    .clip(true)
            ]
        } else {
            row![totals]
        };
        line.spacing(12)
            .align_y(Alignment::Center)
            .width(Length::Fill)
            .into()
    };
    let mut col = column![
        row![half(0), half(1)]
            .spacing(4)
            .padding([2, 6])
            .height(Length::Shrink)
    ];
    if app.config.show_cmdline {
        col = col.push(command_line(app));
    }
    if app.config.show_fkeys {
        col = col.push(fkey_bar());
    }
    col.into()
}

/// TC `path>`: the active panel's dir and the command field.
fn command_line(app: &App) -> Element<'_, Message> {
    let cwd = app.panes[app.active]
        .active()
        .panel
        .cwd()
        .display()
        .to_string();
    // A deep path must leave the field room: it is cut in the middle.
    let prompt = container(
        text(format!("{cwd}>"))
            .size(TEXT)
            .wrapping(Wrapping::None)
            .ellipsize(Ellipsize::Middle(EllipsizeHeightLimit::Lines(1))),
    )
    .max_width(420.0);
    row![
        prompt,
        widget::text_input("", &app.cmdline)
            .id(app.cmd_id.clone())
            .on_input(Message::CmdInput)
            .on_submit(|_| Message::CmdSubmit)
            .size(TEXT)
    ]
    .spacing(6)
    .padding([0, 6, 2, 6])
    .align_y(Alignment::Center)
    .into()
}

/// The last message, else the entry under the active panel's cursor (right-aligned).
fn window_status(app: &App) -> Element<'_, Message> {
    if let Some(s) = &app.status {
        let class = match s.kind {
            StatusKind::Info => theme::Text::Default,
            StatusKind::Busy => theme::Text::Accent,
            StatusKind::Error => theme::Text::Custom(destructive),
        };
        return container(
            text(s.text.as_str())
                .size(TEXT)
                .class(class)
                .wrapping(Wrapping::None)
                .ellipsize(Ellipsize::End(EllipsizeHeightLimit::Lines(1))),
        )
        .width(Length::Fill)
        .align_x(Alignment::End)
        .into();
    }
    let p = &app.panes[app.active].active().panel;
    let details = p
        .current()
        .map(|e| format::details(e, &app.tz, &app.owners))
        .unwrap_or_default();
    container(
        text(details)
            .size(TEXT)
            .wrapping(Wrapping::None)
            .ellipsize(Ellipsize::Middle(EllipsizeHeightLimit::Lines(1))),
    )
    .width(Length::Fill)
    .align_x(Alignment::End)
    .into()
}

fn destructive(t: &cosmic::Theme) -> cosmic::iced::widget::text::Style {
    cosmic::iced::widget::text::Style {
        color: Some(t.cosmic().destructive_text_color().into()),
        ..Default::default()
    }
}

pub fn fkey_bar() -> Element<'static, Message> {
    // Eleven buttons must fit a ~1000 px window: tight padding, one line, centred.
    let key = |label: String, action| {
        button::custom(
            text(label)
                .wrapping(cosmic::iced::widget::text::Wrapping::None)
                .width(Length::Fill)
                .align_x(cosmic::iced::alignment::Horizontal::Center),
        )
        .class(theme::Button::Standard)
        .padding([6, 4])
        .on_press(Message::Key(action))
        .width(Length::Fill)
    };
    row![
        key(fl!("fkey-help"), Action::Help),
        key(fl!("fkey-rename"), Action::Rename),
        key(fl!("fkey-view"), Action::View),
        key(fl!("fkey-edit"), Action::Edit),
        key(fl!("fkey-copy"), Action::Copy),
        key(fl!("fkey-move"), Action::Move),
        key(fl!("fkey-mkdir"), Action::Mkdir),
        key(fl!("fkey-delete"), Action::Delete),
        key(fl!("fkey-name"), Action::CopyNames),
        key(fl!("fkey-path"), Action::CopyPaths),
        key(fl!("fkey-compare"), Action::CompareFiles),
    ]
    .spacing(3)
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

/// Cursor row: accent (dimmed in the inactive pane). Marked rows: red text; a marked row under the
/// active cursor becomes a red bar instead — red text on the accent bar is unreadable in light accents.
fn cursor_style(is_cursor: bool, active: bool, marked: bool) -> theme::Container<'static> {
    theme::Container::custom(move |t| {
        let c = t.cosmic();
        let mut style = container::Style::default();
        match (is_cursor, active, marked) {
            (true, true, true) => {
                style.background = Some(Color::from(c.destructive_color()).into());
                style.text_color = Some(c.on_destructive_color().into());
            }
            (true, true, false) => {
                style.background = Some(Color::from(c.accent_color()).into());
                style.text_color = Some(c.on_accent_color().into());
            }
            (true, false, _) => {
                let mut bg = Color::from(c.accent_color());
                bg.a = 0.35;
                style.background = Some(bg.into());
            }
            _ => {}
        }
        if marked && !(is_cursor && active) {
            style.text_color = Some(c.destructive_text_color().into());
        }
        style
    })
}
