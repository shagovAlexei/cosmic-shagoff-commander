//! Free helpers of the app module (tasks, process spawning, small conversions).

use super::*;

/// Scroll the viewer and record the offset: iced reports nothing when the content fits, and a
/// stale offset would leave the rows out of view.
pub(super) fn lister_scroll(l: &mut Lister, x: f32, y: f32) -> Task<Message> {
    l.offset = (x, y);
    scrollable::scroll_to(
        l.scroll.clone(),
        AbsoluteOffset {
            x: Some(x),
            y: Some(y),
        },
    )
}

/// An archive opened from inside another (155) is a copy in the temp dir: a change to it would
/// reach nothing the user keeps — refused, by the one place every archive change goes through.
pub(super) fn changes_nested(job: &Job) -> bool {
    let tmp = archive::temp_root();
    match job {
        Job::Repack { archive, .. } => archive.starts_with(&tmp),
        Job::Cross {
            from,
            to,
            move_after,
            ..
        } => to.starts_with(&tmp) || (*move_after && from.starts_with(&tmp)),
        _ => false,
    }
}

/// Window events → messages. Panel keys only when no widget took the event (`Ignored`).
pub(super) fn route_event(
    event: cosmic::iced::Event,
    status: event::Status,
    window: cosmic::iced::window::Id,
) -> Option<Message> {
    match event {
        // The hotkeys dialog waits for a key: the next one, whatever it is (not Escape).
        cosmic::iced::Event::Keyboard(keyboard::Event::KeyPressed {
            physical_key,
            modifiers,
            ..
        }) if keymap::CAPTURE.load(std::sync::atomic::Ordering::Relaxed)
            && keymap::key_name(physical_key).is_some() =>
        {
            let b = keymap::bind_of(physical_key, modifiers)?;
            Some(Message::KeyEdit(crate::hotkeys::KeyMsg::Captured(
                b.to_string(),
            )))
        }
        // Any status, any widget under it: the list's own mouse handling takes the release, and a
        // header-edge drag must end wherever the button comes up.
        cosmic::iced::Event::Mouse(cosmic::iced::mouse::Event::ButtonReleased(
            cosmic::iced::mouse::Button::Left,
        )) => Some(Message::ColDragEnd),
        cosmic::iced::Event::Mouse(cosmic::iced::mouse::Event::ButtonReleased(
            cosmic::iced::mouse::Button::Right,
        )) => Some(Message::RightUp),
        // The divider's share is of the window's width.
        // Ctrl / Shift let go in another window are never seen here: not held any more (a click
        // back in must not mark as Ctrl+click). Not left to the compositor's ModifiersChanged.
        cosmic::iced::Event::Window(cosmic::iced::window::Event::Unfocused) => {
            Some(Message::Modifiers(Modifiers::empty()))
        }
        cosmic::iced::Event::Window(cosmic::iced::window::Event::Closed) => {
            Some(Message::WindowClosed(window))
        }
        cosmic::iced::Event::Window(
            cosmic::iced::window::Event::Resized(size)
            | cosmic::iced::window::Event::Opened { size, .. },
        ) => Some(Message::WindowWidth(window, size.width)),
        // Where files are held, in window coordinates: a zone's own `on_drop` point is shifted by
        // the scrollable it sits in. The right-button drop menu opens at the last one.
        cosmic::iced::Event::Dnd(DndEvent::Offer(
            _,
            OfferEvent::Enter { x, y, .. } | OfferEvent::Motion { x, y },
        )) => Some(Message::DropAt(x, y)),
        // Any status: a focused text_input captures Escape to unfocus itself, and the dialog must still close.
        cosmic::iced::Event::Keyboard(keyboard::Event::KeyPressed {
            key: keyboard::Key::Named(keyboard::key::Named::Escape),
            modifiers,
            ..
        }) if modifiers.is_empty() => Some(Message::Escape(window)),
        // Any status: the search window's fields have focus, and they capture it.
        cosmic::iced::Event::Keyboard(keyboard::Event::KeyPressed {
            key,
            physical_key: physical_key @ keyboard::key::Physical::Code(keyboard::key::Code::KeyL),
            modifiers,
            ..
        }) if modifiers == Modifiers::ALT => Some(Message::FeedKey(
            (status == event::Status::Ignored)
                .then(|| keymap::action(&key, physical_key, modifiers))
                .flatten(),
        )),
        cosmic::iced::Event::Keyboard(keyboard::Event::KeyPressed {
            key,
            physical_key,
            modifiers,
            ..
        }) if status == event::Status::Ignored => keymap::action(&key, physical_key, modifiers)
            .map(Message::Key)
            .or_else(|| {
                let c = typed_char(&key, modifiers);
                match (keymap::lister_key(&key, physical_key, modifiers), c) {
                    (Some(k), Some(c)) => Some(Message::Letter(k, c)),
                    (Some(k), None) => Some(Message::ListerKey(k)),
                    (None, c) => c.map(Message::CmdType),
                }
            }),
        // A focused text field captures every key but Up/Down/Tab; pass on the ones it has no use for.
        cosmic::iced::Event::Keyboard(keyboard::Event::KeyPressed {
            key,
            physical_key,
            modifiers,
            ..
        }) if not_for_text(&key, physical_key, modifiers) => {
            keymap::action(&key, physical_key, modifiers)
                .map(Message::FieldKey)
                .or_else(|| {
                    // Shift+F3 in the viewer's search field: previous match.
                    keymap::lister_key(&key, physical_key, modifiers)
                        .filter(|k| *k == ListerKey::FindPrev)
                        .map(Message::ListerKey)
                })
        }
        cosmic::iced::Event::Keyboard(keyboard::Event::ModifiersChanged(m)) => {
            Some(Message::Modifiers(m))
        }
        _ => None,
    }
}

/// Keys a text field captures but does not edit with: F-keys, PgUp/PgDn, Insert and Ctrl combos
/// other than text editing and the clipboard (Ctrl+A/C/V/Z, Ctrl+arrows/Backspace/Delete/Home/End);
/// Ctrl+X goes on too: in the command line it is the next command (TC).
pub(super) fn not_for_text(
    key: &keyboard::Key,
    physical: keyboard::key::Physical,
    mods: Modifiers,
) -> bool {
    use keyboard::key::{Code, Named, Physical};
    match key {
        keyboard::Key::Named(n) => {
            let editing = matches!(
                n,
                Named::Backspace
                    | Named::Delete
                    | Named::Home
                    | Named::End
                    | Named::ArrowLeft
                    | Named::ArrowRight
                    | Named::Enter
                    | Named::Escape
            );
            matches!(
                n,
                Named::F1
                    | Named::F2
                    | Named::F3
                    | Named::F4
                    | Named::F5
                    | Named::F6
                    | Named::F7
                    | Named::F8
                    | Named::F9
                    | Named::F10
                    | Named::F11
                    | Named::F12
                    | Named::PageUp
                    | Named::PageDown
                    | Named::Insert
            ) || (mods.control() && !editing)
        }
        _ => {
            let clipboard = matches!(
                physical,
                Physical::Code(Code::KeyA | Code::KeyC | Code::KeyV | Code::KeyZ)
            );
            mods.control() && !clipboard
        }
    }
}

/// A printable character typed without Ctrl/Alt/Super: TC sends it to the command line.
pub(super) fn typed_char(key: &keyboard::Key, mods: Modifiers) -> Option<char> {
    let keyboard::Key::Character(s) = key else {
        return None;
    };
    let mut chars = s.chars();
    match (chars.next(), chars.next()) {
        // Space is a panel key (marking); Shift+Space must not start the line with a blank.
        (Some(c), None)
            if !c.is_control() && c != ' ' && !(mods.control() || mods.alt() || mods.logo()) =>
        {
            Some(c)
        }
        _ => None,
    }
}

/// Take the keyboard focus from every text field.
pub(super) fn unfocus() -> Task<Message> {
    widget::text_input::focus(widget::Id::unique())
}

pub(super) fn plan_error(e: &PlanError) -> String {
    match e {
        PlanError::IntoItself(p) => fl!("plan-into-itself", path = p.display().to_string()),
        PlanError::SameFile(p) => fl!("plan-same-file", path = p.display().to_string()),
        PlanError::BadName => fl!("rename-bad-name"),
        PlanError::Exists(p) => fl!("rename-exists", path = p.display().to_string()),
        PlanError::SameTarget(p) => fl!("plan-same-target", path = p.display().to_string()),
        PlanError::Empty => String::new(),
    }
}

/// Why F3 / F4 can't open `file` (shown as `name`), if they can't: a link to nothing says so,
/// anything else (deleted meanwhile, in a dir r-- without x) gives the system's reason.
pub(super) fn unopenable(file: &Path, name: &str) -> Option<String> {
    let e = std::fs::metadata(file).err()?;
    let link = std::fs::symlink_metadata(file).is_ok_and(|m| m.file_type().is_symlink());
    Some(if link && e.kind() == std::io::ErrorKind::NotFound {
        fl!("broken-link", name = name)
    } else {
        fl!("file-unreadable", name = name, err = crate::why::text(&e))
    })
}

/// Run a gio call off the UI thread.
pub(super) fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, mount::Error> + Send + 'static,
    msg: impl FnOnce(Result<T, mount::Error>) -> Message + Send + 'static,
) -> Task<Message> {
    Task::perform(
        async move {
            tokio::task::spawn_blocking(f)
                .await
                .unwrap_or_else(|e| Err(mount::Error::Failed(e.to_string())))
        },
        move |r| cosmic::Action::App(msg(r)),
    )
}

/// Alt+F1/F2: the volumes gio knows, appended to the open list when they arrive (~50 ms, D-Bus).
pub(super) fn list_volumes(side: usize) -> Task<Message> {
    blocking(mount::list, move |r| {
        Message::Volumes(side, r.unwrap_or_default())
    })
}

pub(super) fn mount_error(e: &mount::Error) -> String {
    match e {
        mount::Error::NeedPassword => fl!("mount-need-password"),
        mount::Error::WrongPassword => fl!("mount-wrong-password"),
        mount::Error::Question(q) => fl!("mount-question", text = q.replace('\n', " ")),
        mount::Error::Failed(s) => s.clone(),
        mount::Error::Cancelled => fl!("connect-cancelled"),
    }
}

/// A dir's entries, or the search results fed to the panel (re-read: deleted ones drop out).
/// `stop`: Esc during Ctrl+B's second step (the rows of millions of found files).
pub(super) fn read_listing(
    path: &Path,
    show_hidden: bool,
    results: Option<&Vec<PathBuf>>,
    stop: &AtomicBool,
) -> Result<Vec<Entry>, String> {
    match results {
        // A branch read inside an archive: from its index.
        Some(r) if archive::split_path(path).is_some() => Ok(archive::entries(r, path)),
        // Search results fed to the panel: the ones inside archives from their indexes.
        Some(r) if r.iter().any(|p| inside_archive(p)) => {
            let (arc, disk): (Vec<PathBuf>, Vec<PathBuf>) =
                r.iter().cloned().partition(|p| inside_archive(p));
            let mut v = listing::entries(&disk, path, stop);
            v.extend(archive::found_entries(&arc, path));
            Ok(v)
        }
        Some(r) => Ok(listing::entries(r, path, stop)),
        None => listing::scan(path, show_hidden).map_err(|e| crate::why::text(&e)),
    }
}

/// Ctrl+Shift+F5's work, off the UI thread (`ops::link`), its errors as the status line says
/// them. `Ok`: the sources linked and the line for the ones that failed; `Err`: nothing done.
pub(super) fn link_all(
    sources: &[PathBuf],
    target: &Path,
) -> Result<(Vec<PathBuf>, Option<String>), String> {
    let (done, failed) = ops::link(sources, target).map_err(|e| match e {
        ops::LinkError::Mkdir(path, e) => {
            fl!(
                "link-failed",
                path = path.display().to_string(),
                err = crate::why::text(&e)
            )
        }
        ops::LinkError::Plan(e) => plan_error(&e),
    })?;
    let msg = failed.first().map(|(path, e)| {
        let (path, err) = (path.display().to_string(), crate::why::text(e));
        match failed.len() {
            1 => fl!("link-failed", path = path, err = err),
            n => fl!("links-failed", n = n, path = path, err = err),
        }
    });
    Ok((done, msg))
}

/// Acts on the panel's files (marks, the row under the cursor, a file operation): nothing while a
/// tree (Ctrl+F8) is shown in their place.
pub(super) fn on_files(a: Action) -> bool {
    use Action as A;
    matches!(
        a,
        A::View
            | A::Edit
            | A::Copy
            | A::CopySame
            | A::Move
            | A::Rename
            | A::Delete
            | A::DeletePermanent
            | A::Mkdir
            | A::NewFile
            | A::NewFromTemplate
            | A::Link
            | A::Pack
            | A::MoveToArchive
            | A::Unpack
            | A::TestArchives
            | A::ChecksumCreate
            | A::ChecksumVerify
            | A::SplitFile
            | A::CombineFiles
            | A::Properties
            | A::SetDate
            | A::ViewOther
            | A::ShrinkSelection
            | A::Occupied
            | A::CountDirs
            | A::Mark
            | A::MarkDown
            | A::MarkUp
            | A::SelectGroup
            | A::UnselectGroup
            | A::ShowAll
            | A::ShowPrograms
            | A::ShowMask
            | A::SelectAll
            | A::UnselectAll
            | A::Invert
            | A::RestoreMarks
            | A::MarkExt
            | A::UnmarkExt
            | A::MarkName
            | A::UnmarkName
            | A::MarkNameExt
            | A::UnmarkNameExt
            | A::SaveMarks
            | A::SaveMarksToFile
            | A::LoadMarksFromFile
            | A::LoadMarksFromClip
            | A::NextMarked
            | A::PrevMarked
            | A::ShowOnlyMarked
            | A::CopyDirsOfFiles
            | A::CopyDetails
            | A::CopyDetailsHdr
            | A::SaveDetails
            | A::SaveDetailsHdr
            | A::ClipCut
            | A::ClipCopy
            | A::ClipPaste
            | A::MultiRename
            | A::CopyNames
            | A::CopyPaths
            | A::CmdName
            | A::CmdPath
            | A::ContextMenu
            | A::OpenWith
            | A::OpenLeft
            | A::OpenRight
            | A::Branch
            | A::BranchSel
            | A::CompareLists
            | A::CompareListsSubdirs
            | A::CompareListsDiffOnly
            | A::QuickFilter
            | A::QuickView
            | A::Sort(_)
            | A::RunTerminal
    )
}

/// Alt+F10: `dirs`' subdirs read off the UI thread (a dead mount must not hang it), then shown in
/// tree dialog `id`; the cursor on `select` if given (the way there shown even if hidden).
pub(super) fn tree_read(
    id: u64,
    dirs: Vec<PathBuf>,
    hidden: bool,
    select: Option<PathBuf>,
) -> Task<Message> {
    let toward = select.clone();
    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || {
                shagoff_core::tree::read(dirs, hidden, toward.as_deref())
            })
            .await
            .unwrap_or_default()
        },
        move |read| cosmic::Action::App(Message::TreeFilled(id, read, select.clone())),
    )
}

/// Count dirs `names` in `cwd` in the background; each sends its own `Message::DirSize`.
/// `stop`: the tab left `cwd` (a stopped count sends nothing).
// ponytail: one blocking task per dir; a queue if thousands of dirs ever matter.
pub(super) fn count_dirs(
    tab: u64,
    cwd: PathBuf,
    names: Vec<OsString>,
    stop: Arc<AtomicBool>,
) -> Task<Message> {
    Task::batch(names.into_iter().map(|name| {
        let path = cwd.join(&name);
        let cwd = cwd.clone();
        let stop = stop.clone();
        cosmic::iced::Task::future(async move {
            tokio::task::spawn_blocking(move || shagoff_core::props::usage(&[path], &stop))
                .await
                .ok()
                .flatten()
        })
        .and_then(move |u| {
            Task::done(cosmic::Action::App(Message::DirSize(
                tab,
                cwd.clone(),
                name.clone(),
                u.bytes,
            )))
        })
    }))
}

/// A menu name shown as is, with no hot letter: `&` markers dropped, plain `&`s kept (doubled).
pub(super) fn hotkey_free(name: &str) -> String {
    shagoff_core::quicksearch::hotkey(name).0.replace('&', "&&")
}

/// Scroll a list dialog in proportion to its cursor: the row is always in view (separators are
/// shorter, close enough).
pub(super) fn list_snap(cursor: usize, len: usize) -> Task<Message> {
    let y = cursor as f32 / len.saturating_sub(1).max(1) as f32;
    scrollable::snap_to(
        dialogs::list_scroll_id(),
        scrollable::RelativeOffset {
            x: None,
            y: Some(y),
        },
    )
}

/// "Open with…" rows: the programs, a line, "All programs…" (unless they are all shown already),
/// "Other program…".
pub(super) fn open_with_items(apps: Vec<openwith::App>, all: bool) -> Vec<ListItem> {
    let own = |label, kind| ListItem {
        label,
        path: PathBuf::new(),
        kind,
        icon: String::new(),
    };
    let mut items: Vec<ListItem> = (apps.into_iter())
        .map(|a| ListItem {
            label: a.name,
            path: a.desktop,
            kind: Item::App,
            icon: a.icon,
        })
        .collect();
    if !items.is_empty() {
        items.push(own(String::new(), Item::Sep));
    }
    if !all {
        items.push(own(fl!("open-with-all"), Item::All));
    }
    items.push(own(fl!("open-with-other"), Item::Other));
    items
}

/// Where a row's file lies: its dir and its own name — not `cwd` for search results fed to the
/// panel, whose rows carry whole paths (157: some inside archives).
pub(super) fn located(cwd: &Path, os_name: &std::ffi::OsStr) -> (PathBuf, OsString) {
    let full = cwd.join(os_name);
    match (full.parent(), full.file_name()) {
        (Some(d), Some(n)) => (d.to_path_buf(), n.to_os_string()),
        _ => (cwd.to_path_buf(), os_name.to_os_string()),
    }
}

/// A found path through an archive (`/x/a.zip/docs/f`): an ancestor has an archive's name. By
/// name only — a stat per result would stall on a million of them.
pub(super) fn inside_archive(p: &Path) -> bool {
    p.parent().is_some_and(shagoff_core::archive::within)
}

/// The dialog's search fields as a query (`hidden` is the panel's).
pub(super) fn find_query(
    f: &dialogs::Find,
    text: String,
) -> Result<shagoff_core::search::Query, String> {
    let number = |s: &str, what: String| -> Result<Option<u64>, String> {
        let s = s.trim();
        if s.is_empty() {
            return Ok(None);
        }
        s.parse::<u64>()
            .map(Some)
            .map_err(|_| fl!("find-bad-number", field = what, value = s))
    };
    let kb = |n: Option<u64>| n.map(|n| n.saturating_mul(1024));
    let days = number(&f.days, fl!("find-days"))?;
    let (min, max) = (
        kb(number(&f.min_size, fl!("find-min-size"))?),
        kb(number(&f.max_size, fl!("find-max-size"))?),
    );
    if let (Some(a), Some(b)) = (min, max)
        && a > b
    {
        return Err(fl!("find-bad-range"));
    }
    let attrs = shagoff_core::colors::attrs(&f.attrs)
        .ok_or_else(|| fl!("find-bad-attrs", value = f.attrs.trim()))?;
    let regex = match (f.regex, text.is_empty()) {
        (true, false) => Some(
            shagoff_core::search::regex(&text, f.case_sensitive)
                .map_err(|e| fl!("find-bad-regex", err = e))?,
        ),
        _ => None,
    };
    // `*` (the mask's default) and nothing mean any name, not a broken regex.
    let name_regex = match f.name_regex && !matches!(f.mask.trim(), "" | "*") {
        true => Some(
            shagoff_core::search::name_regex(f.mask.trim(), f.case_sensitive)
                .map_err(|e| fl!("find-bad-regex", err = e))?,
        ),
        false => None,
    };
    Ok(shagoff_core::search::Query {
        mask: Mask::parse_case(&f.mask, f.case_sensitive),
        name_regex,
        archives: f.archives,
        text: (regex.is_none() && !text.is_empty()).then_some(text),
        case_sensitive: f.case_sensitive,
        regex,
        min_size: min,
        max_size: max,
        // Huge day counts reach before 1970: from the start of time then (no overflow panic).
        newer_than: days.map(|d| {
            SystemTime::now()
                .checked_sub(std::time::Duration::from_secs(d.saturating_mul(86400)))
                .unwrap_or(SystemTime::UNIX_EPOCH)
        }),
        attrs: Some(attrs),
        ..Default::default()
    })
}

/// A typed target that lies inside an archive (`a.zip/x`). The archive file itself is not: packing
/// or copying onto an existing `a.zip` asks to replace it, as for any file.
pub(super) fn into_archive(p: &Path) -> bool {
    archive::split_path(p).is_some_and(|(_, inner)| !inner.as_os_str().is_empty())
}

/// Files → «Split file»: the part size offered.
pub(super) const SPLIT_SIZE: &str = "100M";

/// Split / Combine off the UI thread; its note or its error.
pub(super) fn pieces(
    f: impl FnOnce() -> std::io::Result<String> + Send + 'static,
) -> Task<Message> {
    Task::perform(
        async move {
            match tokio::task::spawn_blocking(f).await {
                Ok(Ok(note)) => Ok(note),
                Ok(Err(e)) => Err(crate::why::text(&e)),
                Err(e) => Err(e.to_string()),
            }
        },
        |r| cosmic::Action::App(Message::Pieces(r)),
    )
}

/// Default F5/F6 target: the other pane's dir with a trailing `/` (so it reads as "into this dir").
pub(super) fn dir_input(dir: &Path) -> String {
    let s = dir.display().to_string();
    if s.ends_with('/') { s } else { s + "/" }
}

/// Run `argv` without blocking the UI or leaving a zombie.
pub(super) fn spawn_detached(argv: &[OsString]) -> std::io::Result<()> {
    let (prog, args) = argv.split_first().ok_or(std::io::ErrorKind::InvalidInput)?;
    let child = std::process::Command::new(prog).args(args).spawn()?;
    reap(child);
    Ok(())
}

/// The installed terminals, read once at start (`.desktop` files and `gio`: not on the UI thread).
pub(super) fn scan_terminals() -> Task<Message> {
    use i18n_embed::LanguageLoader;
    let lang = (crate::i18n::LANGUAGE_LOADER.current_language().language).to_string();
    Task::perform(
        async move {
            let scan = tokio::task::spawn_blocking(move || terminals::scan(&lang)).await;
            let (list, default) = scan.unwrap_or_default();
            Message::Terminals(list, default)
        },
        cosmic::Action::App,
    )
}

/// A command line in `dir`, its output dropped (TC closes the console too).
pub(super) fn spawn_in(argv: &[String], dir: &Path) -> std::io::Result<()> {
    use std::process::Stdio;
    let (prog, args) = argv.split_first().ok_or(std::io::ErrorKind::InvalidInput)?;
    let child = std::process::Command::new(prog)
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    reap(child);
    Ok(())
}

/// Like `spawn_in`, but a failure is reported (`Message::ToolFailed`): a toolbar button's shell
/// always starts, so "command not found" and the like only show in its exit status and stderr.
/// `path`'s name if it is a file (on disk or in an archive: its dir's listing says), not a dir or
/// an archive to enter. Reads the disk: off the UI thread.
pub(super) fn file_in_its_dir(path: &Path) -> Option<String> {
    let (dir, name) = (path.parent()?, path.file_name()?.to_string_lossy());
    if path.is_dir()
        || archive::split_path(path).is_some_and(|(_, inner)| inner.as_os_str().is_empty())
    {
        return None;
    }
    let e = listing::scan(dir, true)
        .ok()?
        .into_iter()
        .find(|e| e.name == name)?;
    let enter = e.is_dir() || archive::Format::detect(&e.name).is_some_and(|f| f.is_tree());
    (!enter).then_some(e.name)
}

pub(super) fn run_watched(argv: &[String], dir: &Path) -> std::io::Result<Task<Message>> {
    use std::io::BufRead;
    use std::process::Stdio;
    let (prog, args) = argv.split_first().ok_or(std::io::ErrorKind::InvalidInput)?;
    let mut child = std::process::Command::new(prog)
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;
    let err = child.stderr.take();
    Ok(cosmic::iced::Task::future(async move {
        tokio::task::spawn_blocking(move || {
            // Only the last line is kept: a terminal may log for as long as it is open.
            let last = err.map_or_else(String::new, |e| {
                let lines = std::io::BufReader::new(e).lines().map_while(Result::ok);
                lines
                    .filter(|l| !l.trim().is_empty())
                    .last()
                    .unwrap_or_default()
            });
            let status = child.wait().ok()?;
            (!status.success()).then(|| {
                if last.is_empty() {
                    status.to_string()
                } else {
                    last
                }
            })
        })
        .await
        .ok()
        .flatten()
    })
    .and_then(|e| Task::done(cosmic::Action::App(Message::ToolFailed(e)))))
}

pub(super) fn reap(mut child: std::process::Child) {
    std::thread::spawn(move || child.wait());
}

/// A font by family name; "" = the interface's.
pub(super) fn font_named(name: &str) -> cosmic::iced::Font {
    match name.trim() {
        "" => cosmic::font::default(),
        // ponytail: leaks the name once per change of the setting (iced wants `&'static str`).
        n => cosmic::iced::Font {
            family: cosmic::iced::font::Family::Name(Box::leak(n.to_string().into_boxed_str())),
            ..cosmic::font::default()
        },
    }
}
