//! TC layout: per pane a path line, column headers and a virtualized file list; one status line
//! (each pane's half), the command line and the F-keys in the footer.

use crate::app::{App, Message, StatusKind, Tab};
use crate::config::{PANEL_SIZE, Skin};
use crate::fl;
use crate::keymap::Action;
use cosmic::iced::core::text::{Ellipsize, EllipsizeHeightLimit, Wrapping};
use cosmic::iced::widget::{column, row};
use cosmic::iced::{Alignment, Color, Font, Length};
use cosmic::widget::{self, button, container, mouse_area, scrollable, text};
use cosmic::{Element, theme};
use shagoff_core::colors::{self, Paint};
use shagoff_core::drives;
use shagoff_core::format;
use shagoff_core::listing::Entry;
use shagoff_core::panel::{PARENT, Show};
use shagoff_core::sort::{Sort, SortKey};
use shagoff_core::tabs::Tabs;
use shagoff_core::viewport;
use std::cell::OnceCell;
use std::path::PathBuf;
use std::sync::Arc;

const TEXT: u16 = 13;

/// F3 in its own window (TC Lister): a title bar of its own, then the viewer.
pub fn lister_window(app: &App, id: cosmic::iced::window::Id) -> Element<'_, Message> {
    let Some(l) = app.lister.as_deref().filter(|l| l.window == Some(id)) else {
        return widget::Space::new().into(); // closing: one more frame may be asked for
    };
    let focused = cosmic::Application::core(app).focused_window() == Some(id);
    let content = column![
        widget::header_bar()
            .title(l.name.clone())
            .on_close(Message::ListerClose)
            .on_drag(Message::ListerWindow(crate::app::ListerWin::Drag))
            .on_maximize(Message::ListerWindow(crate::app::ListerWin::Maximize))
            .on_double_click(Message::ListerWindow(crate::app::ListerWin::Maximize))
            .focused(focused),
        crate::lister::view(l),
        // Its own messages ("not found"…) where they are looked for.
        container(window_status(app)).padding([2, 8]),
    ];
    // A window of our own gets no background from libcosmic: without one it is see-through.
    container(content)
        .width(Length::Fill)
        .height(Length::Fill)
        .class(theme::Container::WindowBackground)
        .into()
}

pub fn view(app: &App) -> Element<'_, Message> {
    let content = main_view(app);
    match status_popup(app) {
        // Over the bottom of the panels: clicks elsewhere go through to them.
        Some(card) => cosmic::iced::widget::stack![
            content,
            container(card)
                .width(Length::Fill)
                .height(Length::Fill)
                .align_y(Alignment::End)
                .padding(8)
        ]
        .into(),
        None => content,
    }
}

/// Messages as cards of their own: the window's (error, note, "working…") and each pane's read
/// error — whole, wrapped, each with its ×; they last as they did in the status line, until the
/// next action (in that pane) or the result.
fn status_popup(app: &App) -> Option<Element<'_, Message>> {
    let card = |kind: StatusKind, msg: &str, who: Option<usize>| -> Element<'_, Message> {
        let (class, icon) = match kind {
            StatusKind::Info => (theme::Text::Default, "dialog-information-symbolic"),
            StatusKind::Busy => (theme::Text::Accent, "process-working-symbolic"),
            StatusKind::Error => (theme::Text::Custom(destructive), "dialog-error-symbolic"),
        };
        let line = row![
            widget::icon::from_name(icon).size(16),
            text(msg.to_string())
                .size(TEXT)
                .class(class)
                .wrapping(Wrapping::WordOrGlyph)
                .width(Length::Fill),
            button::icon(widget::icon::from_name("window-close-symbolic"))
                .on_press(Message::Dismiss(who)),
        ]
        .spacing(10)
        .align_y(Alignment::Center);
        container(line)
            .padding([4, 4, 4, 12])
            .width(Length::Fill)
            .class(theme::Container::Dialog(true)) // opaque even in a translucent theme
            .into()
    };
    let mut cards: Vec<Element<'_, Message>> = Vec::new();
    for side in 0..2 {
        if let Some(e) = &app.panes[side].active().error {
            cards.push(card(StatusKind::Error, e, Some(side)));
        }
    }
    if let Some(s) = &app.status {
        cards.push(card(s.kind, &s.text, None));
    }
    (!cards.is_empty()).then(|| widget::Column::with_children(cards).spacing(6).into())
}

fn main_view(app: &App) -> Element<'_, Message> {
    if let Some(l) = app.lister.as_deref().filter(|l| l.in_place()) {
        return crate::lister::view(l);
    }
    // Ctrl+O: the terminal in place of the panels (TC: the panels hidden, the console seen).
    if app.term_full {
        return crate::term::view(app, true);
    }
    // Ctrl+Q: the viewer in place of the other pane.
    let side = |s: usize| match app.lister.as_deref() {
        Some(l) if l.quick.is_some() && s != app.active => container(crate::lister::view(l))
            .width(Length::Fill)
            .height(Length::Fill)
            .into(),
        _ => pane(app, s),
    };
    // The panes share the width as `App::split` says; the gap between them is the divider,
    // dragged to change it (double click: half and half), as TC's.
    let (left, right) = viewport::split_portions(app.split);
    let (_, gap) = split_geometry(app.config.skin);
    // Settings «Line between the panels»: 1 px in the middle of the gap.
    let line: Element<'_, Message> = if app.config.divider_line {
        let rule = widget::Space::new()
            .width(Length::Fixed(1.0))
            .height(Length::Fill);
        container(rule)
            .class(theme::Container::custom(|t| container::Style {
                // The theme's divider colour is barely seen on a dark list: the text colour, faded.
                background: Some(
                    Color::from(t.cosmic().on_bg_color())
                        .scale_alpha(0.4)
                        .into(),
                ),
                ..Default::default()
            }))
            .into()
    } else {
        widget::Space::new().into()
    };
    let divider = widget::mouse_area(
        container(line)
            .center_x(Length::Fixed(gap))
            .height(Length::Fill),
    )
    .interaction(cosmic::iced::mouse::Interaction::ResizingHorizontally)
    .on_press(Message::SplitStart)
    .on_double_click(Message::SplitReset);
    // View → «Panel at 100 %»: the active pane alone.
    let panes = if app.wide {
        row![container(side(app.active)).width(Length::Fill)]
    } else {
        row![
            container(side(0)).width(Length::FillPortion(left)),
            divider,
            container(side(1)).width(Length::FillPortion(right)),
        ]
    }
    .height(Length::Fill);
    match app.config.skin {
        Skin::Classic => column![]
            .push_maybe(
                (app.config.show_toolbar)
                    .then(|| crate::toolbar::bar(&app.config.toolbar, app.tool_drag.grab)),
            )
            .push(panes)
            .into(),
        Skin::Modern => panes.padding([4.0, MODERN_PAD, 0.0, MODERN_PAD]).into(),
    }
}

/// For dragging the divider: the padding at each side of the panes row, and the gap between.
pub fn split_geometry(skin: Skin) -> (f32, f32) {
    match skin {
        Skin::Classic => (0.0, CLASSIC_GAP),
        Skin::Modern => (MODERN_PAD, MODERN_GAP),
    }
}

const CLASSIC_GAP: f32 = 4.0;
const MODERN_GAP: f32 = 8.0;
const MODERN_PAD: f32 = 8.0;
/// The Modern card's padding around its contents.
const CARD_PAD: f32 = 6.0;
/// The classic frame around a pane, inside it: the list is this much in from each side.
const FRAME_INSET: f32 = 1.0;

/// The window x between the panes (the middle of the gap), from the left pane's list width;
/// `framed`: the classic «Frame around the active panel» is on (its 1 px inset on each side).
pub fn pane_split(skin: Skin, left_list_w: f32, framed: bool) -> f32 {
    match skin {
        Skin::Classic if framed => left_list_w + 2.0 * FRAME_INSET + CLASSIC_GAP / 2.0,
        Skin::Classic => left_list_w + CLASSIC_GAP / 2.0,
        Skin::Modern => MODERN_PAD + CARD_PAD + left_list_w + CARD_PAD + MODERN_GAP / 2.0,
    }
}

/// Classic skin: TC's button bar under the menu; each button sends the same `Action` as its key.
fn pane(app: &App, side: usize) -> Element<'_, Message> {
    let tabs = &app.panes[side];
    let p = tabs.active();
    let active = app.active == side;

    let cwd = app.shown(p.panel.cwd());
    let title = match &p.results {
        Some(_) if p.branch => fl!("branch-title", dir = cwd.display().to_string()),
        Some(_) => fl!("find-results", dir = cwd.display().to_string()),
        None => cwd.display().to_string(),
    };
    let skin = app.config.skin;
    let editing = app.path_edit.as_ref().filter(|(s, _)| *s == side);
    let path: Element<'_, Message> = match (skin, &p.results) {
        // TC `cm_EditPath`: the line as a field; Enter goes there, Esc leaves it.
        _ if editing.is_some() => {
            let text = editing.map_or("", |(_, t)| t.as_str());
            widget::text_input("", text)
                .id(app.path_id.clone())
                .on_input(Message::PathEdit)
                .on_submit(|_| Message::PathEditGo)
                .on_unfocus(Message::PathEditCancel)
                .size(TEXT)
                .padding([2, 6])
                .width(Length::Fill)
                .into()
        }
        (Skin::Modern, None) => breadcrumbs(side, &cwd, active),
        // TC: the plain path line, each part clickable.
        (Skin::Classic, None) => container(path_parts(side, &cwd))
            .padding([2, 6])
            .width(Length::Fill)
            .class(bar_style(active, app.scheme.paint()))
            .into(),
        _ => container(text(title).size(TEXT))
            .padding([2, 6])
            .width(Length::Fill)
            .class(bar_style(active, app.scheme.paint()))
            .into(),
    };
    // A click on the line where no part of the path is (those take their own) edits it (TC).
    // Hidden (View → Window parts), it is still shown while edited.
    let path: Option<Element<'_, Message>> = match editing {
        Some(_) => Some(path),
        None => (app.config.show_path || p.results.is_some()).then(|| {
            mouse_area(path)
                .on_press(Message::PathEditStart(side))
                .into()
        }),
    };

    let list = if let Some(pt) = &p.tree {
        tree_list(app, side, p, &pt.tree, active)
    } else if p.thumbs {
        thumb_list(app, side, p, active)
    } else if p.brief {
        brief_list(app, side, p, active)
    } else {
        full_list(app, side, p, active)
    };
    // Right-click menu: about the row under the cursor if the press landed on one, else the dir.
    let ctx = match (app.ctx_entry, p.panel.current()) {
        (true, Some(e)) if e.name != PARENT => crate::menu::Ctx::entry(e, p.panel.cwd()),
        _ => crate::menu::Ctx::Dir,
    };
    // Dropped on no dir row: into the pane's dir (not into search results: no one dir there).
    let list = if p.results.is_none() {
        drop_into(side, None, None, list)
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
    // Right button marking: the menu (opened by libcosmic on the release) only for a press held
    // long enough; a quick click's release is kept from it.
    let held = app.right_hold.is_some_and(|h| h.armed && h.side == side);
    let list = crate::catch::catch_right_up(list, app.config.right_marks && !held);
    let list = cosmic::iced::widget::stack![under, list];
    // The list's own text and background (TC Colors), if set, under the header and the rows;
    // rows without a colour of their own take both.
    let paint = app.scheme.paint();
    let panel_style = move || {
        let rgb = |[r, g, b]: [u8; 3]| Color::from_rgb8(r, g, b);
        theme::Container::custom(move |_| container::Style {
            background: paint.back.map(|c| rgb(c).into()),
            text_color: paint.text.map(rgb),
            ..Default::default()
        })
    };
    let list = container(list)
        .width(Length::Fill)
        .height(Length::Fill)
        .class(panel_style());

    let drives = (app.config.show_drives).then(|| match app.config.skin {
        Skin::Classic => drive_list(app, side),
        Skin::Modern => drive_bar(app, side),
    });
    let mut col = column![].push_maybe(drives);
    if tabs.items().len() > 1 && app.config.show_tabs {
        col = col.push(tab_bar(
            side,
            tabs,
            active,
            app.window_id(),
            app.scheme.paint(),
        ));
    }
    let col = col
        .push_maybe(path)
        // The tree (Ctrl+F8) has no columns.
        .push_maybe((p.tree.is_none() && app.config.show_header).then(|| {
            container(header(
                side,
                p.panel.sort(),
                skin,
                app.col_widths(),
                viewport::visible_cols(p.width, app.col_widths()),
            ))
            .class(panel_style())
        }))
        .push(list)
        .width(Length::Fill);
    let pane: Element<'_, Message> = match skin {
        // Settings «Frame around the active panel»: the modern card's border, no card. The
        // inactive pane's border is there but clear, so nothing moves when the focus does.
        Skin::Classic if app.config.active_frame => (container(col)
            .padding(FRAME_INSET)
            .class(frame_style(active)))
        .into(),
        Skin::Classic => col.into(),
        Skin::Modern => container(col.spacing(2))
            .padding(CARD_PAD)
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

/// Classic skin with «Frame around the active panel»: the accent border on the active pane only.
fn frame_style(active: bool) -> theme::Container<'static> {
    theme::Container::custom(move |t| container::Style {
        border: cosmic::iced::Border {
            color: match active {
                true => t.cosmic().accent_color().into(),
                false => Color::TRANSPARENT,
            },
            width: 1.0,
            radius: 0.0.into(),
        },
        ..Default::default()
    })
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
    let inner = shagoff_core::archive::within(p.panel.cwd());
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
            dnd_row(app, side, p, i, &marked, inner, row)
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

/// Ctrl+F8: the dirs as an outline, rows as the file list draws them (only those in view).
fn tree_list<'a>(
    app: &'a App,
    side: usize,
    p: &'a Tab,
    tree: &'a shagoff_core::tree::Tree,
    active: bool,
) -> Element<'a, Message> {
    let row_h = app.row_h();
    let range = viewport::visible_range(tree.rows.len(), row_h, p.offset, p.height);
    let mut list = column![widget::Space::new().height(range.start as f32 * row_h)];
    let font = app.panel_font();
    for i in range.clone() {
        let n = &tree.rows[i];
        let mark = if n.open { "▾" } else { "▸" };
        let label = format!("{}{mark} {}", "    ".repeat(n.depth), n.name);
        let row = container(panel_cell(label, font))
            .padding([0, 6])
            .height(Length::Fixed(row_h))
            .width(Length::Fill)
            .clip(true)
            .align_y(Alignment::Center)
            .class(cursor_style(
                i == tree.cursor,
                active,
                false,
                app.config.skin,
                None,
                i % 2 == 1,
                app.scheme.paint(),
            ));
        list = list.push(
            mouse_area(row)
                .on_press(Message::TreeRow(side, i))
                .on_double_click(Message::TreeRowToggle(side, i)),
        );
    }
    list = list.push(widget::Space::new().height((tree.rows.len() - range.end) as f32 * row_h));
    scrollable(list)
        .id(app.scroll_ids[side].clone())
        .on_scroll(move |v| Message::Scrolled(side, v.absolute_offset().y, v.bounds().height))
        .height(Length::Fill)
        .into()
}

/// Brief view (TC): names only, top to bottom then left to right; scrolled by whole columns.
fn brief_list<'a>(app: &'a App, side: usize, p: &'a Tab, active: bool) -> Element<'a, Message> {
    let (skin, hover, scheme) = (app.config.skin, app.drop_hover, &app.scheme);
    let row_h = app.row_h();
    let font = app.panel_font();
    let (entries, cursor) = (p.panel.entries(), p.panel.cursor());
    let (rows, cols) = p.brief_grid(row_h, app.config.text_scale());
    let mut grid = row![].spacing(2);
    let marked = OnceCell::new();
    let inner = shagoff_core::archive::within(p.panel.cwd());
    for c in p.col..p.col + cols {
        let mut list = column![];
        for (i, e) in entries.iter().enumerate().skip(c * rows).take(rows) {
            let label = if e.is_dir() {
                format::display_name(e)
            } else {
                e.name.clone()
            };
            let lit = i == cursor || hover == Some((side, i)) || p.panel.is_marked(e);
            let look = scheme.pick(e, std::time::SystemTime::now());
            let paint = look.filter(|_| !lit).and_then(|l| l.fg);
            let cell = container(name_cell(e, label, skin, paint, font))
                .padding([0, 6])
                .height(Length::Fixed(row_h))
                .width(Length::Fill)
                .clip(true)
                .class(cursor_style(
                    i == cursor || hover == Some((side, i)),
                    active,
                    p.panel.is_marked(e),
                    skin,
                    look.and_then(|l| l.bg),
                    (i - c * rows) % 2 == 1,
                    scheme.paint(),
                ));
            let cell = row_area(app, side, i, cell.into());
            list = list.push(dnd_row(app, side, p, i, &marked, inner, cell));
        }
        grid = grid.push(container(list).width(Length::FillPortion(1)).clip(true));
    }
    mouse_area(container(grid).width(Length::Fill).height(Length::Fill))
        .on_scroll(move |d| Message::BriefWheel(side, d))
        .into()
}

/// Thumbnails view (TC Ctrl+Shift+F1): cells left to right, then down; scrolled by pixels like
/// the Full view, only the rows on screen built.
fn thumb_list<'a>(app: &'a App, side: usize, p: &'a Tab, active: bool) -> Element<'a, Message> {
    let (skin, hover, scheme) = (app.config.skin, app.drop_hover, &app.scheme);
    let font = app.panel_font();
    let cell_h = crate::thumbs::cell_h(app.row_h());
    let (entries, cursor) = (p.panel.entries(), p.panel.cursor());
    let cols = p.thumb_cols();
    let range = viewport::thumb_range(entries.len(), cols, cell_h, p.offset, p.height);
    let (first, last) = (range.start / cols, entries.len().div_ceil(cols));
    let mut list = column![widget::Space::new().height(first as f32 * cell_h)];
    let marked = OnceCell::new();
    let inner = shagoff_core::archive::within(p.panel.cwd());
    let size = crate::thumbs::SIZE as f32;
    for start in range.clone().step_by(cols) {
        let mut line = row![];
        for (i, e) in entries
            .iter()
            .enumerate()
            .take(range.end)
            .skip(start)
            .take(cols)
        {
            let label = if e.is_dir() {
                format::display_name(e)
            } else {
                e.name.clone()
            };
            let on = i == cursor || hover == Some((side, i));
            let look = scheme.pick(e, std::time::SystemTime::now());
            let paint = look
                .filter(|_| !on && !p.panel.is_marked(e))
                .and_then(|l| l.fg);
            let pic = (app.thumbs.get(&p.panel.cwd().join(&e.os_name)))
                .and_then(|(m, h)| h.as_ref().filter(|_| *m == e.mtime));
            let pic: Element<'_, Message> = match pic {
                Some(h) => widget::image(h.clone()).into(),
                None => entry_icon(e, 64).into(),
            };
            let name = painted(label, paint, font);
            // Inner containers without a style of their own: cosmic's default sets its text colour
            // and the cursor's dark ink was lost (light name on the cursor's cyan), as in the list.
            let bare = || theme::Container::custom(|_| container::Style::default());
            let cell = column![
                container(pic).center(Length::Fixed(size)).class(bare()),
                container(name)
                    .center_x(Length::Fill)
                    .clip(true)
                    .class(bare()),
            ]
            .align_x(Alignment::Center);
            let cell = container(cell)
                .padding(4)
                .width(Length::Fixed(crate::thumbs::CELL_W))
                .height(Length::Fixed(cell_h))
                .clip(true)
                .class(cursor_style(
                    on,
                    active,
                    p.panel.is_marked(e),
                    skin,
                    look.and_then(|l| l.bg),
                    false,
                    scheme.paint(),
                ));
            let cell = row_area(app, side, i, cell.into());
            line = line.push(dnd_row(app, side, p, i, &marked, inner, cell));
        }
        list = list.push(line);
    }
    let rows_after = last.saturating_sub(range.end.div_ceil(cols));
    list = list.push(widget::Space::new().height(rows_after as f32 * cell_h));
    scrollable(list)
        .id(app.scroll_ids[side].clone())
        .on_scroll(move |v| Message::Scrolled(side, v.absolute_offset().y, v.bounds().height))
        .height(Length::Fill)
        .into()
}

/// The theme's icon for `e` (its type's, else the generic one).
pub(crate) fn entry_icon(e: &Entry, size: u16) -> widget::Icon {
    let (name, generic) = format::icon_name(e);
    widget::icon::from_name(name)
        .fallback(Some(widget::icon::IconFallback::Names(vec![
            generic.into(),
        ])))
        .size(size)
        .icon()
}

/// Theme icon by type, then the name (cut with "…").
/// `paint`: the name's colour by type (`None` on the cursor / a marked row: its own colour wins).
fn name_cell(
    e: &Entry,
    label: String,
    skin: Skin,
    paint: Option<Paint>,
    (font, size): (Font, u16),
) -> Element<'static, Message> {
    let icon = entry_icon(
        e,
        match skin {
            Skin::Classic => 16,
            Skin::Modern => 20,
        } * size
            / PANEL_SIZE,
    );
    row![icon, painted(label, paint, (font, size))]
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
        // One line, cut on a narrow pane: wrapped, it would push this pane's list down.
        bar = bar.push(
            text(fl!("disk-free", free = human(free), total = human(total)))
                .size(TEXT)
                .wrapping(Wrapping::None)
                .ellipsize(cosmic::iced::core::text::Ellipsize::End(
                    cosmic::iced::core::text::EllipsizeHeightLimit::Lines(1),
                )),
        );
    }
    container(bar).padding([2, 6]).into()
}

/// A volume to mount, as listed with the drives: "UBUNTU-SERV (not mounted)".
fn unmounted_label(v: &shagoff_core::mount::Volume) -> String {
    format!("{} ({})", v.name, fl!("not-mounted"))
}

/// Drive buttons (the one holding cwd highlighted) and free space on the current disk. One line
/// however narrow the pane (the divider): the buttons scroll sideways, the free space is cut.
fn drive_bar(app: &App, side: usize) -> Element<'_, Message> {
    let current = drives::containing(&app.drives, app.panes[side].active().panel.cwd());
    let mut bar = row![].spacing(2).align_y(Alignment::Center);
    for (i, d) in app.drives.iter().enumerate() {
        let b = if Some(i) == current {
            button::suggested(d.label.clone())
        } else {
            button::text(d.label.clone())
        };
        let b = b.on_press(Message::Drive(side, d.path.clone()));
        bar = bar.push(drop_into(side, Some(d.path.clone()), None, b));
    }
    for v in &app.volumes {
        let b = button::text(unmounted_label(v));
        bar = bar.push(b.on_press(Message::MountVolume(side, v.device.clone())));
    }
    let buttons = widget::scrollable(bar)
        .direction(cosmic::iced::widget::scrollable::Direction::Horizontal(
            cosmic::iced::widget::scrollable::Scrollbar::new()
                .width(2)
                .scroller_width(2),
        ))
        .width(Length::Fill);
    let mut bar = row![buttons].spacing(8).align_y(Alignment::Center);
    if let Some((free, total)) = app.space[side] {
        bar = bar.push(
            text(fl!("disk-free", free = human(free), total = human(total)))
                .size(TEXT)
                .wrapping(Wrapping::None)
                .ellipsize(Ellipsize::End(EllipsizeHeightLimit::Lines(1))),
        );
    }
    container(bar).padding([2, 6]).into()
}

/// `12,3 ГБ`: units and decimal separator come from the locale's ftl.
/// A size as the totals under the panels show it (Show → sizes in KB / MB, or bytes).
pub fn size_text(config: &crate::config::Config, n: u64) -> String {
    match config.human_sizes() {
        true => human(n),
        false => format!("{} {}", format::size(n), fl!("byte-unit")),
    }
}

/// The Size column of a row: a dir its size once Space counted it, else `<DIR>`; as the
/// settings show sizes (also copied by «File details»).
pub fn size_cell(app: &App, panel: &shagoff_core::panel::Panel, e: &Entry) -> String {
    let bytes = if e.is_dir() {
        panel.dir_size(e)
    } else {
        Some(e.size)
    };
    let modern = app.config.skin == Skin::Modern;
    match bytes {
        None => "<DIR>".to_string(),
        Some(_) if e.unknown() => String::new(),
        Some(b) if format::human_size(b, app.config.human_sizes, modern) => human(b),
        Some(b) => format::size(b),
    }
}

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

/// Files dropped on `child` open the F5 / F6 dialog aimed at `dir` (`None`: the pane's dir);
/// `hover`: the row that lights up meanwhile.
fn drop_into<'a>(
    side: usize,
    dir: Option<std::path::PathBuf>,
    hover: Option<(usize, usize)>,
    child: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    let on = move |paths, mv| Message::Drop {
        side,
        dir: dir.clone(),
        paths,
        mv,
    };
    crate::clip::drop_zone(child, on, hover)
}

/// Shown only with 2+ tabs; the active tab is styled like the cursor row.
fn tab_bar(
    side: usize,
    tabs: &Tabs<Tab>,
    pane_active: bool,
    window: cosmic::iced::window::Id,
    paint: colors::PanelPaint,
) -> Element<'_, Message> {
    let mut bar = row![].spacing(2);
    for (i, t) in tabs.items().iter().enumerate() {
        let label = container(cell(t.title()))
            .padding([2, 8])
            .max_width(160.0)
            // In a palette's colours as the rows: the shown tab as the cursor, the rest the list's.
            .class(cursor_style(
                i == tabs.active_index(),
                pane_active,
                false,
                Skin::Classic,
                paint.back,
                false,
                paint,
            ));
        let tab = mouse_area(label)
            .on_press(Message::TabPress(side, i))
            .on_enter(Message::TabEnter(side, i))
            // Right press selects the tab, so the menu's actions (active tab) are about it.
            .on_right_press(Message::SelectTab(side, i))
            .on_middle_press(Message::CloseTabAt(side, i))
            .on_double_click(Message::CloseTabAt(side, i));
        let ctx = crate::menu::Ctx::Tab { lock: t.lock() };
        let tab = widget::context_menu(tab, Some(crate::menu::context(ctx)))
            .window_id(window)
            .on_surface_action(Message::Surface);
        // Files dropped on a tab go into its dir (TC); outside the menu, which hides drop zones.
        bar = bar.push(if t.results.is_some() {
            tab.into()
        } else {
            // Where it is going: a scan still in flight has not moved `cwd` yet.
            drop_into(side, Some(t.target()), None, tab)
        });
    }
    bar.into()
}

/// Column headers on the same grid as `file_row` (padding 6, spacing 6), so labels sit over their columns.
/// Classic: on a bar with a rule in each gap, like TC's header buttons.
fn header(
    side: usize,
    sort: Sort,
    skin: Skin,
    w: [f32; 4],
    shown: usize,
) -> Element<'static, Message> {
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
    // A narrow pane: the columns on the right that don't fit are left out, as in the rows.
    let cells: Vec<_> = cells.into_iter().take(1 + shown).collect();
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
    let size = size_cell(app, &app.panes[side].active().panel, e);
    let (date, attrs) = if is_parent || e.unknown() {
        (String::new(), String::new())
    } else {
        (format::date(e.mtime, &app.tz), format::attrs(e))
    };
    let look = app.scheme.pick(e, std::time::SystemTime::now());
    let font = app.panel_font();
    let cell = |s| panel_cell(s, font);
    // A narrow pane keeps Name readable: fixed columns that don't fit go, rightmost first.
    let cols = app.col_widths();
    let shown = viewport::visible_cols(app.panes[side].active().width, cols);
    let cells = row![
        // Empty style: the default (Transparent) sets its own text colour over the cursor's.
        container(name_cell(
            e,
            format::display_name(e),
            app.config.skin,
            look.filter(|_| !(is_cursor || is_marked))
                .and_then(|l| l.fg),
            font,
        ))
        .width(Length::Fill)
        .class(theme::Container::custom(|_| container::Style::default())),
    ]
    .push_maybe((shown > 0).then(|| cell(e.ext.clone()).width(Length::Fixed(cols[0]))))
    .push_maybe((shown > 1).then(|| {
        cell(size)
            .width(Length::Fixed(cols[1]))
            .align_x(Alignment::End)
    }))
    .push_maybe((shown > 2).then(|| cell(date).width(Length::Fixed(cols[2]))))
    .push_maybe((shown > 3).then(|| cell(attrs).width(Length::Fixed(cols[3]))))
    .spacing(6)
    .align_y(Alignment::Center);
    let row = container(cells)
        .padding([0, 6])
        .height(Length::Fixed(app.row_h()))
        .width(Length::Fill)
        .clip(true)
        .class(cursor_style(
            is_cursor,
            active,
            is_marked,
            app.config.skin,
            look.and_then(|l| l.bg),
            i % 2 == 1,
            app.scheme.paint(),
        ));
    row_area(app, side, i, row.into())
}

/// A row's clicks; while the right button is held in this pane, its entering too (drag marking).
fn row_area<'a>(
    app: &App,
    side: usize,
    i: usize,
    row: Element<'a, Message>,
) -> Element<'a, Message> {
    let area = mouse_area(row)
        .on_press(Message::Click(side, i))
        .on_right_press(Message::RightClick(side, i))
        .on_double_click(Message::DoubleClick(side, i));
    match app.right_hold.is_some_and(|h| h.side == side) {
        true => area.on_enter(Message::RowEnter(side, i)).into(),
        false => area.into(),
    }
}

/// A row can be dragged (the marked files if it is one of them, TC); a dir row also takes drops.
/// `marked`: the pane's marked files, gathered once per frame and only if a marked row is shown.
/// `inner`: the pane is inside an archive (its rows go only to our panes).
fn dnd_row<'a>(
    app: &App,
    side: usize,
    p: &'a Tab,
    i: usize,
    marked: &OnceCell<Arc<Vec<PathBuf>>>,
    inner: bool,
    row: Element<'a, Message>,
) -> Element<'a, Message> {
    let e = &p.panel.entries()[i];
    // Not `..`. From inside an archive only to our own panes: other programs would get paths that
    // are not on disk.
    let row = if e.name == PARENT {
        row
    } else {
        let paths = if p.panel.is_marked(e) {
            marked.get_or_init(|| Arc::new(p.panel.targets())).clone()
        } else {
            Arc::new(vec![p.panel.cwd().join(&e.os_name)])
        };
        // Built when a drag starts, not for every row and frame.
        let n = paths.len();
        let one = (n == 1 && paths[0].file_name() == Some(e.os_name.as_os_str())).then_some(e);
        let icon = move || match one {
            Some(e) => crate::clip::drag_card(entry_icon(e, 24), e.name.clone()),
            // Marked ones (or one that is not this row: Ctrl+B with a dir and its file marked).
            None => crate::clip::drag_card(
                widget::icon::from_name("edit-copy").size(24).icon(),
                fl!("drag-items", n = n),
            ),
        };
        // The right button marks (TC): it does not drag then.
        let right = !app.config.right_marks;
        crate::clip::drag(row, paths, inner, app.window_id(), right, icon)
    };
    // A dir or an archive file (packed into it, as TC). The path itself, not the row number: a
    // rescan before the drop may move the rows.
    match p.panel.drop_target(i).filter(|_| p.results.is_none()) {
        Some(dir) => drop_into(side, Some(dir), Some((side, i)), row),
        None => row,
    }
}

/// One-line table cell: long text is cut with "…" instead of wrapping.
fn cell(s: String) -> widget::Text<'static, cosmic::Theme> {
    panel_cell(s, (cosmic::font::default(), TEXT))
}

/// A cell of the file list, in its font (Configuration → Settings → Fonts).
/// A list name in its colour by type (`None`: the row's own).
fn painted(
    label: String,
    paint: Option<Paint>,
    font: (Font, u16),
) -> widget::Text<'static, cosmic::Theme> {
    let cell = panel_cell(label, font);
    match paint {
        Some(Paint::Accent) => cell.class(theme::Text::Accent),
        Some(Paint::Rgb([r, g, b])) => cell.class(theme::Text::Color(Color::from_rgb8(r, g, b))),
        None => cell,
    }
}

fn panel_cell(s: String, (font, size): (Font, u16)) -> widget::Text<'static, cosmic::Theme> {
    text(s)
        .size(size)
        .font(font)
        .wrapping(Wrapping::None)
        .ellipsize(Ellipsize::End(EllipsizeHeightLimit::Lines(1)))
}

/// A pane's part of the status line: quick search / filter field, or totals (a read error is a card).
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
        // A read error is a card of its own (`status_popup`): the totals stay.
        _ => {
            let (t, m) = (p.panel.totals(), p.panel.marked_totals());
            let size = |n| size_text(&app.config, n);
            let totals = fl!(
                "status",
                sel_bytes = size(m.bytes),
                bytes = size(t.bytes),
                sel_files = m.files.to_string(),
                files = t.files.to_string(),
                sel_dirs = m.dirs.to_string(),
                dirs = t.dirs.to_string()
            );
            let show = match p.panel.show() {
                _ if p.panel.only_marked() => Some(fl!("show-status", what = fl!("show-marked"))),
                Show::All => None,
                Show::Programs => Some(fl!("show-status", what = fl!("show-programs"))),
                Show::Mask(m) => Some(fl!("show-status", what = m.as_str())),
            };
            let filter = (p.panel.filter()).map(|f| fl!("filter-status", pattern = f));
            let line = (show.into_iter().chain(filter))
                .chain([totals])
                .collect::<Vec<_>>()
                .join("  ");
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
    if app.lister.as_ref().is_some_and(|l| l.in_place()) {
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
        // Nothing to show (cursor on ".."): the totals take the whole half.
        let extra = app.panes[side]
            .active()
            .panel
            .current()
            .is_some_and(|e| e.name != shagoff_core::panel::PARENT);
        // In the trash, where the item was deleted from takes the whole half: a path cut to `/h…`
        // between the totals says nothing.
        // With marks, their totals matter more.
        let p = &app.panes[side].active().panel;
        let origin = (p.current()).is_some_and(|e| e.target.is_some() && !e.is_link)
            && !p.has_marks()
            && shagoff_core::trashcan::is_top(p.cwd());
        let line = if side == app.active && origin {
            row![container(window_status(app)).width(Length::Fill).clip(true)]
        } else if side == app.active && extra {
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
        // Each half under its pane: the divider's share.
        let (left, right) = viewport::split_portions(app.split);
        line.spacing(12)
            .padding([0, 24, 0, 0])
            .align_y(Alignment::Center)
            .width(Length::FillPortion(if side == 0 { left } else { right }))
            .into()
    };
    // «Panel at 100 %»: only the shown pane's half, the whole width.
    let halves = if app.wide {
        row![half(app.active)]
    } else {
        row![half(0), half(1)]
    };
    let mut col = column![].push_maybe(job_line(app)).push_maybe(
        (app.config.show_status).then(|| halves.spacing(4).padding([2, 6]).height(Length::Shrink)),
    );
    // The built-in terminal takes the command line's place (TC has none; Double Commander's).
    if app.config.term_builtin && !app.term_full {
        col = col.push(crate::term::view(app, false));
    } else if app.config.show_cmdline && !app.config.term_builtin {
        col = col.push(command_line(app));
    }
    if app.config.show_fkeys {
        col = col.push(fkey_bar(&app.config.f12, app.scheme.paint()));
    }
    col.into()
}

/// TC `path>`: the active panel's dir and the command field.
fn command_line(app: &App) -> Element<'_, Message> {
    let cwd = app
        .shown(app.panes[app.active].active().panel.cwd())
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

/// The entry under the cursor (messages are a card of their own: `status_popup`).
fn window_status(app: &App) -> Element<'_, Message> {
    let p = &app.panes[app.active].active().panel;
    let details = p
        .current()
        .map(|e| {
            let size = |n| match app.config.human_sizes() {
                true => human(n),
                false => format::size(n),
            };
            format::details(e, &app.tz, &app.owners, size)
        })
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

pub fn fkey_bar(
    f12: &crate::config::ToolButton,
    paint: colors::PanelPaint,
) -> Element<'static, Message> {
    // A palette of the list's own: the keys in its cursor colour (NC's cyan keys).
    let class = move || match paint.back.and(paint.cursor) {
        Some(bg) => fkey_class(bg),
        None => theme::Button::Standard,
    };
    let f12 = match f12.cmd.trim() {
        "" => fl!("fkey-f12-none"),
        _ => format!("F12 {}", crate::toolbar::tip(f12)),
    };
    // Twelve buttons must fit a ~1000 px window: tight padding, one line, centred.
    let key = |label: String, action| {
        button::custom(
            text(label)
                .wrapping(cosmic::iced::widget::text::Wrapping::None)
                .width(Length::Fill)
                .align_x(cosmic::iced::alignment::Horizontal::Center),
        )
        .class(class())
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
        // Its own label can be long: cut at the end, not on both sides ("F12" stays readable).
        button::custom(
            text(f12)
                .wrapping(cosmic::iced::widget::text::Wrapping::None)
                .ellipsize(Ellipsize::End(EllipsizeHeightLimit::Lines(1)))
                .width(Length::Fill),
        )
        .class(class())
        .padding([6, 4])
        .on_press(Message::Key(Action::F12))
        .width(Length::Fill),
    ]
    .spacing(3)
    .padding(4)
    .into()
}

/// An F-key in a palette's cursor colour, black or white text on it; lighter while hovered.
fn fkey_class(bg: [u8; 3]) -> theme::Button {
    let rgb = |[r, g, b]: [u8; 3]| Color::from_rgb8(r, g, b);
    let ink = match colors::dark_text_on(bg) {
        true => Color::BLACK,
        false => Color::WHITE,
    };
    let style = move |hover: bool| {
        let mut b = cosmic::widget::button::Style::new();
        let c = rgb(bg);
        b.background = Some(
            match hover {
                true => Color { a: 0.8, ..c },
                false => c,
            }
            .into(),
        );
        b.text_color = Some(ink);
        b.border_radius = 4.0.into();
        b
    };
    theme::Button::Custom {
        active: Box::new(move |_, _| style(false)),
        disabled: Box::new(move |_| style(false)),
        hovered: Box::new(move |_, _| style(true)),
        pressed: Box::new(move |_, _| style(true)),
    }
}

/// The path line: the accent when active. With a palette of the list's own (TC Colors, the
/// «Blue» one) it is in the panel's colours, the active one in the cursor's (as NC / Far).
fn bar_style(active: bool, paint: colors::PanelPaint) -> theme::Container<'static> {
    theme::Container::custom(move |t| {
        let c = t.cosmic();
        let rgb = |[r, g, b]: [u8; 3]| Color::from_rgb8(r, g, b);
        if let Some(back) = paint.back {
            let bg = match active {
                true => paint.cursor.unwrap_or(back),
                false => back,
            };
            let ink = match (active, paint.text) {
                (false, Some(t)) => rgb(t),
                _ if colors::dark_text_on(bg) => Color::BLACK,
                _ => Color::WHITE,
            };
            return container::Style {
                background: Some(rgb(bg).into()),
                text_color: Some(ink),
                ..Default::default()
            };
        }
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
    bg: Option<[u8; 3]>,
    odd: bool,
    marks: colors::PanelPaint,
) -> theme::Container<'static> {
    let (fill, ink) = colors::row_paint(is_cursor, active, marked, bg, odd, marks);
    theme::Container::custom(move |t| {
        let c = t.cosmic();
        let rgb = |[r, g, b]: [u8; 3]| Color::from_rgb8(r, g, b);
        let mut style = container::Style::default();
        if skin == Skin::Modern {
            style.border.radius = c.corner_radii.radius_s.into();
        }
        // The button pairs: `accent.on` is not meant for text on the accent and was unreadable
        // (light on cyan in dark, dark on teal in light).
        let (background, on) = match fill {
            colors::Fill::None => (None, None),
            colors::Fill::Rgb(b) => (Some(rgb(b)), None),
            colors::Fill::Accent => (Some(c.accent_button.base.into()), Some(c.accent_button.on)),
            colors::Fill::Destructive => (
                Some(c.destructive_button.base.into()),
                Some(c.destructive_button.on),
            ),
            colors::Fill::Faint(b) => {
                let mut bg = b.map_or(Color::from(c.accent_color()), rgb);
                bg.a = 0.35;
                (Some(bg), None)
            }
        };
        style.background = background.map(Into::into);
        style.text_color = match ink {
            colors::Ink::Theme => None,
            colors::Ink::OnButton => on.map(Into::into),
            colors::Ink::Marked => Some(c.destructive_text_color().into()),
            colors::Ink::Rgb(i) => Some(rgb(i)),
        };
        style
    })
}
