//! TC layout: per pane a path line, column headers and a virtualized file list; one status line
//! (each pane's half), the command line and the F-keys in the footer.

use crate::app::{App, Message, StatusKind, Tab};
use crate::config::Skin;
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
use std::cell::OnceCell;
use std::path::PathBuf;
use std::sync::Arc;

const TEXT: u16 = 13;

pub fn view(app: &App) -> Element<'_, Message> {
    if let Some(l) = app.lister.as_deref().filter(|l| l.quick.is_none()) {
        return crate::lister::view(l);
    }
    // Ctrl+Q: the viewer in place of the other pane.
    let side = |s: usize| match app.lister.as_deref() {
        Some(l) if s != app.active => container(crate::lister::view(l))
            .width(Length::Fill)
            .height(Length::Fill)
            .into(),
        _ => pane(app, s),
    };
    let panes = row![side(0), side(1)].height(Length::Fill);
    match app.config.skin {
        Skin::Classic => column![crate::toolbar::bar(&app.config.toolbar), panes.spacing(4)].into(),
        Skin::Modern => panes.spacing(8).padding([4, 8, 0, 8]).into(),
    }
}

/// Classic skin: TC's button bar under the menu; each button sends the same `Action` as its key.
fn pane(app: &App, side: usize) -> Element<'_, Message> {
    let tabs = &app.panes[side];
    let p = tabs.active();
    let active = app.active == side;

    let title = match &p.results {
        Some(_) if p.branch => fl!("branch-title", dir = p.panel.cwd().display().to_string()),
        Some(_) => fl!("find-results", dir = p.panel.cwd().display().to_string()),
        None => p.panel.cwd().display().to_string(),
    };
    let skin = app.config.skin;
    let path: Element<'_, Message> = match (skin, &p.results) {
        (Skin::Modern, None) => breadcrumbs(side, p.panel.cwd(), active),
        // TC: the plain path line, each part clickable.
        (Skin::Classic, None) => container(path_parts(side, p.panel.cwd()))
            .padding([2, 6])
            .width(Length::Fill)
            .class(bar_style(active))
            .into(),
        _ => container(text(title).size(TEXT))
            .padding([2, 6])
            .width(Length::Fill)
            .class(bar_style(active))
            .into(),
    };

    let list = if p.brief {
        brief_list(side, p, active, skin, app.drop_hover, app.window_id())
    } else {
        full_list(app, side, p, active)
    };
    // Right-click menu: about the row under the cursor if the press landed on one, else the dir.
    let ctx = match (app.ctx_entry, p.panel.current()) {
        (true, Some(e)) if e.name != PARENT => crate::menu::Ctx::Entry {
            dir: e.is_dir(),
            archive: shagoff_core::archive::Format::detect(&e.name).is_some_and(|f| f.is_tree()),
        },
        _ => crate::menu::Ctx::Dir,
    };
    // Dropped on no dir row: into the pane's dir (not into search results: no one dir there).
    let list = if p.results.is_none() {
        let on = move |paths, mv| Message::Drop {
            side,
            dir: None,
            paths,
            mv,
        };
        crate::clip::drop_zone(list, on, None)
    } else {
        list
    };
    // The right-click menu and the size sensor sit in a layer under the list, not around it:
    // neither passes drop zones through (`drag_destinations`), so rows inside them never got
    // drops. A press reaches the rows first, then this layer, as before (`RightClick`, then
    // `RightEmpty`). A popup surface, not an overlay: libcosmic's overlay menu closes on the
    // press that should pick an item. The sensor: on_scroll misses window resizes.
    let under = cosmic::iced::widget::sensor(
        widget::Space::new()
            .width(Length::Fill)
            .height(Length::Fill),
    )
    .on_show(move |size| Message::Resized(side, size))
    .on_resize(move |size| Message::Resized(side, size));
    let under = widget::context_menu(
        mouse_area(under).on_right_press(Message::RightEmpty(side)),
        Some(crate::menu::context(ctx)),
    )
    .window_id(app.window_id())
    .on_surface_action(Message::Surface);
    let list = cosmic::iced::widget::stack![under, list];

    let drives = match app.config.skin {
        Skin::Classic => drive_list(app, side),
        Skin::Modern => drive_bar(app, side),
    };
    let mut col = column![drives];
    if tabs.items().len() > 1 {
        col = col.push(tab_bar(side, tabs, active, app.window_id()));
    }
    let col = col
        .push(path)
        .push(header(side, p.panel.sort(), skin, app.cols))
        .push(list)
        .width(Length::Fill);
    let pane: Element<'_, Message> = match skin {
        Skin::Classic => col.into(),
        Skin::Modern => container(col.spacing(2))
            .padding(6)
            .class(card_style(active))
            .into(),
    };
    // While a header edge is dragged, the whole pane follows the pointer (it may leave the header).
    match app.col_drag {
        Some((s, ..)) if s == side => mouse_area(pane)
            .on_move(|p| Message::ColDrag(p.x))
            .on_release(Message::ColDragEnd)
            .into(),
        _ => pane,
    }
}

/// Modern skin: the path as clickable parts, `/ › home › shag`; the last part is where we are.
fn breadcrumbs(side: usize, cwd: &std::path::Path, active: bool) -> Element<'static, Message> {
    let mut parts: Vec<(String, std::path::PathBuf)> = cwd
        .ancestors()
        .map(|a| (format::dir_title(a), a.to_path_buf()))
        .collect();
    parts.reverse();
    let last = parts.len().saturating_sub(1);
    let mut items: Vec<Element<'static, Message>> = Vec::new();
    for (i, (label, path)) in parts.into_iter().enumerate() {
        if i > 0 {
            items.push(text("›").size(TEXT).into());
        }
        let b = if i == last && active {
            button::suggested(label)
        } else if i == last {
            button::standard(label)
        } else {
            button::text(label)
        };
        items.push(
            b.padding([0, 6])
                .on_press(Message::PathPart(side, path))
                .into(),
        );
    }
    // A deep path scrolls sideways, kept at its end: the dir we are in stays visible.
    widget::scrollable(
        widget::row::with_children(items)
            .spacing(2)
            .align_y(Alignment::Center),
    )
    .direction(cosmic::iced::widget::scrollable::Direction::Horizontal(
        cosmic::iced::widget::scrollable::Scrollbar::new()
            .width(2)
            .scroller_width(2),
    ))
    .anchor_right()
    .width(Length::Fill)
    .into()
}

/// Classic skin: `/home/shag/x` as text, a click on a part goes there (TC does the same).
fn path_parts(side: usize, cwd: &std::path::Path) -> Element<'static, Message> {
    let mut parts: Vec<std::path::PathBuf> = cwd.ancestors().map(|a| a.to_path_buf()).collect();
    parts.reverse();
    let last = parts.len().saturating_sub(1);
    let mut items: Vec<Element<'static, Message>> = Vec::new();
    for (i, path) in parts.into_iter().enumerate() {
        // "/" for the root; the others get theirs after them, except the last one.
        // The last component as is (`..` too: `file_name` has none for it).
        let mut label = match path.components().next_back() {
            Some(std::path::Component::RootDir) | None => "/".to_string(),
            Some(c) => c.as_os_str().to_string_lossy().into_owned(),
        };
        if i > 0 && i < last {
            label.push('/');
        }
        items.push(
            mouse_area(text(label).size(TEXT).wrapping(Wrapping::None))
                .interaction(cosmic::iced::mouse::Interaction::Pointer)
                .on_press(Message::PathPart(side, path))
                .into(),
        );
    }
    // A deep path scrolls sideways, kept at its end: the dir we are in stays visible.
    widget::scrollable(widget::row::with_children(items).align_y(Alignment::Center))
        .direction(cosmic::iced::widget::scrollable::Direction::Horizontal(
            cosmic::iced::widget::scrollable::Scrollbar::new()
                .width(0)
                .scroller_width(0),
        ))
        .anchor_right()
        .width(Length::Fill)
        .into()
}

/// Modern skin: each pane is a rounded card; the active one has an accent border.
fn card_style(active: bool) -> theme::Container<'static> {
    theme::Container::custom(move |t| {
        let c = t.cosmic();
        let border = if active {
            c.accent_color()
        } else {
            c.primary_container_divider()
        };
        container::Style {
            background: Some(Color::from(c.primary_container_color()).into()),
            border: cosmic::iced::Border {
                color: border.into(),
                width: 1.0,
                radius: c.corner_radii.radius_m.into(),
            },
            ..Default::default()
        }
    })
}

/// Full view: virtualized rows in a vertical scrollable.
fn full_list<'a>(app: &'a App, side: usize, p: &'a Tab, active: bool) -> Element<'a, Message> {
    let (entries, cursor) = (p.panel.entries(), p.panel.cursor());
    let row_h = app.row_h();
    let range = viewport::visible_range(entries.len(), row_h, p.offset, p.height);
    let mut list = column![widget::Space::new().height(range.start as f32 * row_h)];
    let marked = OnceCell::new();
    for i in range.clone() {
        let row = file_row(
            app,
            side,
            i,
            &entries[i],
            i == cursor || app.drop_hover == Some((side, i)),
            active,
            p.panel.is_marked(&entries[i]),
        );
        // While a header edge is dragged, rows must not take the pointer's moves (a drag
        // source captures them, and the pane follows the edge through them).
        list = list.push(if app.col_drag.is_none() {
            dnd_row(app.window_id(), side, p, i, &marked, row)
        } else {
            row
        });
    }
    list = list.push(widget::Space::new().height((entries.len() - range.end) as f32 * row_h));
    scrollable(list)
        .id(app.scroll_ids[side].clone())
        .on_scroll(move |v| Message::Scrolled(side, v.absolute_offset().y, v.bounds().height))
        .height(Length::Fill)
        .into()
}

/// Brief view (TC): names only, top to bottom then left to right; scrolled by whole columns.
fn brief_list<'a>(
    side: usize,
    p: &'a Tab,
    active: bool,
    skin: Skin,
    hover: Option<(usize, usize)>,
    window: cosmic::iced::window::Id,
) -> Element<'a, Message> {
    let row_h = skin.row_h();
    let (entries, cursor) = (p.panel.entries(), p.panel.cursor());
    let (rows, cols) = p.brief_grid(row_h);
    let mut grid = row![].spacing(2);
    let marked = OnceCell::new();
    for c in p.col..p.col + cols {
        let mut list = column![];
        for (i, e) in entries.iter().enumerate().skip(c * rows).take(rows) {
            let label = if e.is_dir() {
                format::display_name(e)
            } else {
                e.name.clone()
            };
            let cell = container(name_cell(e, label, skin))
                .padding([0, 6])
                .height(Length::Fixed(row_h))
                .width(Length::Fill)
                .clip(true)
                .class(cursor_style(
                    i == cursor || hover == Some((side, i)),
                    active,
                    p.panel.is_marked(e),
                    skin,
                ));
            let cell = mouse_area(cell)
                .on_press(Message::Click(side, i))
                .on_right_press(Message::RightClick(side, i))
                .on_double_click(Message::DoubleClick(side, i));
            list = list.push(dnd_row(window, side, p, i, &marked, cell.into()));
        }
        grid = grid.push(container(list).width(Length::FillPortion(1)).clip(true));
    }
    mouse_area(container(grid).width(Length::Fill).height(Length::Fill))
        .on_scroll(move |d| Message::BriefWheel(side, d))
        .into()
}

/// Theme icon by type, then the name (cut with "…").
fn name_cell(e: &Entry, label: String, skin: Skin) -> Element<'static, Message> {
    let (name, generic) = format::icon_name(e);
    let icon = widget::icon::from_name(name)
        .fallback(Some(widget::icon::IconFallback::Names(vec![
            generic.into(),
        ])))
        .size(match skin {
            Skin::Classic => 16,
            Skin::Modern => 20,
        })
        .icon();
    row![icon, cell(label)]
        .spacing(4)
        .align_y(Alignment::Center)
        .into()
}

/// Classic skin, as in TC: the drive in a drop-down, `\\` (root) and `..` (up), free space.
fn drive_list(app: &App, side: usize) -> Element<'_, Message> {
    let current = drives::containing(&app.drives, app.panes[side].active().panel.cwd());
    // Mounted drives, then volumes not mounted yet (picking one mounts it).
    let mut labels: Vec<String> = app.drives.iter().map(|d| d.label.clone()).collect();
    labels.extend(app.volumes.iter().map(unmounted_label));
    let paths: Vec<std::path::PathBuf> = app.drives.iter().map(|d| d.path.clone()).collect();
    let devices: Vec<String> = app.volumes.iter().map(|v| v.device.clone()).collect();
    let small = |label: &'static str, action: Action| {
        button::text(label)
            .padding([0, 8])
            .on_press(Message::PaneKey(side, action))
    };
    let mut bar = row![
        widget::dropdown(labels, current, move |i| match paths.get(i) {
            Some(p) => Message::Drive(side, p.clone()),
            None => Message::MountVolume(side, devices[i - paths.len()].clone()),
        }),
        small("\\", Action::Root),
        small("..", Action::Parent),
        widget::Space::new().width(Length::Fill),
    ]
    .spacing(4)
    .align_y(Alignment::Center);
    if let Some((free, total)) = app.space[side] {
        bar = bar.push(text(fl!("disk-free", free = human(free), total = human(total))).size(TEXT));
    }
    container(bar).padding([2, 6]).into()
}

/// A volume to mount, as listed with the drives: "UBUNTU-SERV (not mounted)".
fn unmounted_label(v: &shagoff_core::mount::Volume) -> String {
    format!("{} ({})", v.name, fl!("not-mounted"))
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
    for v in &app.volumes {
        let b = button::text(unmounted_label(v));
        bar = bar.push(b.on_press(Message::MountVolume(side, v.device.clone())));
    }
    bar = bar.push(widget::Space::new().width(Length::Fill));
    if let Some((free, total)) = app.space[side] {
        bar = bar.push(text(fl!("disk-free", free = human(free), total = human(total))).size(TEXT));
    }
    container(bar).padding([2, 6]).into()
}

/// `12,3 ГБ`: units and decimal separator come from the locale's ftl.
pub fn human(n: u64) -> String {
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
fn tab_bar(
    side: usize,
    tabs: &Tabs<Tab>,
    pane_active: bool,
    window: cosmic::iced::window::Id,
) -> Element<'_, Message> {
    let mut bar = row![].spacing(2);
    for (i, t) in tabs.items().iter().enumerate() {
        let label = container(cell(t.title()))
            .padding([2, 8])
            .max_width(160.0)
            .class(cursor_style(
                i == tabs.active_index(),
                pane_active,
                false,
                Skin::Classic,
            ));
        let tab = mouse_area(label)
            .on_press(Message::TabPress(side, i))
            .on_enter(Message::TabEnter(side, i))
            // Right press selects the tab, so the menu's actions (active tab) are about it.
            .on_right_press(Message::SelectTab(side, i))
            .on_middle_press(Message::CloseTabAt(side, i))
            .on_double_click(Message::CloseTabAt(side, i));
        let ctx = crate::menu::Ctx::Tab { locked: t.locked };
        bar = bar.push(
            widget::context_menu(tab, Some(crate::menu::context(ctx)))
                .window_id(window)
                .on_surface_action(Message::Surface),
        );
    }
    bar.into()
}

/// Column headers on the same grid as `file_row` (padding 6, spacing 6), so labels sit over their columns.
/// Classic: on a bar with a rule in each gap, like TC's header buttons.
fn header(side: usize, sort: Sort, skin: Skin, w: [f32; 4]) -> Element<'static, Message> {
    let cell = |label: String, key: SortKey, width: Length, align: Alignment| {
        let arrow = match (sort.key == key, sort.asc) {
            (true, true) => " ▲",
            (true, false) => " ▼",
            _ => "",
        };
        mouse_area(cell(format!("{label}{arrow}")).width(width).align_x(align))
            .on_press(Message::PaneKey(side, Action::Sort(key)))
    };
    let start = Alignment::Start;
    let cells: Vec<Element<'static, Message>> = vec![
        cell(fl!("col-name"), SortKey::Name, Length::Fill, start).into(),
        cell(fl!("col-ext"), SortKey::Ext, Length::Fixed(w[0]), start).into(),
        cell(
            fl!("col-size"),
            SortKey::Size,
            Length::Fixed(w[1]),
            Alignment::End,
        )
        .into(),
        cell(fl!("col-date"), SortKey::Date, Length::Fixed(w[2]), start).into(),
        self::cell(fl!("col-attr"))
            .width(Length::Fixed(w[3]))
            .into(),
    ];
    // The 6 px gaps are the grabbable edges (TC: drag to resize the column on their right);
    // Classic draws a rule in them, Modern leaves them blank.
    let mut out = Vec::with_capacity(cells.len() * 2);
    for (i, c) in cells.into_iter().enumerate() {
        if i > 0 {
            let rule: Element<'static, Message> = match skin {
                Skin::Classic => widget::divider::vertical::default().into(),
                Skin::Modern => widget::Space::new().into(),
            };
            out.push(
                mouse_area(
                    container(rule)
                        .width(Length::Fixed(6.0))
                        .height(Length::Fixed(16.0))
                        .center_x(Length::Fixed(6.0)),
                )
                .interaction(cosmic::iced::mouse::Interaction::ResizingHorizontally)
                .on_press(Message::ColDragStart(side, i))
                .on_double_click(Message::PaneKey(side, Action::ResetColumns))
                .into(),
            );
        }
        out.push(c);
    }
    let row = widget::row::with_children(out).align_y(Alignment::Center);
    let bar = container(row).padding([2, 6]).width(Length::Fill);
    match skin {
        Skin::Modern => bar.into(),
        Skin::Classic => bar.class(header_style()).into(),
    }
}

/// Classic header bar: the theme's component (button) background.
fn header_style() -> theme::Container<'static> {
    theme::Container::custom(|t| container::Style {
        background: Some(Color::from(t.cosmic().primary_component_color()).into()),
        ..Default::default()
    })
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
    // A dir shows its size once Space counted it.
    let bytes = if e.is_dir() {
        app.panes[side].active().panel.dir_size(e)
    } else {
        Some(e.size)
    };
    let size = match (bytes, app.config.skin) {
        (None, _) => "<DIR>".to_string(),
        // The column fits "999 999 999"; above that "45,1 GB" (exact bytes in the status line).
        (Some(b), Skin::Classic) if b < 1_000_000_000 => format::size(b),
        // Modern: "12,3 KB" — the exact bytes are in the status line.
        (Some(b), _) => human(b),
    };
    let (date, attrs) = if is_parent {
        (String::new(), String::new())
    } else {
        (format::date(e.mtime, &app.tz), format::perms(e.mode))
    };
    let cells = row![
        // Empty style: the default (Transparent) sets its own text colour over the cursor's.
        container(name_cell(e, format::display_name(e), app.config.skin))
            .width(Length::Fill)
            .class(theme::Container::custom(|_| container::Style::default())),
        cell(e.ext.clone()).width(Length::Fixed(app.cols[0])),
        cell(size)
            .width(Length::Fixed(app.cols[1]))
            .align_x(Alignment::End),
        cell(date).width(Length::Fixed(app.cols[2])),
        cell(attrs).width(Length::Fixed(app.cols[3])),
    ]
    .spacing(6)
    .align_y(Alignment::Center);
    let row = container(cells)
        .padding([0, 6])
        .height(Length::Fixed(app.row_h()))
        .width(Length::Fill)
        .clip(true)
        .class(cursor_style(is_cursor, active, is_marked, app.config.skin));
    mouse_area(row)
        .on_press(Message::Click(side, i))
        .on_right_press(Message::RightClick(side, i))
        .on_double_click(Message::DoubleClick(side, i))
        .into()
}

/// A row can be dragged (the marked files if it is one of them, TC); a dir row also takes drops.
/// `marked`: the pane's marked files, gathered once per frame and only if a marked row is shown.
fn dnd_row<'a>(
    window: cosmic::iced::window::Id,
    side: usize,
    p: &'a Tab,
    i: usize,
    marked: &OnceCell<Arc<Vec<PathBuf>>>,
    row: Element<'a, Message>,
) -> Element<'a, Message> {
    let e = &p.panel.entries()[i];
    // Not `..`; not from inside an archive either: other programs would get paths that are not
    // on disk (Ctrl+C extracts for that, a drag cannot).
    let row = if e.name == PARENT || in_archive(p.panel.cwd()) {
        row
    } else {
        let paths = if p.panel.is_marked(e) {
            marked.get_or_init(|| Arc::new(p.panel.targets())).clone()
        } else {
            Arc::new(vec![p.panel.cwd().join(&e.os_name)])
        };
        crate::clip::drag(row, paths, window)
    };
    if !e.is_dir() || p.results.is_some() {
        return row;
    }
    // The dir itself, not the row number: a rescan before the drop may move the rows.
    let dir = p.panel.drop_dir(Some(i));
    let on = move |paths, mv| Message::Drop {
        side,
        dir: Some(dir.clone()),
        paths,
        mv,
    };
    crate::clip::drop_zone(row, on, Some((side, i)))
}

/// Inside an archive (`/x/a.zip/docs`), by the names alone: no disk access while drawing.
fn in_archive(cwd: &std::path::Path) -> bool {
    cwd.ancestors().any(|a| {
        (a.file_name().map(|n| n.to_string_lossy()))
            .and_then(|n| shagoff_core::archive::Format::detect(&n))
            .is_some_and(|f| f.is_tree())
    })
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
    if app.lister.as_ref().is_some_and(|l| l.quick.is_none()) {
        let status = container(window_status(app)).padding([2, 8]);
        return match job_line(app) {
            Some(job) => column![job, status].into(),
            None => status.into(),
        };
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
        // A gap at the right: the active half's details would run into the other half's totals.
        line.spacing(12)
            .padding([0, 24, 0, 0])
            .align_y(Alignment::Center)
            .width(Length::Fill)
            .into()
    };
    let mut col = column![].push_maybe(job_line(app)).push(
        row![half(0), half(1)]
            .spacing(4)
            .padding([2, 6])
            .height(Length::Shrink),
    );
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
/// A job in the background: bar, "Packing: 26 % — file", Show, Cancel (above the status line).
fn job_line(app: &App) -> Option<Element<'_, Message>> {
    let j = app.job.as_ref().filter(|j| j.hidden)?;
    let f = crate::dialogs::job_fraction(j);
    let mut label = format!(
        "{}: {} %",
        crate::dialogs::job_title(j.kind),
        (f * 100.0).round() as u32
    );
    if j.paused() {
        label = format!("{label} — {}", fl!("job-paused"));
    } else if !j.current.is_empty() {
        label = format!("{label} — {}", j.current);
    }
    if !app.queue.is_empty() {
        label = format!("{label} ({})", fl!("job-waiting", n = app.queue.len()));
    }
    let line = row![
        widget::progress_bar::determinate_linear(f)
            .width(Length::Fixed(160.0))
            .girth(8),
        text(label)
            .size(TEXT)
            .width(Length::Fill)
            .wrapping(Wrapping::None)
            .ellipsize(Ellipsize::End(EllipsizeHeightLimit::Lines(1))),
        button::text(if j.paused() {
            fl!("job-resume")
        } else {
            fl!("job-pause")
        })
        .on_press(Message::JobPause),
        button::text(fl!("job-show")).on_press(Message::JobShow),
        button::text(fl!("cancel")).on_press(Message::CancelJob),
    ]
    .spacing(8)
    .align_y(Alignment::Center);
    Some(container(line).padding([0, 8]).into())
}

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
pub(crate) fn cursor_style(
    is_cursor: bool,
    active: bool,
    marked: bool,
    skin: Skin,
) -> theme::Container<'static> {
    theme::Container::custom(move |t| {
        let c = t.cosmic();
        let mut style = container::Style::default();
        if skin == Skin::Modern {
            style.border.radius = c.corner_radii.radius_s.into();
        }
        match (is_cursor, active, marked) {
            // The button pairs: `accent.on` is not meant for text on the accent and was
            // unreadable (light on cyan in dark, dark on teal in light).
            (true, true, true) => {
                style.background = Some(Color::from(c.destructive_button.base).into());
                style.text_color = Some(c.destructive_button.on.into());
            }
            (true, true, false) => {
                style.background = Some(Color::from(c.accent_button.base).into());
                style.text_color = Some(c.accent_button.on.into());
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
