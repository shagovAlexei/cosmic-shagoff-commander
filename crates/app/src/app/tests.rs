use super::*;
use crate::config::{Config, LastTab, State};
use crate::dialogs::ListKind;
use cosmic::iced::keyboard::key::{Code, Named, Physical};
use cosmic::iced::keyboard::{Key, Location};
use shagoff_core::archive::Format;
use shagoff_core::diff::Outcome;
use shagoff_core::drives::Drive;
use shagoff_core::lister::Mode;
use shagoff_core::multirename::Case;
use shagoff_core::session::PaneState;

fn app_with(config: Config, state: State) -> App {
    App::build(Core::default(), config, state, None, std::env::temp_dir()).0
}

fn cwds(app: &App, side: usize) -> Vec<PathBuf> {
    app.panes[side]
        .items()
        .iter()
        .map(|t| t.panel.cwd().to_path_buf())
        .collect()
}

#[test]
fn spawn_detached_reports_missing_program() {
    let argv = [
        std::ffi::OsString::from("/nonexistent/shagoff-test-prog"),
        "x".into(),
    ];
    assert!(spawn_detached(&argv).is_err());
}

#[test]
fn open_with_lists_programs_and_other() {
    let mut app = app_with(Config::default(), State::default());
    // No programs: still "Other program…".
    let _ = app.update(Message::OpenWithApps(0, Vec::new(), Ok(Default::default())));
    let Some(Dialog::List { items, .. }) = &app.dialog else {
        panic!("no list")
    };
    assert_eq!(
        items.iter().map(|i| i.kind).collect::<Vec<_>>(),
        [Item::All, Item::Other]
    );
    app.dialog = None;
    let ed = openwith::App {
        name: "Editor".into(),
        icon: "ed".into(),
        desktop: "/usr/share/applications/ed.desktop".into(),
    };
    let found = openwith::Found {
        mime: "text/plain".into(),
        apps: vec![ed],
    };
    let files = vec![PathBuf::from("/x/a b.txt")];
    let _ = app.update(Message::OpenWithApps(0, files.clone(), Ok(found)));
    let Some(Dialog::List { kind, items, .. }) = &app.dialog else {
        panic!("no list")
    };
    assert_eq!(*kind, ListKind::OpenWith);
    assert_eq!(
        (items[0].label.as_str(), items[0].icon.as_str()),
        ("Editor", "ed")
    );
    let kinds: Vec<Item> = items.iter().map(|i| i.kind).collect();
    assert_eq!(kinds, [Item::App, Item::Sep, Item::All, Item::Other]);
    // "All programs…" replaces the list, without itself.
    let other = openwith::App {
        name: "Gimp".into(),
        icon: String::new(),
        desktop: "/usr/share/applications/gimp.desktop".into(),
    };
    let _ = app.update(Message::OpenWithAll(vec![other]));
    let Some(Dialog::List { items, .. }) = &app.dialog else {
        panic!("no list")
    };
    let kinds: Vec<Item> = items.iter().map(|i| i.kind).collect();
    assert_eq!(kinds, [Item::App, Item::Sep, Item::Other]);
    assert_eq!(items[0].label, "Gimp");
    // "Other program…" asks for a command, with the files taken when the list was asked for.
    let _ = app.update(Message::ListPick(2));
    let Some(Dialog::Input { op, sources, .. }) = &app.dialog else {
        panic!("no input")
    };
    assert_eq!((*op, sources.clone()), (InputOp::OpenWith, files));
}

#[test]
fn other_program_remembers_commands() {
    let mut app = app_with(Config::default(), State::default());
    let input = |app: &App| match &app.dialog {
        Some(Dialog::Input { input, .. }) => input.clone(),
        _ => panic!("no input"),
    };
    let found = || Ok(openwith::Found::default());
    // `true` / `false` with no file: nothing to open, the command is still remembered.
    for cmd in ["false", "true"] {
        let _ = app.update(Message::OpenWithApps(0, Vec::new(), found()));
        let _ = app.update(Message::ListPick(1)); // "Other program…"
        let _ = app.update(Message::DialogInput(cmd.into()));
        let _ = app.update(Message::DialogSubmit);
    }
    assert_eq!(app.other_cmds, ["true", "false"]);
    let _ = app.update(Message::OpenWithApps(0, Vec::new(), found()));
    let _ = app.update(Message::ListPick(1));
    assert_eq!(input(&app), "true"); // opens with the last one
    let _ = app.update(Message::FieldKey(Action::Up));
    assert_eq!(input(&app), "false");
}

#[test]
fn drive_dialog_navigates_and_opens() {
    let mut app = app_with(Config::default(), State::default());
    app.drives = vec![
        Drive {
            label: "/".into(),
            path: "/".into(),
        },
        Drive {
            label: "tmp".into(),
            path: std::env::temp_dir(),
        },
    ];
    let _ = app.update(Message::Key(Action::Drives(1)));
    assert!(matches!(
        app.dialog,
        Some(Dialog::List {
            kind: ListKind::Drives,
            side: 1,
            ..
        })
    ));
    let _ = app.update(Message::Key(Action::Up));
    let _ = app.update(Message::Key(Action::Up)); // clamped at 0
    assert!(matches!(app.dialog, Some(Dialog::List { cursor: 0, .. })));
    let _ = app.update(Message::Key(Action::Down));
    let _ = app.update(Message::Key(Action::Down)); // clamped at 1
    assert!(matches!(app.dialog, Some(Dialog::List { cursor: 1, .. })));
    // A letter jumps as in a menu, and never reaches the command line.
    let _ = app.update(Message::CmdType('/'));
    let _ = app.update(Message::Key(Action::Up));
    let _ = app.update(Message::CmdType('T'));
    assert!(matches!(app.dialog, Some(Dialog::List { cursor: 1, .. })));
    assert!(app.cmdline.is_empty());
    app.panes[1].active_mut().pending = None;
    let _ = app.update(Message::Key(Action::Enter));
    assert!(app.dialog.is_none());
    assert_eq!(app.active, 1);
    assert!(app.panes[1].active().pending.is_some());
}

fn entry(name: &str) -> Entry {
    Entry {
        name: name.into(),
        os_name: name.into(),
        ext: String::new(),
        size: 0,
        mtime: std::time::UNIX_EPOCH,
        kind: shagoff_core::listing::Kind::File,
        is_link: false,
        mode: 0o644,
        owner: None,
        target: None,
    }
}

#[test]
fn a_big_list_is_sorted_by_a_background_read_keeping_cursor_and_marks() {
    use shagoff_core::sort::{Sort, SortKey};
    let mut app = app_with(Config::default(), State::default());
    let dir = PathBuf::from("/big");
    let rows = |n: usize| -> Vec<Entry> {
        (0..n)
            .map(|i| Entry {
                size: (n - i) as u64, // by size: reversed
                ..entry(&format!("f{i:05}"))
            })
            .collect()
    };
    let t = app.panes[0].active_mut();
    t.pending = None; // the startup read, never answered here
    let p = &mut t.panel;
    p.set_listing(dir.clone(), rows(SORT_HERE_MAX + 1), None);
    p.set_cursor(2); // "f00001"
    p.toggle_mark();
    let _ = app.update(Message::Key(Action::Sort(SortKey::Size)));
    // Not sorted here: the rows stay, the sort is set, the dir is read again.
    let t = app.panes[0].active();
    assert_eq!(t.panel.sort().key, SortKey::Size);
    assert_eq!(t.panel.entries()[1].name, "f00000");
    assert_eq!(pending_path(&app), Some(dir.clone()));
    let mut sorted = rows(SORT_HERE_MAX + 1);
    let sort = Sort {
        key: SortKey::Size,
        asc: true,
    };
    shagoff_core::sort::sort_entries(&mut sorted, sort, &HashMap::new());
    let (id, generation) = (t.id, t.pending.as_ref().unwrap().0);
    let _ = app.update(Message::Listed {
        tab: id,
        generation,
        path: dir,
        focus: None,
        result: Ok(sorted),
        space: None,
    });
    let p = &app.panes[0].active().panel;
    assert_eq!(p.entries()[1].name, format!("f{SORT_HERE_MAX:05}"));
    assert_eq!(p.current().unwrap().name, "f00001"); // the cursor stays on its file
    assert_eq!(p.targets(), [PathBuf::from("/big/f00001")]); // still marked
}

#[test]
fn regression_second_scan_of_same_path_wins() {
    let mut app = app_with(Config::default(), State::default());
    let (id, cwd) = (
        app.panes[0].active().id,
        app.panes[0].active().panel.cwd().to_path_buf(),
    );
    let _ = app.load(0, cwd.clone(), None);
    let first = app.panes[0].active().pending.as_ref().unwrap().0;
    let _ = app.load(0, cwd.clone(), None);
    let second = app.panes[0].active().pending.as_ref().unwrap().0;
    let listed = |generation, name: &str| Message::Listed {
        tab: id,
        generation,
        path: cwd.clone(),
        focus: None,
        result: Ok(vec![entry(name)]),
        space: None,
    };
    let _ = app.update(listed(second, "new"));
    let _ = app.update(listed(first, "old")); // late, stale
    let names: Vec<_> = app.panes[0]
        .active()
        .panel
        .entries()
        .iter()
        .map(|e| e.name.clone())
        .collect();
    assert!(
        names.contains(&"new".to_string()) && !names.contains(&"old".to_string()),
        "{names:?}"
    );
}

/// Start navigating pane 0 to a fresh temp dir; returns (dir guard, tab id, generation).
fn navigating(app: &mut App) -> (tempfile::TempDir, u64, u64) {
    let tmp = tempfile::tempdir().unwrap();
    let _ = app.load(0, tmp.path().into(), None);
    let t = app.panes[0].active();
    let generation = t.pending.as_ref().unwrap().0;
    (tmp, t.id, generation)
}

fn listed_ok(id: u64, generation: u64, path: &Path) -> Message {
    Message::Listed {
        tab: id,
        generation,
        path: path.into(),
        focus: None,
        result: Ok(vec![]),
        space: Some((1, 2)),
    }
}

#[test]
fn regression_watcher_does_not_cancel_navigation() {
    let mut app = app_with(Config::default(), State::default());
    let (tmp, id, generation) = navigating(&mut app);
    let _ = app.update(Message::Changed(0)); // old dir changed while the new one loads
    let _ = app.update(listed_ok(id, generation, tmp.path()));
    assert_eq!(app.panes[0].active().panel.cwd(), tmp.path());
}

#[test]
fn regression_a_finished_job_does_not_cancel_navigation() {
    let mut app = app_with(Config::default(), State::default());
    let gone = tempfile::NamedTempFile::new().unwrap().into_temp_path();
    let paths = vec![gone.to_path_buf()];
    let job = Job::Delete {
        paths,
        permanent: true,
    };
    let _ = app.start_job(0, OpKind::Delete, job, None);
    let _ = app.update(Message::JobHide); // in the background
    let (tmp, id, _) = navigating(&mut app);
    let _ = app.finish_job(&Report::default()); // it ends while the new dir loads
    assert!(app.job.is_none());
    // Still going there (read again), not back to the dir it left.
    assert_eq!(pending_path(&app).as_deref(), Some(tmp.path()));
    let generation = app.panes[0].active().pending.as_ref().unwrap().0;
    let _ = app.update(listed_ok(id, generation, tmp.path()));
    assert_eq!(app.panes[0].active().panel.cwd(), tmp.path());
}

#[test]
fn regression_watcher_leaves_search_results_alone() {
    let mut app = app_with(Config::default(), State::default());
    let t = app.panes[0].active_mut();
    t.pending = None;
    t.results = Some((t.panel.cwd().to_path_buf(), Arc::new(Vec::new())));
    let _ = app.update(Message::Changed(0));
    assert_eq!(pending_path(&app), None); // not read again
}

#[test]
fn regression_ctrl_r_before_the_results_arrive_keeps_them() {
    let mut app = app_with(Config::default(), State::default());
    let root = tempfile::tempdir().unwrap();
    let t = app.panes[0].active_mut();
    assert_ne!(t.panel.cwd(), root.path());
    // «В панель»: the results are set, their listing not here yet.
    t.results = Some((root.path().to_path_buf(), Arc::new(Vec::new())));
    t.pending = Some((1, root.path().to_path_buf()));
    let _ = app.update(Message::Key(Action::Reload));
    let t = app.panes[0].active();
    assert!(t.results.is_some());
    assert_eq!(pending_path(&app).as_deref(), Some(root.path()));
}

#[test]
fn regression_ctrl_t_does_not_copy_the_filter() {
    let mut app = app_with(Config::default(), State::default());
    app.panes[0]
        .active_mut()
        .panel
        .set_filter(Some("*.rs".into()));
    let _ = app.update(Message::Key(Action::NewTab));
    assert_eq!(app.panes[0].active().panel.filter(), None);
    let _ = app.update(Message::Key(Action::PrevTab));
    assert_eq!(app.panes[0].active().panel.filter(), Some("*.rs")); // the old tab keeps it
}

#[test]
fn regression_click_in_the_other_pane_clears_the_error_like_tab() {
    let mut app = app_with(Config::default(), State::default());
    app.panes[0].active_mut().error = Some("no access".into());
    let _ = app.update(Message::Click(1, 0));
    assert_eq!(
        (app.active, app.panes[0].active().error.as_deref()),
        (1, None)
    );
}

#[test]
fn status_card_closes_by_its_cross() {
    let mut app = app_with(Config::default(), State::default());
    app.say(
        StatusKind::Error,
        "a long message that no longer has to fit half a line".into(),
    );
    assert!(app.msg().is_some());
    let _ = app.update(Message::Dismiss(None));
    assert!(app.msg().is_none());
    app.panes[1].active_mut().error = Some("No such file or directory".into());
    let _ = app.update(Message::Dismiss(Some(1)));
    assert!(app.panes[1].active().error.is_none());
}

#[test]
fn regression_alt_left_to_a_deleted_dir_keeps_history_on_the_dir_shown() {
    let tmp = tempfile::tempdir().unwrap();
    let (a, b) = (tmp.path().join("a"), tmp.path().join("b"));
    std::fs::create_dir_all(b.join("sub")).unwrap();
    std::fs::create_dir(&a).unwrap();
    let mut app = app_with(Config::default(), State::default());
    listed_at(&mut app, 0, &a);
    listed_at(&mut app, 0, &b);
    std::fs::remove_dir(&a).unwrap();
    let _ = app.update(Message::Key(Action::HistoryBack));
    let t = app.panes[0].active();
    let (id, generation) = (t.id, t.pending.as_ref().unwrap().0);
    let _ = app.update(Message::Listed {
        tab: id,
        generation,
        path: a.clone(),
        focus: None,
        result: Err("No such file or directory".into()),
        space: None,
    });
    listed_at(&mut app, 0, &b.join("sub")); // Enter
    let recent = app.panes[0].active().history.recent();
    assert_eq!(recent[..2], [b.join("sub"), b.clone()]); // /b is not lost
}

#[test]
fn regression_ctrl_h_keeps_pending_navigation() {
    let mut app = app_with(Config::default(), State::default());
    let (tmp, _, _) = navigating(&mut app);
    let _ = app.update(Message::Key(Action::ToggleHidden));
    let pending = app.panes[0]
        .active()
        .pending
        .as_ref()
        .map(|(_, p)| p.clone());
    assert_eq!(pending.as_deref(), Some(tmp.path()));
}

#[test]
fn listed_brings_free_space_of_the_active_tab() {
    let mut app = app_with(Config::default(), State::default());
    let (tmp, id, generation) = navigating(&mut app);
    let _ = app.update(listed_ok(id, generation, tmp.path()));
    assert_eq!(app.space[0], Some((1, 2)));
}

#[test]
fn build_restores_tabs_and_active_pane() {
    let tmp = tempfile::tempdir().unwrap();
    let a = tmp.path().join("a");
    std::fs::create_dir(&a).unwrap();
    let state = State {
        panes: [
            PaneState {
                tabs: vec![tmp.path().into(), a.clone()],
                active: 1,
                ..PaneState::default()
            },
            PaneState {
                tabs: vec![tmp.path().join("gone")],
                active: 0,
                ..PaneState::default()
            },
        ],
        active: 1,
        ..State::default()
    };
    let app = app_with(Config::default(), state);
    assert_eq!(cwds(&app, 0), [tmp.path().to_path_buf(), a]);
    assert_eq!(app.panes[0].active_index(), 1);
    // Not looked at on the UI thread (a dead mount would hang the start): the tab's first
    // read goes up to an existing dir (`session::existing_dir`, in the background).
    assert_eq!(cwds(&app, 1), [tmp.path().join("gone")]);
    assert_eq!(app.active, 1);
}

#[test]
fn regression_sort_is_kept_per_tab_across_runs() {
    // Every start sorted by name: the tab's sort was never saved.
    use shagoff_core::sort::{Sort, SortKey};
    let by_date = Sort {
        key: SortKey::Date,
        asc: false,
    };
    let state = State {
        panes: [
            PaneState {
                tabs: vec!["/".into(), "/".into()],
                sort: vec![by_date],
                ..PaneState::default()
            },
            PaneState::default(),
        ],
        ..State::default()
    };
    let mut app = app_with(Config::default(), state);
    assert_eq!(app.panes[0].items()[0].panel.sort(), by_date);
    assert_eq!(app.panes[0].items()[1].panel.sort(), Sort::default());
    app.save_state();
    assert_eq!(app.saved.panes[0].sort, [by_date, Sort::default()]);
}

#[test]
fn regression_gvfs_dir_is_never_read_on_the_ui_thread() {
    // A hung network mount blocks the gvfs dir: listing it in `update` froze the window.
    let mut app = app_with(Config::default(), State::default());
    let net = Drive {
        label: "net".into(),
        path: "/run/user/1000/gvfs/sftp:host=h".into(),
    };
    app.gvfs_busy = true; // the read is under way
    app.refresh_mounts();
    assert!(app.gvfs_want && !app.drives.contains(&net));
    let _ = app.read_gvfs(); // one at a time: a hung read does not pile up threads
    assert!(app.gvfs_want);
    let _ = app.handle(Message::Gvfs(vec![net.clone()]));
    assert!(!app.gvfs_busy && app.drives.contains(&net));
    app.refresh_mounts(); // kept while the next read runs
    assert!(app.drives.contains(&net));
}

#[test]
fn argv_path_replaces_the_active_left_tab() {
    let tmp = tempfile::tempdir().unwrap();
    let state = State {
        panes: [
            PaneState {
                tabs: vec!["/".into(), "/".into()],
                active: 1,
                ..PaneState::default()
            },
            PaneState::default(),
        ],
        active: 0,
        ..State::default()
    };
    let app = App::build(
        Core::default(),
        Config::default(),
        state,
        Some(tmp.path().into()),
        "/".into(),
    )
    .0;
    assert_eq!(
        cwds(&app, 0),
        [PathBuf::from("/"), tmp.path().canonicalize().unwrap()]
    );
}

#[test]
fn argv_file_opens_its_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("f.txt");
    std::fs::write(&file, "x").unwrap();
    let app = App::build(
        Core::default(),
        Config::default(),
        State::default(),
        Some(file),
        "/".into(),
    )
    .0;
    assert_eq!(cwds(&app, 0), [tmp.path().canonicalize().unwrap()]);
}

#[test]
fn regression_last_tab_home_dir_with_tilde_or_gone() {
    let config = Config {
        last_tab_close: LastTab::Home,
        home_dir: Some("~/shagoff-no-such-dir".into()),
        ..Config::default()
    };
    let mut app = app_with(config, State::default());
    let _ = app.update(Message::Key(Action::CloseTab));
    let pending = app.panes[0]
        .active()
        .pending
        .as_ref()
        .map(|(_, p)| p.clone());
    assert_eq!(pending, Some(std::env::temp_dir())); // `~` = app.home, missing dir → parent
}

#[test]
fn regression_drive_list_is_fixed_while_dialog_open() {
    let mut app = app_with(Config::default(), State::default());
    let drive = |label: &str, path: &str| Drive {
        label: label.into(),
        path: path.into(),
    };
    app.drives = vec![drive("/", "/"), drive("usr", "/usr"), drive("etc", "/etc")];
    let _ = app.update(Message::Key(Action::Drives(0))); // cursor on "/" (cwd is temp_dir)
    let _ = app.update(Message::Key(Action::Down));
    let _ = app.update(Message::Key(Action::Down)); // on "etc"
    app.drives.remove(1); // a stick was pulled: indices shift under the open dialog
    app.panes[0].active_mut().pending = None;
    let _ = app.update(Message::Key(Action::Enter));
    let pending = app.panes[0]
        .active()
        .pending
        .as_ref()
        .map(|(_, p)| p.clone());
    assert_eq!(pending, Some(PathBuf::from("/etc")));
}

fn pending_path(app: &App) -> Option<PathBuf> {
    app.panes[0]
        .active()
        .pending
        .as_ref()
        .map(|(_, p)| p.clone())
}

#[test]
fn the_selection_goes_to_a_file_and_back() {
    let d = tempfile::tempdir().unwrap();
    let mut app = app_with(Config::default(), State::default());
    let rows = vec![entry("a.txt"), entry("b.txt"), entry("c.txt")];
    let p = &mut app.panes[0].active_mut().panel;
    p.set_listing(d.path().to_path_buf(), rows, None);
    // Nothing marked: nothing to save, no dialog.
    let _ = app.update(Message::Key(Action::SaveMarksToFile));
    assert!(app.dialog.is_none());
    let p = &mut app.panes[0].active_mut().panel;
    p.mark_names(&["a.txt".into(), "c.txt".into()]);
    let _ = app.update(Message::Key(Action::SaveMarksToFile));
    let _ = app.update(Message::DialogSubmit); // the offered name, in the panel's dir
    let file = d.path().join(MARKS_FILE);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "a.txt\nc.txt\n");
    app.panes[0].active_mut().panel.mark_names(&[]);
    let _ = app.update(Message::Key(Action::LoadMarksFromFile));
    let _ = app.update(Message::DialogSubmit);
    assert_eq!(
        app.panes[0].active().panel.marked_names(),
        ["a.txt", "c.txt"]
    );
    // The clipboard's text: one more.
    let _ = app.update(Message::MarksText(0, Some("b.txt\n".into())));
    assert_eq!(app.panes[0].active().panel.marked_names().len(), 3);
    // Saved again: the file is there, kept as it was.
    let _ = app.update(Message::Key(Action::SaveMarksToFile));
    let _ = app.update(Message::DialogSubmit);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "a.txt\nc.txt\n");
}

#[test]
fn details_go_to_a_file_and_paths_to_the_clipboard() {
    let d = tempfile::tempdir().unwrap();
    let mut app = app_with(Config::default(), State::default());
    let rows = vec![entry("a.txt"), entry("b")];
    let p = &mut app.panes[0].active_mut().panel;
    p.set_listing(d.path().to_path_buf(), rows, None);
    let _ = app.update(Message::Key(Action::SaveDetailsHdr));
    let _ = app.update(Message::DialogSubmit);
    let text = std::fs::read_to_string(d.path().join(DETAILS_FILE)).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 3); // the titles, then every row but ".."
    assert!(lines[0].starts_with(&format!("{}\t", fl!("col-name"))));
    assert!(lines[1].starts_with("a.txt\t\t")); // `entry` leaves the extension empty
    // The other panel's path: said (and put in the clipboard).
    let o = tempfile::tempdir().unwrap();
    let p = &mut app.panes[1].active_mut().panel;
    p.set_listing(o.path().to_path_buf(), Vec::new(), None);
    let other = o.path().display().to_string();
    let _ = app.update(Message::Key(Action::CopyTrgPath));
    assert!(app.status.as_ref().is_some_and(|s| s.text.contains(&other)));
}

#[test]
fn the_start_menu_is_edited_apart_from_the_bar_and_runs_its_items() {
    let mut app = app_with(Config::default(), State::default());
    let bar = app.config.toolbar.clone();
    let _ = app.update(Message::Key(Action::ConfigureStartMenu));
    use crate::toolbar::ToolMsg;
    for m in [
        ToolMsg::Reset,
        ToolMsg::Add,
        ToolMsg::Cmd("cm_Exchange".into()),
        ToolMsg::Add,
    ] {
        let _ = app.update(Message::ToolEdit(m)); // no defaults to reset to
    }
    let _ = app.update(Message::DialogSubmit);
    assert_eq!(app.config.toolbar, bar);
    assert_eq!(app.config.start_menu.len(), 1); // the empty one is not kept
    // Its item runs as a button does: here the internal command, panels swapped.
    let left = app.panes[0].active().id;
    let _ = app.update(Message::Key(Action::UserMenu(0)));
    assert_eq!(app.panes[1].active().id, left);
    let _ = app.update(Message::Key(Action::UserMenu(5))); // gone meanwhile: nothing
}

#[test]
fn sync_change_dir_takes_the_other_panel_along() {
    let mut app = app_with(Config::default(), State::default());
    let other = |app: &App| {
        app.panes[1]
            .active()
            .pending
            .as_ref()
            .map(|(_, p)| p.clone())
    };
    let docs = Entry {
        kind: shagoff_core::listing::Kind::Dir,
        ..entry("docs")
    };
    app.panes[0]
        .active_mut()
        .panel
        .set_listing("/a".into(), vec![docs.clone()], None);
    app.panes[1]
        .active_mut()
        .panel
        .set_listing("/b".into(), vec![docs], None);
    app.panes[1].active_mut().pending = None; // its start-up scan
    // Off: the other stays.
    let _ = app.load(0, "/a/docs".into(), None);
    assert_eq!(other(&app), None);
    app.panes[0].active_mut().pending = None;
    let _ = app.update(Message::Key(Action::SyncChangeDir));
    assert!(app.sync_dirs);
    let _ = app.load(0, "/a/docs".into(), None);
    assert_eq!(other(&app), Some(PathBuf::from("/b/docs")));
    // Deeper at once: it stays.
    app.panes[1].active_mut().pending = None;
    app.panes[1]
        .active_mut()
        .panel
        .set_listing("/b/docs".into(), Vec::new(), None);
    app.panes[0].active_mut().pending = None;
    app.panes[0]
        .active_mut()
        .panel
        .set_listing("/a/docs".into(), Vec::new(), None);
    let _ = app.load(0, "/a/docs/x/y".into(), None);
    assert_eq!(other(&app), None);
    // Up: it goes up too, even while its last step is still being read.
    app.panes[0].active_mut().pending = None;
    let _ = app.load(0, "/a".into(), None);
    assert_eq!(other(&app), Some(PathBuf::from("/b")));
    // A locked tab there stays.
    app.panes[1].active_mut().pending = None;
    app.panes[1].active_mut().locked = true;
    let _ = app.load(0, "/a/docs".into(), None);
    assert_eq!(other(&app), None);
    app.panes[1].active_mut().locked = false;
    app.panes[1].active_mut().panel.set_listing(
        "/b".into(),
        vec![Entry {
            kind: shagoff_core::listing::Kind::Dir,
            ..entry("docs")
        }],
        None,
    );
    // A dir it lacks: it stays where it is.
    app.panes[0].active_mut().pending = None;
    app.panes[0]
        .active_mut()
        .panel
        .set_listing("/a".into(), Vec::new(), None);
    let _ = app.load(0, "/a/src".into(), None);
    assert_eq!(other(&app), None);
    // Other panel → the same dir.
    app.panes[0].active_mut().pending = None;
    let _ = app.update(Message::Key(Action::MatchSrc));
    assert_eq!(other(&app), Some(PathBuf::from("/a")));
}

#[test]
fn regression_backspace_while_loading_goes_up_from_the_target() {
    let mut app = app_with(Config::default(), State::default());
    let _ = app.load(0, "/usr/share/doc".into(), None); // slow fs: not listed yet
    let _ = app.update(Message::Key(Action::Parent));
    assert_eq!(pending_path(&app), Some(PathBuf::from("/usr/share")));
    let _ = app.update(Message::Key(Action::Parent));
    assert_eq!(pending_path(&app), Some(PathBuf::from("/usr")));
}

#[test]
fn enter_while_loading_is_ignored() {
    // The rows on screen belong to the dir being left; entering one would undo the navigation.
    let mut app = app_with(Config::default(), State::default());
    let t = app.panes[0].active();
    let (id, generation) = (t.id, t.pending.as_ref().unwrap().0);
    let _ = app.update(listed_ok(id, generation, &std::env::temp_dir())); // shows ".."
    let _ = app.load(0, "/usr".into(), None);
    let _ = app.update(Message::Key(Action::Enter)); // cursor on ".." of the old dir
    assert_eq!(pending_path(&app), Some(PathBuf::from("/usr")));
}

#[test]
fn f7_made_in_the_background_rereads_or_says_why() {
    let mut app = app_with(Config::default(), State::default());
    let t = app.panes[0].active();
    let (id, generation) = (t.id, t.pending.as_ref().unwrap().0);
    let tmp = std::env::temp_dir();
    let _ = app.update(listed_ok(id, generation, &tmp));
    let made = |made| Message::Made {
        side: 0,
        cwd: tmp.clone(),
        name: "new/sub".into(),
        first: Some("new".into()),
        made,
    };
    let _ = app.update(made(Err("Permission denied".into())));
    assert!(app.msg().is_some_and(|m| m.contains("Permission denied")));
    let _ = app.update(made(Ok(())));
    assert_eq!(pending_path(&app), Some(tmp.clone())); // read again, the cursor onto "new"
}

#[test]
fn regression_enter_works_again_while_a_read_hangs() {
    let mut app = app_with(Config::default(), State::default());
    let t = app.panes[0].active();
    let (id, generation) = (t.id, t.pending.as_ref().unwrap().0);
    let tmp = std::env::temp_dir();
    let _ = app.update(listed_ok(id, generation, &tmp)); // shows ".."
    let _ = app.load(0, "/dead/mount".into(), None); // never answers
    app.panes[0].active_mut().pending_at -= ENTER_WAIT;
    let _ = app.update(Message::Key(Action::Enter)); // ".." of the dir shown
    assert_eq!(pending_path(&app), tmp.parent().map(Path::to_path_buf));
}

#[test]
fn listing_error_names_the_dir_and_clears_on_next_key() {
    let mut app = app_with(Config::default(), State::default());
    let (id, generation) = {
        let _ = app.load(0, "/root/secret".into(), None);
        let t = app.panes[0].active();
        (t.id, t.pending.as_ref().unwrap().0)
    };
    let _ = app.update(Message::Listed {
        tab: id,
        generation,
        path: "/root/secret".into(),
        focus: None,
        result: Err("Permission denied".into()),
        space: None,
    });
    let err = app.panes[0].active().error.clone().unwrap_or_default();
    assert!(
        err.contains("/root/secret") && err.contains("Permission denied"),
        "{err}"
    );
    let _ = app.update(Message::Key(Action::Down));
    assert!(app.panes[0].active().error.is_none());
}

#[test]
fn regression_f3_on_broken_symlink_reports_it() {
    let tmp = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink("nowhere", tmp.path().join("dangling")).unwrap();
    let config = Config {
        viewer: vec!["/nonexistent/viewer".into()], // never launch anything from a test
        ..Config::default()
    };
    let mut app = app_with(config, State::default());
    let _ = app.load(0, tmp.path().into(), None);
    let t = app.panes[0].active();
    let (id, generation) = (t.id, t.pending.as_ref().unwrap().0);
    let entries = listing::scan(tmp.path(), false).unwrap();
    let _ = app.update(Message::Listed {
        tab: id,
        generation,
        path: tmp.path().into(),
        focus: Some("dangling".into()),
        result: Ok(entries),
        space: None,
    });
    let _ = app.update(Message::Key(Action::View));
    let err = app.msg().unwrap_or_default().to_string();
    assert_eq!(err, fl!("broken-link", name = "dangling"));
}

#[test]
fn regression_unopenable_says_why_not_always_broken_link() {
    let tmp = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink("nowhere", tmp.path().join("dangling")).unwrap();
    assert_eq!(
        unopenable(&tmp.path().join("dangling"), "dangling"),
        Some(fl!("broken-link", name = "dangling"))
    );
    // Deleted meanwhile: not a link, the system's reason.
    let gone = unopenable(&tmp.path().join("gone.txt"), "gone.txt").unwrap();
    assert_ne!(gone, fl!("broken-link", name = "gone.txt"));
    assert!(gone.contains("gone.txt"));
    std::fs::write(tmp.path().join("ok"), "x").unwrap();
    assert_eq!(unopenable(&tmp.path().join("ok"), "ok"), None);
}

/// Pane 0 in a temp dir with `sub/`, `a.txt` ("alpha\nfoo\n"), `b.bin`; cursor on `a.txt`.
fn lister_app(config: Config) -> (App, tempfile::TempDir) {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir(tmp.path().join("sub")).unwrap();
    std::fs::write(tmp.path().join("a.txt"), "alpha\nfoo\n").unwrap();
    std::fs::write(tmp.path().join("b.bin"), b"x\0y").unwrap();
    let mut app = app_with(config, State::default());
    let _ = app.load(0, tmp.path().into(), None);
    let t = app.panes[0].active();
    let (id, generation) = (t.id, t.pending.as_ref().unwrap().0);
    let entries = listing::scan(tmp.path(), false).unwrap();
    let _ = app.update(Message::Listed {
        tab: id,
        generation,
        path: tmp.path().into(),
        focus: Some("a.txt".into()),
        result: Ok(entries),
        space: None,
    });
    (app, tmp)
}

#[test]
fn the_trash_deletes_for_good_and_restores() {
    let tmp = tempfile::tempdir().unwrap();
    use std::os::unix::fs::MetadataExt;
    let uid = std::fs::metadata(tmp.path()).unwrap().uid();
    let root = tmp.path().join(format!(".Trash-{uid}"));
    let files = root.join("files");
    std::fs::create_dir_all(&files).unwrap();
    std::fs::create_dir_all(root.join("info")).unwrap();
    std::fs::write(files.join("a.txt"), "x").unwrap();
    let from = tmp.path().join("docs/a.txt");
    std::fs::write(
        root.join("info/a.txt.trashinfo"),
        "[Trash Info]\nPath=docs/a.txt\n",
    )
    .unwrap();
    let mut app = app_with(Config::default(), State::default());
    let _ = app.load(0, files.clone(), None);
    let t = app.panes[0].active();
    let (id, generation) = (t.id, t.pending.as_ref().unwrap().0);
    let _ = app.update(Message::Listed {
        tab: id,
        generation,
        path: files.clone(),
        focus: Some("a.txt".into()),
        result: Ok(listing::scan(&files, false).unwrap()),
        space: None,
    });
    // F8 there is for good (never into the trash again).
    let _ = app.update(Message::Key(Action::Delete));
    assert!(matches!(
        app.dialog,
        Some(Dialog::ConfirmDelete {
            permanent: true,
            ..
        })
    ));
    let _ = app.update(Message::DialogCancel);
    // The row's menu is the trash's own.
    let e = app.panes[0].active().panel.current().unwrap().clone();
    assert!(matches!(
        crate::menu::Ctx::entry(&e, &files),
        crate::menu::Ctx::Trash
    ));
    // Restore: back where it came from (done off the UI thread, here by hand).
    let _ = app.update(Message::Key(Action::Restore));
    assert!(app.msg().is_none());
    assert_eq!(trashcan::restore(&files.join("a.txt")).unwrap(), from);
    let _ = app.update(Message::Restored(vec!["b: taken".into()]));
    assert_eq!(app.msg(), Some("b: taken"));
    // «Empty trash»: asked about everything found; nothing found — said so.
    let _ = app.update(Message::TrashListed(0, Ok(vec![files.join("x")])));
    assert!(matches!(
        app.dialog,
        Some(Dialog::ConfirmDelete {
            permanent: true,
            ..
        })
    ));
    let _ = app.update(Message::DialogCancel);
    let _ = app.update(Message::TrashListed(0, Ok(Vec::new())));
    assert_eq!(app.msg(), Some(fl!("trash-empty").as_str()));
    // Not in the trash: nothing to restore.
    let (mut app, _tmp) = lister_app(Config::default());
    let _ = app.update(Message::Key(Action::Restore));
    assert_eq!(app.msg(), Some(fl!("why-not-in-trash").as_str()));
}

#[test]
fn the_names_encoding_is_chosen_for_a_zip() {
    let (mut app, tmp) = lister_app(Config::default());
    // Not on a zip: said so.
    let _ = app.update(Message::Key(Action::ArchiveEncoding));
    assert!(app.dialog.is_none());
    assert_eq!(app.msg(), Some(fl!("archive-encoding-no-zip").as_str()));
    // Inside one: every encoding offered, the chosen one kept for it.
    let zip = tmp.path().join("z.zip");
    zip::ZipWriter::new(std::fs::File::create(&zip).unwrap())
        .finish()
        .unwrap();
    let _ = app.load(0, zip.clone(), None);
    let t = app.panes[0].active();
    let (id, generation) = (t.id, t.pending.as_ref().unwrap().0);
    let _ = app.update(Message::Listed {
        tab: id,
        generation,
        path: zip.clone(),
        focus: None,
        result: Ok(Vec::new()),
        space: None,
    });
    let _ = app.update(Message::Key(Action::ArchiveEncoding));
    let Some(Dialog::List { items, cursor, .. }) = &app.dialog else {
        panic!("no list");
    };
    assert_eq!((items.len(), *cursor), (archive::NameEnc::ALL.len(), 0));
    let _ = app.update(Message::ListPick(1));
    assert_eq!(archive::name_encoding(&zip), archive::NameEnc::Cp866);
}

/// What the background read would deliver for the viewer's current file.
fn lister_loaded(app: &mut App, dir: &Path) {
    let l = app.lister.as_ref().unwrap();
    let (id, name) = (l.id, l.name.clone());
    let loaded = lister::Loaded::read(&dir.join(&name), &name);
    let _ = app.update(Message::ListerLoaded(id, Arc::new(loaded)));
}

#[test]
fn regression_a_picture_wakes_the_loop_until_the_renderer_has_it() {
    let (mut app, tmp) = lister_app(Config::default());
    let _ = app.update(Message::Key(Action::View));
    let png = tmp.path().join("p.png");
    image::RgbImage::new(4, 4).save(&png).unwrap();
    let l = app.lister.as_mut().unwrap();
    l.name = "p.png".into();
    let id = l.id;
    let ticking = |app: &App| {
        (app.lister.as_ref().and_then(|l| l.wake_until))
            .is_some_and(|t| std::time::Instant::now() < t)
    };
    lister_loaded(&mut app, tmp.path());
    // The renderers' answer may never come while the main window isn't drawn: woken a while.
    assert!(ticking(&app));
    let _ = app.update(Message::ListerAllocated(id, None));
    assert!(
        ticking(&app),
        "failed: the viewer's renderer loads it alone, still woken"
    );
    let until = app.lister.as_ref().unwrap().wake_until.unwrap();
    assert!(until <= std::time::Instant::now() + lister::WAKE_MAX);
    // Text: nothing to wait for.
    let l = app.lister.as_mut().unwrap();
    l.name = "a.txt".into();
    lister_loaded(&mut app, tmp.path());
    assert!(!ticking(&app));
}

#[test]
fn the_path_line_is_edited_and_goes_there() {
    let (mut app, tmp) = lister_app(Config::default());
    let editing = |app: &App| app.path_edit.as_ref().map(|(s, t)| (*s, t.clone()));
    // A file under the cursor: Shift+F6 renames it, as always.
    let _ = app.update(Message::Key(Action::Rename));
    assert!(app.dialog.is_some() && editing(&app).is_none());
    let _ = app.update(Message::DialogCancel);
    // On `..` with nothing marked: the path (TC).
    let _ = app.update(Message::Key(Action::Home));
    let _ = app.update(Message::Key(Action::Rename));
    let cwd = tmp.path().to_string_lossy().into_owned();
    assert_eq!(editing(&app), Some((0, cwd.clone())));
    let _ = app.update(Message::Escape(app.window_id()));
    assert_eq!(editing(&app), None);
    // A relative dir; a file: its dir with the cursor on it; a click on the other pane's line.
    let _ = app.update(Message::Key(Action::EditPath));
    let _ = app.update(Message::PathEdit("sub".into()));
    let _ = app.update(Message::PathEditGo); // checked off the UI thread, then:
    assert_eq!(editing(&app), None);
    let _ = app.update(Message::PathEditFound(0, tmp.path().join("sub"), None));
    assert_eq!(app.panes[0].active().target(), tmp.path().join("sub"));
    let _ = app.update(Message::PathEditStart(1));
    assert_eq!(app.active, 1);
    let _ = app.update(Message::PathEditCancel); // clicked elsewhere
    assert_eq!(editing(&app), None);
    let a = tmp.path().join("a.txt");
    let _ = app.update(Message::PathEditFound(1, a.clone(), Some("a.txt".into())));
    assert_eq!(app.panes[1].active().target(), tmp.path());
    // What is a file: on disk and inside an archive; a dir and an archive are entered.
    let zip = tmp.path().join("z.zip");
    let mut w = zip::ZipWriter::new(std::fs::File::create(&zip).unwrap());
    w.start_file("in.txt", zip::write::SimpleFileOptions::default())
        .unwrap();
    w.finish().unwrap();
    assert_eq!(file_in_its_dir(&a).as_deref(), Some("a.txt"));
    assert_eq!(
        file_in_its_dir(&zip.join("in.txt")).as_deref(),
        Some("in.txt")
    );
    for enter in [tmp.path().join("sub"), zip.clone(), tmp.path().join("none")] {
        assert_eq!(file_in_its_dir(&enter), None, "{enter:?}");
    }
}

#[test]
fn f3_opens_the_viewer_and_esc_closes_it() {
    let (mut app, tmp) = lister_app(Config::default());
    let _ = app.update(Message::Key(Action::View));
    assert_eq!(app.lister.as_ref().map(|l| l.name.as_str()), Some("a.txt"));
    lister_loaded(&mut app, tmp.path());
    assert_eq!(app.lister.as_ref().unwrap().rows(), 2);
    // Panel keys are the viewer's now: F8 does not ask to delete.
    let _ = app.update(Message::Key(Action::Delete));
    assert!(app.dialog.is_none());
    let _ = app.update(Message::DialogCancel);
    assert!(app.lister.is_none());
}

#[test]
fn f3_opens_a_window_of_its_own_unless_set_otherwise() {
    let (mut app, _tmp) = lister_app(Config::default());
    let _ = app.update(Message::Key(Action::View));
    let window = app.lister.as_ref().and_then(|l| l.window).unwrap();
    assert!(!app.lister.as_ref().unwrap().in_place());
    // N / P and F3 again: the same window
    let _ = app.update(Message::ListerStep(true));
    assert_eq!(app.lister.as_ref().and_then(|l| l.window), Some(window));
    // the window closed by the compositor / its ×: the viewer goes too
    assert!(matches!(
        app.on_close_requested(window),
        Some(Message::ListerClose)
    ));
    let _ = app.update(Message::WindowClosed(window));
    assert!(app.lister.is_none());
    // F3 over Ctrl+Q's viewer: its own window all the same
    let _ = app.update(Message::Key(Action::QuickView));
    let _ = app.update(Message::Key(Action::View));
    assert!(app.lister.as_ref().is_some_and(|l| l.window.is_some()));
    // F3 in the other pane with the window open (from the main window, as F3 there does):
    // N / P walk that pane now
    let file = app.panes[1].active().panel.cwd().join("x");
    let _ = app.open_lister(1, file, "x".into());
    assert_eq!(
        app.lister.as_ref().map(|l| (l.side, l.window.is_some())),
        Some((1, true))
    );
    let (mut app, _tmp) = lister_app(Config {
        lister_window: false,
        ..Config::default()
    });
    let _ = app.update(Message::Key(Action::View));
    assert!(app.lister.as_ref().unwrap().in_place());
}

#[test]
fn regression_a_viewer_window_covers_no_pane() {
    // F3 in a window of its own: the panels stay on screen (their thumbnails still load).
    let (mut app, _tmp) = lister_app(Config::default());
    let _ = app.update(Message::Key(Action::View));
    assert!(app.lister.as_ref().is_some_and(|l| l.window.is_some()));
    assert_eq!(app.viewer_shown(), None);
    let (mut app, _tmp) = lister_app(Config {
        lister_window: false,
        ..Config::default()
    });
    let _ = app.update(Message::Key(Action::View));
    assert_eq!(app.viewer_shown(), Some(None)); // in place: both covered
}

#[test]
fn ctrl_q_follows_the_cursor_and_leaves_keys_to_the_panels() {
    let (mut app, tmp) = lister_app(Config::default());
    let _ = app.update(Message::Key(Action::QuickView));
    let shown = |app: &App| app.lister.as_ref().map(|l| l.name.clone());
    assert_eq!(shown(&app).as_deref(), Some("a.txt"));
    lister_loaded(&mut app, tmp.path());
    assert_eq!(app.lister.as_ref().unwrap().rows(), 2);
    // ↓ moves the panel's cursor, and the view follows it.
    let _ = app.update(Message::Key(Action::Down));
    assert_eq!(shown(&app).as_deref(), Some("b.bin"));
    // A dir: a note, no contents.
    let _ = app.update(Message::Key(Action::Up));
    let _ = app.update(Message::Key(Action::Up));
    let l = app.lister.as_ref().unwrap();
    assert_eq!(l.name, "sub");
    // A dir is counted (TC: its size), and the count stops once the cursor leaves it.
    assert!(matches!(l.loaded.as_deref(), Some(Err(t)) if *t == fl!("quick-counting")));
    let counting = app.quick_stop.clone();
    let _ = app.update(Message::Key(Action::Up)); // `..`
    assert!(counting.load(Ordering::Relaxed));
    let l = app.lister.as_ref().unwrap();
    assert!(matches!(l.loaded.as_deref(), Some(Err(t)) if *t == fl!("quick-none")));
    let _ = app.update(Message::Key(Action::Down));
    // F8 is the panel's: it asks to delete.
    let _ = app.update(Message::Key(Action::Delete));
    assert!(app.dialog.is_some());
    let _ = app.update(Message::DialogCancel);
    assert!(app.lister.is_some()); // Esc closed the dialog, not the view
    let counting = app.quick_stop.clone();
    let _ = app.update(Message::Key(Action::QuickView));
    assert!(app.lister.is_none());
    assert!(counting.load(Ordering::Relaxed), "closed: its count stops");
}

#[test]
fn ctrl_q_inside_an_archive_reads_files_and_notes_dirs() {
    use std::io::Write;
    let tmp = tempfile::tempdir().unwrap();
    let z = tmp.path().join("a.zip");
    let mut w = zip::ZipWriter::new(std::fs::File::create(&z).unwrap());
    let opts = zip::write::SimpleFileOptions::default();
    w.add_directory("d/", opts).unwrap();
    w.start_file("f.txt", opts).unwrap();
    w.write_all(b"inside").unwrap();
    w.start_file("g.bin", opts).unwrap(); // past 32 MB (zeros pack small)
    w.write_all(&vec![0; (shagoff_core::lister::LIMIT + 1) as usize])
        .unwrap();
    w.finish().unwrap();
    let mut app = app_with(Config::default(), State::default());
    let _ = app.load(0, z.clone(), None);
    let t = app.panes[0].active();
    let (id, generation) = (t.id, t.pending.as_ref().unwrap().0);
    let _ = app.update(Message::Listed {
        tab: id,
        generation,
        path: z.clone(),
        focus: Some("d".into()),
        result: Ok(listing::scan(&z, false).unwrap()),
        space: None,
    });
    let _ = app.update(Message::Key(Action::QuickView));
    let l = app.lister.as_ref().unwrap();
    assert_eq!(l.name, "d");
    // Counted from the archive's index, nothing unpacked.
    assert!(matches!(l.loaded.as_deref(), Some(Err(t)) if *t == fl!("quick-counting")));
    let u = archive::usage(&z, Path::new("d")).unwrap();
    assert_eq!((u.files, u.dirs), (0, 1));
    let _ = app.update(Message::Key(Action::Down));
    let l = app.lister.as_ref().unwrap();
    assert_eq!(l.name, "f.txt");
    assert!(l.loaded.is_none(), "being taken out and read, not refused");
    // Too big to unpack whole for a 32 MB look.
    let _ = app.update(Message::Key(Action::Down));
    let l = app.lister.as_ref().unwrap();
    assert_eq!(l.name, "g.bin");
    assert!(matches!(l.loaded.as_deref(), Some(Err(t)) if t.contains("F3")));
    // What the background read does for it.
    let stop = std::sync::atomic::AtomicBool::new(false);
    let out = tmp.path().join("out");
    let f = archive::extract_one(&z, Path::new(""), Path::new("f.txt"), &out, &stop).unwrap();
    assert_eq!(std::fs::read_to_string(f).unwrap(), "inside");
}

#[test]
fn regression_ctrl_q_reloads_a_changed_file_and_shows_a_zip() {
    let (mut app, tmp) = lister_app(Config::default());
    let rescan = |app: &mut App, focus: &str| {
        let _ = app.load(0, tmp.path().into(), Some(focus.into()));
        let t = app.panes[0].active();
        let (id, generation) = (t.id, t.pending.as_ref().unwrap().0);
        let _ = app.update(Message::Listed {
            tab: id,
            generation,
            path: tmp.path().into(),
            focus: Some(focus.into()),
            result: Ok(listing::scan(tmp.path(), false).unwrap()),
            space: None,
        });
    };
    let _ = app.update(Message::Key(Action::QuickView));
    let before = app.lister.as_ref().unwrap().id;
    std::fs::write(tmp.path().join("a.txt"), "changed, and longer\n").unwrap();
    rescan(&mut app, "a.txt");
    assert_ne!(
        app.lister.as_ref().unwrap().id,
        before,
        "same name, new size: read again"
    );
    // A zip on disk is no path inside an archive: shown like a folder, what it holds.
    std::fs::write(tmp.path().join("c.zip"), b"PK").unwrap();
    rescan(&mut app, "c.zip");
    let l = app.lister.as_ref().unwrap();
    assert_eq!(l.name, "c.zip");
    assert!(matches!(l.loaded.as_deref(), Some(Err(t)) if *t == fl!("quick-counting")));
}

#[test]
fn ctrl_b_lists_every_file_below_and_again_leaves() {
    let (mut app, tmp) = lister_app(Config::default());
    std::fs::write(tmp.path().join("sub/deep.txt"), "d").unwrap();
    let tab = app.panes[0].active().id;
    let root = tmp.path().to_path_buf();
    let listed = |app: &mut App, root: PathBuf| {
        let paths = listing::branch(std::slice::from_ref(&root), false, &AtomicBool::new(false));
        let id = app.panes[0].active().branching.unwrap_or(0);
        let _ = app.update(Message::BranchListed {
            tab,
            id,
            root,
            paths,
        });
    };
    let _ = app.update(Message::Key(Action::Branch)); // reads in the background
    listed(&mut app, root.clone());
    let t = app.panes[0].active();
    assert!(t.branch);
    let mut found: Vec<String> = (t.results.as_ref().unwrap().1.iter())
        .map(|p| p.strip_prefix(&root).unwrap().display().to_string())
        .collect();
    found.sort();
    assert_eq!(found, ["a.txt", "b.bin", "sub/deep.txt"]);
    // Ctrl+B again: the dir as usual.
    let _ = app.update(Message::Key(Action::Branch));
    assert!(app.panes[0].active().results.is_none());
    // A read that comes back after the tab went elsewhere — even back here — is dropped.
    let _ = app.update(Message::Key(Action::Branch));
    let id = app.panes[0].active().branching.unwrap();
    let _ = app.load(0, root.join("sub"), None);
    let _ = app.load(0, root.clone(), None);
    let paths = listing::branch(std::slice::from_ref(&root), false, &AtomicBool::new(false));
    let _ = app.update(Message::BranchListed {
        tab,
        id,
        root: root.clone(),
        paths,
    });
    assert!(app.panes[0].active().results.is_none());
    // Leaving the dir stops the read too.
    let _ = app.update(Message::Key(Action::Branch));
    let stop = app.panes[0].active().branch_stop.clone();
    let _ = app.load(0, root.join("sub"), None);
    assert!(stop.load(Ordering::Relaxed));
}

#[test]
fn esc_stops_the_second_step_of_a_branch_too() {
    let (mut app, tmp) = lister_app(Config::default());
    let root = tmp.path().to_path_buf();
    let tab = app.panes[0].active().id;
    let branch = |app: &mut App| {
        let _ = app.update(Message::Key(Action::Branch));
        let t = app.panes[0].active();
        let id = t.branching.unwrap();
        let paths = listing::branch(std::slice::from_ref(&root), false, &AtomicBool::new(false));
        let _ = app.update(Message::BranchListed {
            tab,
            id,
            root: root.clone(),
            paths,
        });
    };
    branch(&mut app);
    // Found; the rows (one lstat each) are being read: still under way, Esc can stop it.
    let t = app.panes[0].active();
    assert!(t.branching.is_some() && t.results.is_some());
    let stop = t.branch_stop.clone();
    let _ = app.update(Message::DialogCancel); // Esc
    assert!(stop.load(Ordering::Relaxed));
    let t = app.panes[0].active();
    assert_eq!(t.branching, None);
    assert!(t.results.is_none()); // back to the dir as usual
    assert_eq!(t.pending.as_ref().map(|p| p.1.clone()), Some(root.clone()));
    // Read to the end: no longer under way, the status cleared.
    branch(&mut app);
    let t = app.panes[0].active();
    let generation = t.pending.as_ref().unwrap().0;
    let (_, paths) = t.results.clone().unwrap();
    let rows = read_listing(&root, false, Some(&paths), &AtomicBool::new(false));
    let _ = app.update(Message::Listed {
        tab,
        generation,
        path: root.clone(),
        focus: None,
        result: rows,
        space: None,
    });
    let t = app.panes[0].active();
    assert_eq!(t.branching, None);
    assert!(t.results.is_some());
    assert!(app.status.is_none());
}

#[test]
fn esc_stops_a_branch_read() {
    let (mut app, tmp) = lister_app(Config::default());
    let tab = app.panes[0].active().id;
    let _ = app.update(Message::Key(Action::Branch));
    let (id, stop) = {
        let t = app.panes[0].active();
        (t.branching.unwrap(), t.branch_stop.clone())
    };
    let _ = app.update(Message::DialogCancel); // Esc
    assert!(stop.load(Ordering::Relaxed));
    assert_eq!(app.panes[0].active().branching, None);
    assert_eq!(app.status.as_ref().map(|s| s.kind), Some(StatusKind::Info));
    // What the worker sends back afterwards (stopped, or done just before) is dropped.
    let _ = app.update(Message::BranchListed {
        tab,
        id,
        root: tmp.path().to_path_buf(),
        paths: Some(Vec::new()),
    });
    assert!(app.panes[0].active().results.is_none());
    // Another tab's read: Esc stops it too; closing a tab stops its own.
    let _ = app.update(Message::Key(Action::Branch));
    let first = app.panes[0].active().branch_stop.clone();
    let _ = app.update(Message::Key(Action::NewTab));
    let _ = app.update(Message::DialogCancel);
    assert!(first.load(Ordering::Relaxed));
    let _ = app.update(Message::Key(Action::Branch));
    let second = app.panes[0].active().branch_stop.clone();
    let _ = app.update(Message::Key(Action::CloseTab));
    assert!(second.load(Ordering::Relaxed));
}

#[test]
fn esc_during_a_drag_ends_it_and_nothing_else() {
    let mut app = app_with(Config::default(), State::default());
    app.panes[0].active_mut().panel.set_filter(Some("x".into()));
    let _ = app.update(Message::DragStart(Some(Arc::new(vec![PathBuf::from(
        "/a",
    )]))));
    let _ = app.update(Message::Escape(app.window_id()));
    assert!(!app.dragging && app.right_drag.is_none());
    assert!(app.panes[0].active().panel.filter().is_some()); // the drag's Esc, not the filter's
    let _ = app.update(Message::Escape(app.window_id()));
    assert!(app.panes[0].active().panel.filter().is_none());
    // A drag that ended (dropped) leaves Esc to the panels again.
    let _ = app.update(Message::DragStart(None));
    let _ = app.update(Message::DragEnd);
    assert!(!app.dragging);
}

#[test]
fn f3_without_the_builtin_viewer_runs_the_program() {
    let config = Config {
        internal_viewer: false,
        viewer: vec!["/nonexistent/viewer".into()], // never launch anything from a test
        ..Config::default()
    };
    let (mut app, _tmp) = lister_app(config);
    let _ = app.update(Message::Key(Action::View));
    assert!(app.lister.is_none());
    assert_eq!(app.status.as_ref().map(|s| s.kind), Some(StatusKind::Error));
}

#[test]
fn regression_viewer_n_in_an_archive_cancelled_keeps_the_cursor_on_the_shown_file() {
    use std::io::Write;
    let tmp = tempfile::tempdir().unwrap();
    let a = tmp.path().join("a.zip");
    let mut z = zip::ZipWriter::new(std::fs::File::create(&a).unwrap());
    for n in ["a.txt", "b.txt"] {
        z.start_file(n, zip::write::SimpleFileOptions::default())
            .unwrap();
        z.write_all(b"x").unwrap();
    }
    z.finish().unwrap();
    let mut app = app_with(Config::default(), State::default());
    listed_at(&mut app, 0, &a);
    let _ = app.update(Message::Key(Action::Down)); // a.txt
    let cur = |app: &App| app.panes[0].active().panel.current().unwrap().name.clone();
    // F3: extracted, then shown.
    let _ = app.update(Message::Key(Action::View));
    let file = app
        .job
        .as_ref()
        .and_then(|j| j.open.as_ref())
        .unwrap()
        .1
        .clone();
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, "x").unwrap();
    let done = Report {
        cancelled: false,
        completed: vec![a.join("a.txt")],
    };
    let _ = app.update(Message::Op(jobs::Event::Finished(Arc::new(done))));
    assert_eq!(app.lister.as_ref().unwrap().name, "a.txt");
    // N: b.txt is being extracted, the cursor already on it; Esc stops that.
    let _ = app.update(Message::ListerKey(ListerKey::Next));
    assert_eq!(cur(&app), "b.txt");
    let stopped = Report {
        cancelled: true,
        completed: Vec::new(),
    };
    let _ = app.update(Message::Op(jobs::Event::Finished(Arc::new(stopped))));
    assert_eq!(app.lister.as_ref().unwrap().name, "a.txt");
    assert_eq!(cur(&app), "a.txt"); // with the file on screen
}

#[test]
fn viewer_n_and_p_walk_files_skipping_dirs() {
    let (mut app, tmp) = lister_app(Config::default());
    let _ = app.update(Message::Key(Action::View));
    let _ = app.update(Message::ListerKey(ListerKey::Next));
    assert_eq!(app.lister.as_ref().unwrap().name, "b.bin");
    let cur = |app: &App| app.panes[0].active().panel.current().unwrap().name.clone();
    assert_eq!(cur(&app), "b.bin");
    lister_loaded(&mut app, tmp.path());
    assert_eq!(app.lister.as_ref().unwrap().mode, Mode::Hex); // NUL inside
    let _ = app.update(Message::ListerKey(ListerKey::Next)); // last file: stays
    assert_eq!(cur(&app), "b.bin");
    let _ = app.update(Message::ListerKey(ListerKey::Prev));
    let _ = app.update(Message::ListerKey(ListerKey::Prev)); // `sub` and `..` skipped
    assert_eq!(cur(&app), "a.txt");
    assert_eq!(app.lister.as_ref().unwrap().name, "a.txt");
}

#[test]
fn regression_mode_switch_resets_offset() {
    let (mut app, tmp) = lister_app(Config::default());
    let _ = app.update(Message::Key(Action::View));
    lister_loaded(&mut app, tmp.path());
    // Scrolled far down in another mode; a short text fits, so iced reports no scroll.
    let _ = app.update(Message::ListerScrolled(30.0, 4700.0, 400.0));
    let _ = app.update(Message::ListerMode(Mode::Hex));
    assert_eq!(app.lister.as_ref().unwrap().offset, (0.0, 0.0));
    let _ = app.update(Message::Key(Action::End));
    assert_eq!(app.lister.as_ref().unwrap().offset, (0.0, 0.0)); // 1 hex row fits
}

#[test]
fn settings_field_keys_do_not_reach_the_viewer() {
    let (mut app, _tmp) = lister_app(Config::default());
    let _ = app.update(Message::Key(Action::View));
    let _ = app.update(Message::Key(Action::Settings));
    assert!(app.drawer.is_some());
    let _ = app.update(Message::FieldKey(Action::Mkdir)); // F7 typed in a settings field
    assert!(!app.lister.as_ref().unwrap().searching);
}

#[test]
fn viewer_next_onto_a_broken_link_keeps_cursor_and_file() {
    let (mut app, tmp) = lister_app(Config::default());
    std::os::unix::fs::symlink("nowhere", tmp.path().join("c_dangling")).unwrap();
    let _ = app.load(0, tmp.path().into(), None);
    let t = app.panes[0].active();
    let (id, generation) = (t.id, t.pending.as_ref().unwrap().0);
    let _ = app.update(Message::Listed {
        tab: id,
        generation,
        path: tmp.path().into(),
        focus: Some("b.bin".into()),
        result: Ok(listing::scan(tmp.path(), false).unwrap()),
        space: None,
    });
    let _ = app.update(Message::Key(Action::View));
    let _ = app.update(Message::ListerKey(ListerKey::Next));
    assert_eq!(app.lister.as_ref().unwrap().name, "b.bin");
    let cur = app.panes[0].active().panel.current().unwrap().name.clone();
    assert_eq!(cur, "b.bin");
    assert_eq!(app.status.as_ref().map(|s| s.kind), Some(StatusKind::Error));
}

#[test]
fn regression_viewer_arrows_scroll_sideways() {
    // ← / → are Brief keys in the main table now; the viewer still takes them.
    let (mut app, tmp) = lister_app(Config::default());
    let _ = app.update(Message::Key(Action::View));
    lister_loaded(&mut app, tmp.path());
    let _ = app.update(Message::ListerKey(ListerKey::Mode(Mode::Hex)));
    let _ = app.update(Message::Key(Action::Right));
    assert!(app.lister.as_ref().unwrap().offset.0 > 0.0);
    let _ = app.update(Message::Key(Action::Left));
    assert_eq!(app.lister.as_ref().unwrap().offset.0, 0.0);
    assert_eq!(app.panes[0].active().panel.cursor(), 2); // still on a.txt
}

#[test]
fn viewer_modes_and_search() {
    let (mut app, tmp) = lister_app(Config::default());
    let _ = app.update(Message::Key(Action::View));
    lister_loaded(&mut app, tmp.path());
    let _ = app.update(Message::ListerKey(ListerKey::Mode(Mode::Hex)));
    assert_eq!(app.lister.as_ref().unwrap().mode, Mode::Hex);
    let _ = app.update(Message::ListerKey(ListerKey::Mode(Mode::Text)));
    // F3 with nothing to find opens the field; Esc closes the field, not the viewer.
    let _ = app.update(Message::Key(Action::View));
    assert!(app.lister.as_ref().unwrap().searching);
    let _ = app.update(Message::DialogCancel);
    assert!(app.lister.as_ref().is_some_and(|l| !l.searching));
    let _ = app.update(Message::Key(Action::Mkdir)); // F7
    let _ = app.update(Message::ListerQuery("FOO".into()));
    let _ = app.update(Message::ListerFind);
    let l = app.lister.as_ref().unwrap();
    assert_eq!((l.hit, l.searching), (Some(1), false));
    let _ = app.update(Message::Key(Action::View)); // F3: no further match
    assert_eq!(
        app.msg(),
        Some(fl!("lister-not-found", query = "FOO").as_str())
    );
}

/// Row height of the default skin, for tests that size the list in rows.
const ROW: f32 = 20.0;

/// Pane 0 listing 20 files, viewport `height` px tall, scrolled to the top.
fn tall_list(height: f32) -> App {
    let mut app = app_with(Config::default(), State::default());
    let t = app.panes[0].active();
    let (id, generation) = (t.id, t.pending.as_ref().unwrap().0);
    // 21-char names: Brief columns of ~195 px, two on the 500 px list.
    let entries = (0..20)
        .map(|i| entry(&format!("file-with-long-name{i:02}")))
        .collect();
    let _ = app.update(Message::Listed {
        tab: id,
        generation,
        path: std::env::temp_dir(),
        focus: None,
        result: Ok(entries),
        space: None,
    });
    let _ = app.update(Message::Resized(0, Size::new(FALLBACK_LIST_W, height)));
    app
}

#[test]
fn default_skin_row_matches_the_tests() {
    assert_eq!(config::Skin::default().row_h(), ROW);
}

#[test]
fn switching_skin_keeps_the_cursor_on_screen() {
    let mut app = tall_list(4.5 * ROW);
    let _ = app.update(Message::Key(Action::End)); // row 20
    let _ = app.update(Message::Key(Action::Settings));
    let _ = app.update(Message::Setting(crate::drawer::Setting::Skin(1)));
    assert_eq!(app.config.skin, config::Skin::Modern);
    let (t, row_h) = (app.panes[0].active(), app.row_h());
    let bottom = 21.0 * row_h;
    assert!(
        t.offset <= 20.0 * row_h && t.offset + t.height >= bottom,
        "offset {}",
        t.offset
    );
}

#[test]
fn thumbs_view_moves_by_cells_and_rows() {
    // ".." + 20 files; 500 px = 4 cells a row
    let mut app = tall_list(4.5 * ROW);
    let cur = |app: &App| app.panes[0].active().panel.cursor();
    let _ = app.update(Message::Key(Action::ViewBrief));
    let _ = app.update(Message::Key(Action::ViewThumbs));
    let t = app.panes[0].active();
    assert!(t.thumbs && !t.brief);
    let _ = app.update(Message::Key(Action::Right));
    assert_eq!(cur(&app), 1);
    let _ = app.update(Message::Key(Action::Down));
    assert_eq!(cur(&app), 5);
    let _ = app.update(Message::Key(Action::PageDown)); // one row on screen
    assert_eq!(cur(&app), 9);
    let _ = app.update(Message::Key(Action::Up));
    let _ = app.update(Message::Key(Action::Left));
    assert_eq!(cur(&app), 4);
    let _ = app.update(Message::Key(Action::End));
    let t = app.panes[0].active();
    let cell_h = crate::thumbs::cell_h(app.row_h());
    assert!(t.offset + t.height >= 6.0 * cell_h); // the last row (6th) on screen
    let _ = app.update(Message::Key(Action::ViewFull));
    assert!(!app.panes[0].active().thumbs);
}

#[test]
fn brief_view_moves_by_columns() {
    // ".." + 20 files; 4.5 rows high = 4 rows, 500 px = 2 columns on screen
    let mut app = tall_list(4.5 * ROW);
    let cur = |app: &App| app.panes[0].active().panel.cursor();
    let _ = app.update(Message::Key(Action::Right));
    assert_eq!(cur(&app), 0); // Full view: ← / → do nothing
    let _ = app.update(Message::Key(Action::ViewBrief));
    assert!(app.panes[0].active().brief);
    let _ = app.update(Message::Key(Action::Right));
    assert_eq!(cur(&app), 4);
    let _ = app.update(Message::Key(Action::PageDown));
    assert_eq!(cur(&app), 12);
    let _ = app.update(Message::Key(Action::Left));
    assert_eq!(cur(&app), 8);
    let _ = app.update(Message::Key(Action::End));
    let _ = app.update(Message::Key(Action::Right));
    assert_eq!(cur(&app), 20);
    assert_eq!(app.panes[0].active().col, 4); // 6 columns, the last two on screen
    let _ = app.update(Message::BriefWheel(
        0,
        mouse::ScrollDelta::Lines { x: 0.0, y: 1.0 },
    ));
    assert_eq!(app.panes[0].active().col, 3);
    let _ = app.update(Message::Key(Action::Home));
    assert_eq!(app.panes[0].active().col, 0);
    let _ = app.update(Message::Key(Action::ViewFull));
    assert!(!app.panes[0].active().brief);
}

#[test]
fn brief_columns_follow_a_resize_and_touchpad_steps_add_up() {
    let mut app = tall_list(4.5 * ROW); // 4 rows
    let _ = app.update(Message::Key(Action::ViewBrief));
    let _ = app.update(Message::Key(Action::End)); // entry 20, column 5
    assert_eq!(app.panes[0].active().col, 4);
    // taller: 10 rows → 3 columns, both fit; the old first column would hide the cursor
    let _ = app.update(Message::Resized(0, Size::new(FALLBACK_LIST_W, 10.5 * ROW)));
    assert_eq!(app.panes[0].active().col, 1);
    let px = |y| Message::BriefWheel(0, mouse::ScrollDelta::Pixels { x: 0.0, y });
    let _ = app.update(px(5.0));
    let _ = app.update(px(5.0));
    assert_eq!(app.panes[0].active().col, 1); // 10 px: less than a row
    let _ = app.update(px(15.0));
    assert_eq!(app.panes[0].active().col, 0);
}

#[test]
fn regression_click_on_half_visible_row_scrolls_it_in() {
    let mut app = tall_list(4.5 * ROW); // rows 0..4 full, row 4 cut in half
    let _ = app.update(Message::Click(0, 4));
    let t = app.panes[0].active();
    assert!(
        t.offset + t.height >= 5.0 * app.row_h(),
        "offset {}",
        t.offset
    );
}

#[test]
fn regression_shrinking_window_keeps_cursor_visible() {
    let mut app = tall_list(400.0);
    let _ = app.update(Message::Click(0, 10));
    let _ = app.update(Message::Resized(0, Size::new(FALLBACK_LIST_W, 100.0))); // window got smaller
    let t = app.panes[0].active();
    assert!(
        t.offset + t.height >= 11.0 * app.row_h(),
        "offset {}",
        t.offset
    );
}

#[test]
fn wheel_scroll_never_snaps_back_even_when_it_reports_a_smaller_height() {
    let mut app = tall_list(400.0);
    let _ = app.update(Message::Click(0, 10));
    let _ = app.update(Message::Scrolled(0, 0.0, 100.0));
    assert_eq!(app.panes[0].active().offset, 0.0);
}

#[test]
fn resize_applies_to_every_tab_of_the_pane() {
    // One scrollable per pane: hidden tabs must not keep a stale height.
    let mut app = tall_list(400.0);
    let _ = app.update(Message::Key(Action::NewTab));
    let _ = app.update(Message::Resized(0, Size::new(FALLBACK_LIST_W, 150.0)));
    assert!(app.panes[0].items().iter().all(|t| t.height == 150.0));
}

#[test]
fn toolbar_shift_drag_moves_a_button_without_running_it() {
    let mut app = app_with(Config::default(), State::default());
    let bar = app.config.toolbar.clone();
    let _ = app.update(Message::Modifiers(Modifiers::SHIFT));
    let _ = app.update(Message::ToolHover(Some(1)));
    let _ = app.update(Message::ToolGrab);
    let _ = app.update(Message::ToolHover(Some(2)));
    let _ = app.update(Message::Tool(2)); // the release over it: not run
    let _ = app.update(Message::ToolRelease);
    assert!(!app.panes[0].active().brief, "cm_SrcShort not run");
    assert_eq!(app.config.toolbar[2], bar[1]);
    assert_eq!(app.config.toolbar[1], bar[2]);
    // Released: hovering moves nothing.
    let _ = app.update(Message::ToolHover(Some(0)));
    assert_eq!(app.config.toolbar[2], bar[1]);
    // Shift let go mid-drag (the release may never come, e.g. focus left): dropped there.
    let _ = app.update(Message::ToolGrab);
    let _ = app.update(Message::Modifiers(Modifiers::empty()));
    let _ = app.update(Message::ToolHover(Some(1)));
    assert_eq!(app.config.toolbar[0], bar[0]);
    // Under a dialog nothing is grabbed.
    let _ = app.update(Message::Modifiers(Modifiers::SHIFT));
    let _ = app.update(Message::Key(Action::ConfigureToolbar));
    let _ = app.update(Message::ToolGrab);
    assert_eq!(app.tool_drag.grab, None);
}

#[test]
fn toolbar_buttons_run_and_the_bar_is_edited() {
    use crate::toolbar::ToolMsg;
    let mut app = app_with(Config::default(), State::default());
    let brief = (app.config.toolbar.iter())
        .position(|b| b.cmd == "cm_SrcShort")
        .unwrap();
    let _ = app.update(Message::Tool(brief));
    assert!(app.panes[0].active().brief);
    let _ = app.update(Message::Key(Action::ConfigureToolbar));
    let _ = app.update(Message::ToolEdit(ToolMsg::Select(0)));
    let _ = app.update(Message::ToolEdit(ToolMsg::Delete));
    let _ = app.update(Message::DialogSubmit);
    assert!(app.dialog.is_none());
    assert_eq!(app.config.toolbar, crate::toolbar::default_bar()[1..]);
}

#[test]
fn a_tab_dropped_on_the_other_pane_moves_there_or_is_copied_with_ctrl() {
    let mut app = app_with(Config::default(), State::default());
    let _ = app.update(Message::Key(Action::NewTab)); // pane 0: two tabs
    for t in app.panes[0].items_mut() {
        t.width = 500.0; // the right pane starts past 508
    }
    let count = |app: &App| (app.panes[0].items().len(), app.panes[1].items().len());
    // Released over its own pane: nothing.
    let _ = app.update(Message::TabPress(0, 1));
    let _ = app.update(Message::TabDragAt(app.window_id(), 10.0));
    let _ = app.update(Message::ColDragEnd);
    assert_eq!(count(&app), (2, 1));
    // Over the other pane: moved there, that pane active.
    let _ = app.update(Message::TabPress(0, 1));
    let _ = app.update(Message::TabDragAt(app.window_id(), 700.0));
    let _ = app.update(Message::ColDragEnd);
    assert_eq!((count(&app), app.active), ((1, 2), 1));
    // A pointer that only passes over the other pane without a tab held: nothing.
    let _ = app.update(Message::TabDragAt(app.window_id(), 10.0));
    let _ = app.update(Message::ColDragEnd);
    assert_eq!(count(&app), (1, 2));
    // With Ctrl: copied; the last tab of a pane can be copied too.
    app.mods = Modifiers::CTRL;
    let _ = app.update(Message::TabPress(0, 0));
    let _ = app.update(Message::TabDragAt(app.window_id(), 700.0));
    let _ = app.update(Message::ColDragEnd);
    assert_eq!(count(&app), (1, 3));
    // The split between the panes, for both skins: the middle of the gap.
    assert_eq!(
        crate::view::pane_split(crate::config::Skin::Classic, 500.0, false),
        502.0
    );
    // the classic frame: its 1 px inset on each side of the left pane
    assert_eq!(
        crate::view::pane_split(crate::config::Skin::Classic, 500.0, true),
        504.0
    );
    assert_eq!(
        crate::view::pane_split(crate::config::Skin::Modern, 500.0, false),
        524.0
    );
    // A popup's coordinates are not the main window's.
    let _ = app.update(Message::TabPress(0, 0));
    let popup = cosmic::iced::window::Id::unique();
    let _ = app.update(Message::TabDragAt(popup, 700.0));
    let _ = app.update(Message::ColDragEnd);
    assert_eq!(count(&app), (1, 3));
}

#[test]
fn regression_a_viewer_window_does_not_stop_a_tab_drop() {
    let (mut app, _tmp) = lister_app(Config::default());
    app.active = 0;
    let _ = app.update(Message::Key(Action::NewTab)); // pane 0: two tabs
    let _ = app.update(Message::Key(Action::View)); // F3 in its own window
    assert!(app.lister.as_ref().is_some_and(|l| l.window.is_some()));
    for t in app.panes[0].items_mut() {
        t.width = 500.0;
    }
    let before = app.panes[1].items().len();
    let _ = app.update(Message::TabPress(0, 1));
    let _ = app.update(Message::TabDragAt(app.window_id(), 700.0));
    let _ = app.update(Message::ColDragEnd);
    assert_eq!(app.panes[1].items().len(), before + 1);
}

#[test]
fn tabs_move_by_drag_and_keys() {
    let mut app = app_with(Config::default(), State::default());
    for _ in 0..2 {
        let _ = app.update(Message::Key(Action::NewTab));
    }
    for (i, t) in app.panes[0].items_mut().iter_mut().enumerate() {
        t.name = Some(i.to_string());
    }
    let names = |app: &App| -> String {
        (app.panes[0].items().iter())
            .map(|t| t.name.clone().unwrap())
            .collect()
    };
    let _ = app.update(Message::TabEnter(0, 1)); // no drag: nothing moves
    assert_eq!(names(&app), "012");
    let _ = app.update(Message::TabPress(0, 0));
    let _ = app.update(Message::TabEnter(0, 1));
    let _ = app.update(Message::TabEnter(0, 2));
    let _ = app.update(Message::ColDragEnd);
    let _ = app.update(Message::TabEnter(0, 0)); // released
    assert_eq!(
        (names(&app), app.panes[0].active_index()),
        ("120".into(), 2)
    );
    let _ = app.update(Message::Key(Action::MoveTabRight)); // last: stays
    let _ = app.update(Message::Key(Action::MoveTabLeft));
    assert_eq!(
        (names(&app), app.panes[0].active_index()),
        ("102".into(), 1)
    );
}

#[test]
fn wheel_scroll_does_not_drag_the_view_back_to_the_cursor() {
    let mut app = tall_list(100.0);
    let _ = app.update(Message::Scrolled(0, 200.0, 100.0)); // user scrolled away
    assert_eq!(app.panes[0].active().offset, 200.0);
}

/// Pane 0 shows `names` (files) in temp_dir.
#[test]
fn ctrl_arrows_open_in_that_panel_and_shift_f10_is_the_row_menu() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir(tmp.path().join("sub")).unwrap();
    std::fs::write(tmp.path().join("f.txt"), "x").unwrap();
    let mut app = app_with(Config::default(), State::default());
    listed_at(&mut app, 0, tmp.path());
    let _ = app.update(Message::Key(Action::Down)); // "sub"
    let _ = app.update(Message::Key(Action::OpenRight));
    let pending = |app: &App, s: usize| app.panes[s].active().pending.clone().map(|(_, p)| p);
    assert_eq!(pending(&app, 1), Some(tmp.path().join("sub")));
    let _ = app.update(Message::Key(Action::Down)); // "f.txt": this panel's dir
    let _ = app.update(Message::Key(Action::OpenRight));
    assert_eq!(pending(&app, 1).as_deref(), Some(tmp.path()));
    // On a file, Ctrl+← in its own (left) panel: nothing read again.
    let before = pending(&app, 0);
    let _ = app.update(Message::Key(Action::OpenLeft));
    assert_eq!(pending(&app, 0), before);
    // Shift+F10 on the file: its menu as a list; a pick does what the item does.
    let _ = app.update(Message::Key(Action::ContextMenu));
    let Some(Dialog::List {
        kind: ListKind::Context,
        items,
        ..
    }) = &app.dialog
    else {
        panic!("no menu");
    };
    let mkdir_free = !items
        .iter()
        .any(|it| it.kind == Item::Act(crate::menu::MenuAct::Key(Action::Mkdir)));
    let props = (items.iter())
        .position(|it| it.kind == Item::Act(crate::menu::MenuAct::Key(Action::Properties)))
        .unwrap();
    assert!(mkdir_free && items.iter().any(|it| it.kind == Item::Sep));
    let _ = app.update(Message::ListPick(props));
    assert!(matches!(app.dialog, Some(Dialog::Props(_))));
}

#[test]
fn regression_dialog_arrows_walk_its_buttons_not_the_panel() {
    let mut app = files_app(&["a.txt", "b.txt"]);
    let _ = app.update(Message::Key(Action::Down));
    let _ = app.update(Message::Key(Action::Delete));
    assert!(matches!(app.dialog, Some(Dialog::ConfirmDelete { .. })));
    let cursor = app.panes[0].active().panel.cursor();
    for a in [
        Action::Down,
        Action::Right,
        Action::Up,
        Action::Left,
        Action::SwitchPane,
    ] {
        let _ = app.update(Message::Key(a)); // focus moves (a widget operation)
    }
    assert_eq!(app.panes[0].active().panel.cursor(), cursor);
    assert_eq!(app.active, 0);
    assert!(matches!(app.dialog, Some(Dialog::ConfirmDelete { .. })));
}

#[test]
fn alt_shift_f9_tests_the_archives_and_says_what_is_wrong() {
    let mut app = files_app(&["notes.txt"]);
    let _ = app.update(Message::Key(Action::Down));
    let _ = app.update(Message::Key(Action::TestArchives)); // no archive here
    assert_eq!(app.msg(), Some(fl!("test-no-archives").as_str()));
    let ok = (PathBuf::from("/x/a.zip"), Ok(3));
    let _ = app.update(Message::Tested(vec![ok.clone()]));
    assert!(app.msg().is_some_and(|m| m.contains('3')));
    let bad = (PathBuf::from("/x/b.7z"), Err("checksum".to_string()));
    let _ = app.update(Message::Tested(vec![ok, bad]));
    assert!(app.msg().is_some_and(|m| m.contains("b.7z: checksum")));
}

#[test]
fn ctrl_f10_f11_f12_show_all_programs_or_by_mask() {
    let mut app = files_app(&["a.rs", "b.txt"]);
    let shown = |app: &App| app.panes[0].active().panel.entries().len();
    let _ = app.update(Message::Key(Action::ShowPrograms)); // none has an x bit
    assert_eq!(shown(&app), 1, "only ..");
    let _ = app.update(Message::Key(Action::ShowMask));
    let Some(Dialog::Mask {
        op: MaskOp::Show, ..
    }) = &app.dialog
    else {
        panic!("no mask dialog");
    };
    let _ = app.update(Message::DialogInput("*.rs".into()));
    let _ = app.update(Message::DialogSubmit);
    assert_eq!(
        app.panes[0].active().panel.show(),
        &Show::Mask("*.rs".into())
    );
    assert_eq!(shown(&app), 2);
    let _ = app.update(Message::Key(Action::ShowMask)); // offers the mask in force
    assert!(matches!(&app.dialog, Some(Dialog::Mask { input, .. }) if input == "*.rs"));
    let _ = app.update(Message::DialogInput("*.*".into()));
    let _ = app.update(Message::DialogSubmit);
    assert_eq!(app.panes[0].active().panel.show(), &Show::All);
    let _ = app.update(Message::Key(Action::ShowPrograms));
    let _ = app.update(Message::Key(Action::ShowAll));
    assert_eq!(shown(&app), 3);
}

#[test]
fn checksums_dialog_keeps_the_algo_and_verify_says_what_is_wrong() {
    let mut app = files_app(&["notes.txt"]);
    let _ = app.update(Message::Key(Action::Down));
    let _ = app.update(Message::Key(Action::ChecksumVerify)); // no sum file
    assert_eq!(app.msg(), Some(fl!("checksum-no-files").as_str()));
    let _ = app.update(Message::Key(Action::ChecksumCreate));
    assert!(matches!(
        app.dialog,
        Some(Dialog::Checksum {
            algo: Algo::Sha256,
            ..
        })
    ));
    let _ = app.update(Message::SumAlgo(Algo::Md5));
    assert_eq!(app.config.checksum_algo, "md5");
    assert!(matches!(
        app.dialog,
        Some(Dialog::Checksum {
            algo: Algo::Md5,
            ..
        })
    ));
    let _ = app.update(Message::Key(Action::Enter)); // no text field: Enter creates
    assert!(app.dialog.is_none());
    let sum = PathBuf::from("/x/SHA256SUMS");
    let good = vec![(PathBuf::from("a"), Check::Ok)];
    let _ = app.update(Message::Verified(vec![(sum.clone(), Ok(good.clone()))]));
    assert_eq!(app.msg(), Some(fl!("checksum-ok", n = 1).as_str()));
    let bad = vec![(PathBuf::from("b"), Check::Bad)];
    let _ = app.update(Message::Verified(vec![(sum, Ok([good, bad].concat()))]));
    assert!(
        app.msg()
            .is_some_and(|m| m.contains(&format!("b: {}", fl!("checksum-bad"))))
    );
}

#[test]
fn alt_f10_tree_opens_the_way_to_here_and_goes_where_chosen() {
    use shagoff_core::tree::{chain, read};
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir(tmp.path().join("inner")).unwrap();
    let mut app = app_with(Config::default(), State::default());
    listed_at(&mut app, 0, tmp.path());
    let _ = app.update(Message::Key(Action::TreeDialog));
    let id = match &app.dialog {
        Some(Dialog::Tree { id, .. }) => *id,
        _ => panic!("no tree"),
    };
    // What its background read brings: the way down from `/`, the cursor on the dir here.
    let got = read(chain(Path::new("/"), tmp.path()), false, Some(tmp.path()));
    let _ = app.update(Message::TreeFilled(id, got, Some(tmp.path().into())));
    let cur = |app: &App| match &app.dialog {
        Some(Dialog::Tree { tree, .. }) => tree.current().unwrap().path.clone(),
        _ => panic!("no tree"),
    };
    assert_eq!(cur(&app), tmp.path());
    // → reads it (here: what that read brings), → again steps in.
    let _ = app.update(Message::Key(Action::Right));
    let inner = vec![(tmp.path().to_path_buf(), vec![OsString::from("inner")])];
    let _ = app.update(Message::TreeFilled(id + 1, inner.clone(), None)); // another dialog's
    let _ = app.update(Message::Key(Action::Right));
    assert_eq!(cur(&app), tmp.path()); // not read yet: nothing to step into
    let _ = app.update(Message::TreeFilled(id, inner, None));
    let _ = app.update(Message::Key(Action::Right));
    assert_eq!(cur(&app), tmp.path().join("inner"));
    let _ = app.update(Message::Key(Action::Left)); // closed: to its parent
    assert_eq!(cur(&app), tmp.path());
    let _ = app.update(Message::Key(Action::Left)); // open: closed
    let _ = app.update(Message::Key(Action::Left)); // to its parent
    assert_eq!(cur(&app), tmp.path().parent().unwrap());
    let _ = app.update(Message::Key(Action::Enter));
    assert!(app.dialog.is_none());
    let pending = app.panes[0].active().pending.clone().map(|(_, p)| p);
    assert_eq!(pending.as_deref(), tmp.path().parent());
}

#[test]
fn ctrl_f8_tree_in_the_panel_leads_the_other_one() {
    use shagoff_core::tree::{chain, read};
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir(tmp.path().join("a")).unwrap();
    std::fs::create_dir(tmp.path().join("b")).unwrap();
    let mut app = app_with(Config::default(), State::default());
    listed_at(&mut app, 0, tmp.path());
    let _ = app.update(Message::Key(Action::TreePanel));
    let id = app.panes[0].active().tree.as_ref().map(|pt| pt.id).unwrap();
    let mut got = read(chain(Path::new("/"), tmp.path()), false, Some(tmp.path()));
    got.extend(read(vec![tmp.path().into()], false, None)); // and its own subdirs
    let _ = app.update(Message::TreeFilled(id, got, Some(tmp.path().into())));
    let cur = |app: &App| {
        let pt = app.panes[0].active().tree.as_ref().unwrap();
        pt.tree.current().unwrap().path.clone()
    };
    assert_eq!(cur(&app), tmp.path());
    // ↓: onto "a"; the other panel goes there.
    let _ = app.update(Message::Key(Action::Down));
    assert_eq!(cur(&app), tmp.path().join("a"));
    let pending = |app: &App| app.panes[1].active().pending.clone().map(|(_, p)| p);
    assert_eq!(pending(&app), Some(tmp.path().join("a")));
    // F8 under the tree: no files there to delete.
    let _ = app.update(Message::Key(Action::Delete));
    assert!(app.dialog.is_none());
    // Not about the files: as usual (Ctrl+T, then back).
    let _ = app.update(Message::Key(Action::NewTab));
    assert_eq!(app.panes[0].items().len(), 2);
    assert!(app.panes[0].active().tree.is_none()); // the new tab: files
    let _ = app.update(Message::Key(Action::CloseTab));
    assert!(app.panes[0].active().tree.is_some());
    // The tree's tab sent elsewhere (Alt+F10…): its tree opened again there.
    let before = app.panes[0].active().tree.as_ref().unwrap().id;
    listed_at(&mut app, 0, &tmp.path().join("b"));
    assert_ne!(app.panes[0].active().tree.as_ref().unwrap().id, before);
    // Enter: on to the files.
    let _ = app.update(Message::Key(Action::Enter));
    assert_eq!(app.active, 1);
    app.active = 0;
    let _ = app.update(Message::Key(Action::TreePanel)); // off
    assert!(app.panes[0].active().tree.is_none());
}

#[test]
fn ctrl_l_says_the_occupied_space() {
    let mut app = app_with(Config::default(), State::default());
    let u = shagoff_core::props::Usage {
        bytes: 2048,
        files: 3,
        dirs: 1,
    };
    let _ = app.update(Message::Occupied(Some(u)));
    assert!(
        app.msg()
            .is_some_and(|m| m.contains('3') && m.contains('1'))
    );
}

fn files_app(names: &[&str]) -> App {
    let mut app = app_with(Config::default(), State::default());
    let t = app.panes[0].active();
    let (id, generation) = (t.id, t.pending.as_ref().unwrap().0);
    let entries = names.iter().map(|n| entry(n)).collect();
    let _ = app.update(Message::Listed {
        tab: id,
        generation,
        path: std::env::temp_dir(),
        focus: None,
        result: Ok(entries),
        space: None,
    });
    app
}

fn cursor_name(app: &App) -> String {
    app.panes[0].active().panel.current().unwrap().name.clone()
}

#[test]
fn shift_f5_offers_the_name_in_the_same_dir() {
    let mut app = files_app(&["a.txt"]);
    let _ = app.update(Message::Key(Action::Down));
    let _ = app.update(Message::Key(Action::CopySame));
    let Some(Dialog::Input {
        op, input, sources, ..
    }) = &app.dialog
    else {
        panic!("no dialog");
    };
    assert_eq!((*op, input.as_str()), (InputOp::Copy, "a.txt"));
    assert_eq!(sources, &[std::env::temp_dir().join("a.txt")]);
    // on ".." there is nothing to copy
    let mut app = files_app(&["a.txt"]);
    let _ = app.update(Message::Key(Action::CopySame));
    assert!(app.dialog.is_none());
}

#[test]
fn ctrl_shift_f5_links_the_file_into_the_other_pane() {
    let (tmp, mut app, a) = paste_setup();
    let (src, dst) = (tmp.path().join("src"), tmp.path().join("dst"));
    listed_at(&mut app, 0, &src);
    listed_at(&mut app, 1, &dst);
    app.panes[0].active_mut().panel.set_cursor(1); // "a", after ".."
    let _ = app.update(Message::Key(Action::Link));
    let Some(Dialog::Input {
        op: InputOp::Link,
        input,
        ..
    }) = &app.dialog
    else {
        panic!("no link dialog");
    };
    assert_eq!(input, &dir_input(&dst));
    let target = PathBuf::from(input);
    let _ = app.update(Message::DialogSubmit);
    assert!(app.dialog.is_none());
    // What its background task does, then its message.
    let _ = app.update(Message::Linked(
        0,
        link_all(std::slice::from_ref(&a), &target),
    ));
    assert_eq!(std::fs::read_link(dst.join("a")).unwrap(), a);
    // Again: the name is taken.
    let _ = app.update(Message::Linked(
        0,
        link_all(std::slice::from_ref(&a), &target),
    ));
    assert!(app.msg().is_some_and(|m| m.contains("dst/a")));
}

#[test]
fn shift_f5_with_marks_offers_a_mask_for_all_of_them() {
    let mut app = files_app(&["a.txt", "b.txt"]);
    let _ = app.update(Message::Key(Action::Down));
    let _ = app.update(Message::Key(Action::MarkDown));
    let _ = app.update(Message::Key(Action::MarkDown));
    let _ = app.update(Message::Key(Action::CopySame));
    let Some(Dialog::Input {
        op, input, sources, ..
    }) = &app.dialog
    else {
        panic!("no dialog");
    };
    assert_eq!((*op, input.as_str()), (InputOp::Copy, "*.*"));
    assert_eq!(sources.len(), 2);
}

#[test]
fn archive_password_error_asks_and_retries_with_it() {
    let tmp = tempfile::tempdir().unwrap();
    let a = tmp.path().join("e.zip");
    let mut app = app_with(Config::default(), State::default());
    let (reply, rx) = std::sync::mpsc::channel();
    let _ = app.on_job_event(jobs::Event::Error {
        path: a.clone(),
        error: archive::PASSWORD_ERROR.into(),
        reply,
    });
    assert!(matches!(
        app.dialog,
        Some(Dialog::Password { wrong: false, .. })
    ));
    let _ = app.update(Message::ArchivePassword("pw".into()));
    let _ = app.update(Message::DialogSubmit);
    assert_eq!(rx.try_recv(), Ok(ErrorChoice::Retry));
    assert_eq!(archive::password(&a).as_deref(), Some("pw"));
    // Asked again: the one given was wrong; Esc cancels the job.
    let (reply, rx) = std::sync::mpsc::channel();
    let _ = app.on_job_event(jobs::Event::Error {
        path: a.clone(),
        error: archive::PASSWORD_ERROR.into(),
        reply,
    });
    assert!(matches!(
        app.dialog,
        Some(Dialog::Password { wrong: true, .. })
    ));
    let _ = app.update(Message::DialogCancel);
    assert_eq!(rx.try_recv(), Ok(ErrorChoice::Cancel));
    archive::set_password(&a, None);
}

#[test]
fn conflict_rename_sends_the_typed_name() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("a.txt"), "").unwrap();
    let mut app = files_app(&[]);
    let info = |p: PathBuf| ops::FileInfo {
        path: p,
        size: 0,
        mtime: std::time::SystemTime::UNIX_EPOCH,
    };
    let (reply, rx) = std::sync::mpsc::channel();
    let _ = app.on_job_event(jobs::Event::Conflict {
        src: info("/x/a.txt".into()),
        dst: info(tmp.path().join("a.txt")),
        reply,
    });
    let Some(Dialog::Conflict { name, .. }) = &app.dialog else {
        panic!("no dialog");
    };
    assert_eq!(name, "a (1).txt");
    let _ = app.update(Message::DialogInput("b.txt".into()));
    let _ = app.update(Message::Resolve(Resolution::Rename("b.txt".into())));
    assert_eq!(rx.recv().unwrap(), Resolution::Rename("b.txt".into()));
    assert!(app.dialog.is_none());
}

#[test]
fn alt_letter_jumps_to_first_match() {
    let mut app = files_app(&["alpha", "beta", "bravo"]);
    let _ = app.update(Message::Key(Action::QuickSearch('b')));
    assert_eq!(cursor_name(&app), "beta");
    assert_eq!(app.search.as_ref().map(|s| s.text.as_str()), Some("b"));
    let _ = app.update(Message::SearchInput("br".into()));
    assert_eq!(cursor_name(&app), "bravo");
}

#[test]
fn alt_letter_without_match_opens_nothing() {
    let mut app = files_app(&["alpha"]);
    let _ = app.update(Message::Key(Action::QuickSearch('z')));
    assert!(app.search.is_none());
}

#[test]
fn search_rejects_letter_without_match() {
    let mut app = files_app(&["alpha", "beta"]);
    let _ = app.update(Message::Key(Action::QuickSearch('a')));
    let _ = app.update(Message::SearchInput("az".into()));
    assert_eq!(app.search.as_ref().unwrap().text, "a");
    assert_eq!(cursor_name(&app), "alpha");
}

#[test]
fn down_in_search_goes_to_next_match() {
    let mut app = files_app(&["b1", "x", "b2"]);
    let _ = app.update(Message::Key(Action::QuickSearch('b')));
    let _ = app.update(Message::Key(Action::Down));
    assert_eq!(cursor_name(&app), "b2");
    let _ = app.update(Message::Key(Action::Down)); // wraps
    assert_eq!(cursor_name(&app), "b1");
    assert!(app.search.is_some());
}

#[test]
fn regression_alt_letter_adds_to_the_open_search() {
    let mut app = files_app(&["alpha", "beta", "bravo"]);
    let _ = app.update(Message::Key(Action::QuickSearch('b')));
    // Unfocused, the field's typing comes as keys: Alt+r adds to "b", not starts over.
    let _ = app.update(Message::Key(Action::QuickSearch('r')));
    assert_eq!(app.search.as_ref().map(|s| s.text.as_str()), Some("br"));
    assert_eq!(cursor_name(&app), "bravo");
    let _ = app.update(Message::Key(Action::QuickSearch('z'))); // no match: not taken
    assert_eq!(app.search.as_ref().map(|s| s.text.as_str()), Some("br"));
}

#[test]
fn other_key_closes_search_and_acts() {
    let mut app = files_app(&["alpha", "beta"]);
    let _ = app.update(Message::Key(Action::QuickSearch('b')));
    let _ = app.update(Message::Key(Action::Copy));
    assert!(app.search.is_none());
    assert!(matches!(
        app.dialog,
        Some(Dialog::Input {
            op: InputOp::Copy,
            ..
        })
    ));
}

#[test]
fn enter_in_search_opens_the_dir() {
    let mut app = files_app(&[]);
    let t = app.panes[0].active();
    let (id, cwd) = (t.id, t.panel.cwd().to_path_buf());
    let _ = app.load(0, cwd.clone(), None);
    let generation = app.panes[0].active().pending.as_ref().unwrap().0;
    let mut sub = entry("sub");
    sub.kind = shagoff_core::listing::Kind::Dir;
    let _ = app.update(Message::Listed {
        tab: id,
        generation,
        path: cwd.clone(),
        focus: None,
        result: Ok(vec![sub]),
        space: None,
    });
    let _ = app.update(Message::Key(Action::QuickSearch('s')));
    let _ = app.update(Message::SearchSubmit);
    assert!(app.search.is_none());
    let pending = app.panes[0]
        .active()
        .pending
        .as_ref()
        .map(|(_, p)| p.clone());
    assert_eq!(pending, Some(cwd.join("sub")));
}

#[test]
fn ctrl_s_filters_enter_keeps_escape_clears() {
    let mut app = files_app(&["a.rs", "b.md", "c.rs"]);
    let _ = app.update(Message::Key(Action::QuickFilter));
    let _ = app.update(Message::SearchInput("*.rs".into()));
    let shown = |app: &App| app.panes[0].active().panel.entries().len();
    assert_eq!(shown(&app), 3); // .., a.rs, c.rs
    let _ = app.update(Message::SearchSubmit);
    assert!(app.search.is_none());
    assert_eq!(app.panes[0].active().panel.filter(), Some("*.rs"));
    let _ = app.update(Message::DialogCancel); // Escape
    assert_eq!(app.panes[0].active().panel.filter(), None);
    assert_eq!(shown(&app), 4);
}

#[test]
fn escape_closes_search_then_clears_filter() {
    let mut app = files_app(&["a.rs", "b.md"]);
    let _ = app.update(Message::Key(Action::QuickFilter));
    let _ = app.update(Message::SearchInput("a".into()));
    let _ = app.update(Message::DialogCancel); // in filter field: close + clear
    assert!(app.search.is_none());
    assert_eq!(app.panes[0].active().panel.filter(), None);

    let _ = app.update(Message::Key(Action::QuickSearch('a')));
    let _ = app.update(Message::DialogCancel); // in search field: only close
    assert!(app.search.is_none());
    assert_eq!(cursor_name(&app), "a.rs");
}

#[test]
fn clicking_a_list_entry_opens_it() {
    let mut app = app_with(Config::default(), State::default());
    app.drives = vec![
        Drive {
            label: "/".into(),
            path: "/".into(),
        },
        Drive {
            label: "etc".into(),
            path: "/etc".into(),
        },
    ];
    let _ = app.update(Message::Key(Action::Drives(1)));
    app.panes[1].active_mut().pending = None;
    let _ = app.update(Message::ListPick(1));
    assert!(app.dialog.is_none());
    let pending = app.panes[1]
        .active()
        .pending
        .as_ref()
        .map(|(_, p)| p.clone());
    assert_eq!(pending, Some(PathBuf::from("/etc")));
}

/// Simulate a finished scan of `path` for the active tab of `side`.
fn arrive(app: &mut App, side: usize, path: &Path) {
    let _ = app.load(side, path.into(), None);
    let t = app.panes[side].active();
    let (id, generation) = (t.id, t.pending.as_ref().unwrap().0);
    let _ = app.update(Message::Listed {
        tab: id,
        generation,
        path: path.into(),
        focus: None,
        result: Ok(vec![]),
        space: None,
    });
}

fn pending_of(app: &App, side: usize) -> Option<PathBuf> {
    app.panes[side]
        .active()
        .pending
        .as_ref()
        .map(|(_, p)| p.clone())
}

#[test]
fn alt_left_right_walk_tab_history() {
    let mut app = app_with(Config::default(), State::default());
    arrive(&mut app, 0, Path::new("/usr"));
    arrive(&mut app, 0, Path::new("/etc"));
    let _ = app.update(Message::Key(Action::HistoryBack));
    assert_eq!(pending_of(&app, 0), Some(PathBuf::from("/usr")));
    arrive(&mut app, 0, Path::new("/usr")); // the jump lands
    let _ = app.update(Message::Key(Action::HistoryForward));
    assert_eq!(pending_of(&app, 0), Some(PathBuf::from("/etc")));
}

#[test]
fn rescan_does_not_add_history() {
    let mut app = app_with(Config::default(), State::default());
    arrive(&mut app, 0, &std::env::temp_dir()); // the start dir's first listing
    arrive(&mut app, 0, Path::new("/usr"));
    arrive(&mut app, 0, Path::new("/usr")); // Ctrl+R / watcher
    let _ = app.update(Message::Key(Action::HistoryBack));
    // One step back leaves /usr for the start dir, not /usr again.
    assert_eq!(pending_of(&app, 0), Some(std::env::temp_dir()));
}

#[test]
fn alt_down_lists_history() {
    let mut app = app_with(Config::default(), State::default());
    arrive(&mut app, 0, Path::new("/usr"));
    arrive(&mut app, 0, Path::new("/etc"));
    let _ = app.update(Message::Key(Action::HistoryList));
    let Some(Dialog::List {
        kind: ListKind::History,
        items,
        ..
    }) = &app.dialog
    else {
        panic!("no history list");
    };
    assert_eq!(items[0].path, Path::new("/etc"));
    assert!(items.iter().any(|i| i.path == Path::new("/usr")));
}

#[test]
fn ctrl_u_swaps_panes() {
    let mut app = app_with(Config::default(), State::default());
    arrive(&mut app, 0, Path::new("/usr"));
    arrive(&mut app, 1, Path::new("/etc"));
    app.space = [Some((1, 10)), Some((2, 20))];
    let _ = app.update(Message::Key(Action::SwapPanes));
    assert_eq!(app.space, [Some((2, 20)), Some((1, 10))]); // free space follows its pane
    assert_eq!(app.panes[0].active().panel.cwd(), Path::new("/etc"));
    assert_eq!(app.panes[1].active().panel.cwd(), Path::new("/usr"));
}

#[test]
fn regression_swap_keeps_scan_in_flight() {
    let mut app = app_with(Config::default(), State::default());
    let _ = app.load(0, "/usr".into(), None);
    let t = app.panes[0].active();
    let (id, generation) = (t.id, t.pending.as_ref().unwrap().0);
    let _ = app.update(Message::Key(Action::SwapPanes)); // tab now on side 1
    let _ = app.update(Message::Listed {
        tab: id,
        generation,
        path: "/usr".into(),
        focus: None,
        result: Ok(vec![]),
        space: None,
    });
    let t = app.panes[1].active();
    assert_eq!(t.panel.cwd(), Path::new("/usr"));
    assert!(t.pending.is_none(), "tab stuck in pending");
}

fn hotlist_items(app: &App) -> Vec<PathBuf> {
    match &app.dialog {
        Some(Dialog::List {
            kind: ListKind::Hotlist,
            items,
            ..
        }) => items.iter().map(|i| i.path.clone()).collect(),
        _ => panic!("hotlist not open"),
    }
}

#[test]
fn regression_hotlist_delete_after_an_outside_change_deletes_the_entry_shown() {
    let hot = |name: &str, path: &str| HotEntry {
        name: name.into(),
        path: path.into(),
    };
    let config = Config {
        hotlist: vec![hot("A", "/a"), hot("B", "/b")],
        ..Config::default()
    };
    let mut app = app_with(config.clone(), State::default());
    let _ = app.update(Message::Key(Action::Hotlist));
    let _ = app.update(Message::Key(Action::Down)); // on B
    // Edited outside meanwhile: an entry added at the top.
    let edited = Config {
        hotlist: vec![hot("X", "/x"), hot("A", "/a"), hot("B", "/b")],
        ..config
    };
    let _ = app.update(Message::Config(Box::new(edited)));
    let _ = app.update(Message::Key(Action::Delete));
    let left: Vec<&str> = app.config.hotlist.iter().map(|h| h.name.as_str()).collect();
    assert_eq!(left, ["X", "A"]); // B, the one under the cursor, not A
}

#[test]
fn hotlist_hot_letters_pick_or_step() {
    let hot = |name: &str, path: &str| HotEntry {
        name: name.into(),
        path: path.into(),
    };
    let tmp = std::env::temp_dir();
    let config = Config {
        hotlist: vec![
            hot("&Wiki", "/w1"),
            hot("&Temp", &tmp.display().to_string()),
            hot("&Work", "/w2"),
            hot("R&&D", "/rd"),
        ],
        ..Config::default()
    };
    let mut app = app_with(config, State::default());
    let cursor = |app: &App| match &app.dialog {
        Some(Dialog::List { cursor, .. }) => *cursor,
        _ => panic!("hotlist not open"),
    };
    let _ = app.update(Message::Key(Action::Hotlist));
    // Shared hot letter: steps, does not open.
    let _ = app.update(Message::CmdType('W'));
    assert_eq!(cursor(&app), 2);
    let _ = app.update(Message::CmdType('w'));
    assert_eq!(cursor(&app), 0);
    // `&&` is no hot letter: "r" only moves there.
    let _ = app.update(Message::CmdType('r'));
    assert_eq!(cursor(&app), 3);
    // The only one with "t": opened at once.
    app.panes[0].active_mut().pending = None;
    let _ = app.update(Message::CmdType('t'));
    assert!(app.dialog.is_none());
}

#[test]
fn regression_hot_letters_not_taken_by_back_row_or_dir_names() {
    let hot = |name: &str, path: &str| HotEntry {
        name: name.into(),
        path: path.into(),
    };
    let tmp = std::env::temp_dir();
    let config = Config {
        hotlist: vec![
            hot("-&Projects", ""),
            hot("&Pop", &tmp.display().to_string()),
            hot("--", ""),
        ],
        ..Config::default()
    };
    let mut app = app_with(config, State::default());
    let _ = app.update(Message::Key(Action::Hotlist));
    let _ = app.update(Message::CmdType('p')); // the only "p" here: into the submenu
    let _ = app.update(Message::CmdType('p')); // "‹ Projects" has no hot letter: Pop opens
    assert!(app.dialog.is_none());
    assert_eq!(crate::hotlist::menu_name(Path::new("/x/R&D")), "R&&D");
}

#[test]
fn hotlist_submenus_open_go_back_and_take_new_dirs() {
    let hot = |name: &str, path: &str| HotEntry {
        name: name.into(),
        path: path.into(),
    };
    let config = Config {
        hotlist: vec![
            hot("a", "/a"),
            hot("-Work", ""),
            hot("w", "/w"),
            hot("--", ""),
        ],
        ..Config::default()
    };
    let mut app = app_with(config, State::default());
    let labels = |app: &App| match &app.dialog {
        Some(Dialog::List { items, .. }) => {
            items.iter().map(|i| i.label.clone()).collect::<Vec<_>>()
        }
        _ => panic!("hotlist not open"),
    };
    let _ = app.update(Message::Key(Action::Hotlist));
    assert_eq!(labels(&app)[..2], ["a", "Work"]);
    let _ = app.update(Message::Key(Action::Down));
    let _ = app.update(Message::Key(Action::Right)); // into "Work"
    assert_eq!(labels(&app)[..2], ["‹ Work", "w"]);
    assert_eq!(hotlist_items(&app)[1], PathBuf::from("/w"));
    let _ = app.update(Message::ListPick(labels(&app).len() - 2)); // add the panel's dir here
    assert_eq!(
        app.config.hotlist[3].path,
        app.panes[0].active().panel.cwd()
    );
    assert_eq!(app.config.hotlist[4].name, "--");
    let _ = app.update(Message::Key(Action::Hotlist));
    let _ = app.update(Message::ListPick(1));
    let _ = app.update(Message::Key(Action::Parent)); // Backspace: back to the top, on "Work"
    assert_eq!(labels(&app)[1], "Work");
    assert!(matches!(&app.dialog, Some(Dialog::List { cursor: 1, .. })));
}

#[test]
fn the_running_job_pauses_and_a_queued_one_can_be_taken_out() {
    let mut app = files_app(&["a", "b", "c"]);
    let dir = app.panes[0].active().panel.cwd().to_path_buf();
    for n in ["a", "b", "c"] {
        let _ = app.start_job(
            0,
            OpKind::Delete,
            Job::Delete {
                paths: vec![dir.join(n)],
                permanent: true,
            },
            None,
        );
    }
    let _ = app.update(Message::JobPause);
    assert!(app.job.as_ref().unwrap().paused());
    let _ = app.update(Message::JobPause);
    assert!(!app.job.as_ref().unwrap().paused());
    let labels = |app: &App| app.queue.iter().map(Queued::label).collect::<Vec<_>>();
    assert_eq!(labels(&app), ["b", "c"]);
    let _ = app.update(Message::QueueUp(1));
    assert_eq!(labels(&app), ["c", "b"]);
    let _ = app.update(Message::QueueUp(0)); // already first
    assert_eq!(labels(&app), ["c", "b"]);
    let _ = app.update(Message::Unqueue(0));
    assert_eq!(labels(&app), ["b"]);
    app.cancel_job();
}

#[test]
fn a_job_in_the_background_frees_the_panels_and_a_second_one_waits_in_the_queue() {
    let mut app = files_app(&["a", "b", "c"]);
    let dir = app.panes[0].active().panel.cwd().to_path_buf();
    let gone = vec![dir.join("a")];
    let _ = app.start_job(
        0,
        OpKind::Delete,
        Job::Delete {
            paths: gone,
            permanent: true,
        },
        None,
    );
    assert!(app.busy() && app.dialog().is_some());
    let before = cursor_name(&app);
    let _ = app.update(Message::Key(Action::Down)); // the dialog is up: the panel waits
    assert_eq!(cursor_name(&app), before);
    let _ = app.update(Message::JobHide);
    assert!(!app.busy() && app.dialog().is_none());
    let _ = app.update(Message::Key(Action::Down));
    assert_ne!(cursor_name(&app), before);
    let _ = app.update(Message::DialogCancel); // Esc: not for the hidden job
    assert!(
        !app.job
            .as_ref()
            .unwrap()
            .control
            .cancel
            .load(Ordering::Relaxed)
    );
    let again = vec![dir.join("b")];
    let _ = app.start_job(
        0,
        OpKind::Delete,
        Job::Delete {
            paths: again,
            permanent: true,
        },
        None,
    );
    assert_eq!(app.msg(), Some(fl!("job-queued", n = 1).as_str()));
    assert_eq!(app.queue.len(), 1);
    let _ = app.update(Message::JobShow);
    assert!(app.busy());
    // The first one ends: the queued one starts, in the background.
    let _ = app.finish_job(&Report::default());
    assert!(app.queue.is_empty());
    assert!(
        app.job
            .as_ref()
            .is_some_and(|j| j.hidden && j.kind == OpKind::Delete)
    );
    assert!(!app.busy());
    // Cancel empties the queue as well.
    let nothing = Job::Delete {
        paths: Vec::new(),
        permanent: true,
    };
    let _ = app.start_job(0, OpKind::Delete, nothing, None);
    assert_eq!(app.queue.len(), 1);
    let _ = app.update(Message::CancelJob);
    assert!(app.queue.is_empty());
}

fn idle_job() -> Job {
    Job::Delete {
        paths: Vec::new(),
        permanent: true,
    }
}

#[test]
fn regression_a_queued_copy_is_planned_when_it_starts() {
    let tmp = tempfile::tempdir().unwrap();
    let (f, new) = (tmp.path().join("f"), tmp.path().join("new"));
    std::fs::write(&f, "x").unwrap();
    let mut app = app_with(Config::default(), State::default());
    listed_at(&mut app, 0, tmp.path());
    let _ = app.start_job(0, OpKind::Delete, idle_job(), None);
    app.dialog = Some(Dialog::Input {
        op: InputOp::Copy,
        side: 0,
        sources: vec![f],
        input: new.display().to_string(),
    });
    let _ = app.update(Message::DialogSubmit);
    assert_eq!(app.queue.len(), 1);
    // The running one made the target dir meanwhile: the file goes into it, not over it.
    std::fs::create_dir(&new).unwrap();
    let _ = app.finish_job(&Report::default());
    let copied = new.join("f");
    for _ in 0..300 {
        if copied.exists() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(copied.exists());
}

#[test]
fn regression_every_job_dropped_from_the_queue_says_why() {
    let tmp = tempfile::tempdir().unwrap();
    let mut app = app_with(Config::default(), State::default());
    listed_at(&mut app, 0, tmp.path());
    let _ = app.start_job(0, OpKind::Delete, idle_job(), None);
    for name in ["one", "two"] {
        std::fs::create_dir(tmp.path().join(name)).unwrap();
        app.dialog = Some(Dialog::Input {
            op: InputOp::Copy,
            side: 0,
            sources: vec![tmp.path().join(name)],
            input: tmp.path().join(format!("to-{name}")).display().to_string(),
        });
        let _ = app.update(Message::DialogSubmit);
    }
    assert_eq!(app.queue.len(), 2);
    // Each target became a link to its own source meanwhile: a dir into itself, refused.
    for name in ["one", "two"] {
        std::os::unix::fs::symlink(name, tmp.path().join(format!("to-{name}"))).unwrap();
    }
    let _ = app.finish_job(&Report::default());
    assert!(app.job.is_none() && app.queue.is_empty());
    let msg = app.msg().unwrap();
    assert!(msg.contains("one") && msg.contains("two"), "{msg}");
}

#[test]
fn regression_quick_search_up_in_an_empty_list() {
    let mut app = app_with(Config::default(), State::default());
    app.panes[0]
        .active_mut()
        .panel
        .set_listing("/".into(), Vec::new(), None);
    app.search = Some(Search {
        side: 0,
        text: "a".into(),
        filter: false,
    });
    let _ = app.update(Message::Key(Action::Up)); // was: `0 + 0 - 1`, overflow
    assert_eq!(app.panes[0].active().panel.cursor(), 0);
}

#[test]
fn regression_closing_the_window_asks_while_a_job_runs_or_waits() {
    let mut app = files_app(&[]);
    assert!(matches!(app.on_app_exit(), Some(Message::Quit)));
    let _ = app.start_job(0, OpKind::Delete, idle_job(), None);
    let _ = app.update(Message::Quit);
    assert!(matches!(app.dialog, Some(Dialog::ConfirmExit)));
    let _ = app.update(Message::DialogCancel); // stay: the job goes on
    assert!(app.dialog.is_none());
    assert!(
        !app.job
            .as_ref()
            .unwrap()
            .control
            .cancel
            .load(Ordering::Relaxed)
    );
    // A job's question steps aside for the exit dialog and comes back after Cancel.
    let (reply, _rx) = std::sync::mpsc::channel();
    let _ = app.on_job_event(jobs::Event::Error {
        path: "x".into(),
        error: "e".into(),
        reply,
    });
    let _ = app.update(Message::Quit);
    assert!(matches!(app.dialog, Some(Dialog::ConfirmExit)));
    let _ = app.update(Message::DialogCancel);
    assert!(matches!(app.dialog, Some(Dialog::Error { .. })));
    let _ = app.update(Message::Quit);
    let _ = app.update(Message::DialogSubmit);
    assert!(app.held.is_none() && app.quitting);
    assert!(
        app.job
            .as_ref()
            .unwrap()
            .control
            .cancel
            .load(Ordering::Relaxed)
    );
}

#[test]
fn regression_a_background_question_waits_for_the_open_dialog() {
    let mut app = files_app(&["a"]);
    let _ = app.update(Message::Key(Action::Mkdir));
    let (reply, _rx) = std::sync::mpsc::channel();
    let _ = app.on_job_event(jobs::Event::Error {
        path: "x".into(),
        error: "e".into(),
        reply,
    });
    assert!(matches!(app.dialog, Some(Dialog::Input { .. })));
    let _ = app.update(Message::DialogCancel);
    assert!(matches!(app.dialog, Some(Dialog::Error { .. })));
}

#[test]
fn f2_in_the_copy_dialog_runs_it_in_the_background() {
    let (_tmp, mut app, a) = paste_setup();
    let _ = app.update(Message::Drop {
        side: 0,
        dir: None,
        paths: vec![a],
        mv: false,
    });
    assert!(matches!(
        app.dialog,
        Some(Dialog::Input {
            op: InputOp::Copy,
            ..
        })
    ));
    let _ = app.update(Message::FieldKey(Action::Rename)); // F2
    assert!(app.dialog.is_none());
    assert!(app.job.as_ref().is_some_and(|j| j.hidden));
}

#[test]
fn dragging_a_header_edge_resizes_and_is_saved_on_release() {
    let mut app = app_with(Config::default(), State::default());
    let before = app.config.columns;
    let _ = app.update(Message::ColDragStart(0, 3)); // Size | Date
    let _ = app.update(Message::ColDrag(500.0)); // first move: where the drag starts
    let _ = app.update(Message::ColDrag(530.0));
    assert_eq!(app.cols[1], before[1] + 30.0); // Size wider, shown at once
    assert_eq!(app.config.columns, before); // not saved yet
    let _ = app.update(Message::ColDragEnd);
    assert_eq!(app.config.columns[1], before[1] + 30.0);
    assert!(app.col_drag.is_none());
}

#[test]
fn reset_columns_restores_and_saves_the_defaults() {
    let mut app = app_with(Config::default(), State::default());
    let _ = app.update(Message::ColDragStart(0, 2));
    let _ = app.update(Message::ColDrag(0.0));
    let _ = app.update(Message::ColDrag(40.0));
    let _ = app.update(Message::ColDragEnd);
    assert_ne!(app.config.columns, Config::default().columns);
    let _ = app.update(Message::Key(Action::ResetColumns));
    assert_eq!(app.cols, Config::default().columns);
    assert_eq!(app.config.columns, Config::default().columns);
}

#[test]
fn regression_column_drag_ends_on_any_release_and_esc_undoes_it() {
    // The release over the file list never reached the pane's own handler: the drag stuck.
    let up = cosmic::iced::Event::Mouse(cosmic::iced::mouse::Event::ButtonReleased(
        cosmic::iced::mouse::Button::Left,
    ));
    let id = cosmic::iced::window::Id::unique();
    assert!(matches!(
        route_event(up, event::Status::Captured, id),
        Some(Message::ColDragEnd)
    ));
    let mut app = app_with(Config::default(), State::default());
    let before = app.cols;
    let _ = app.update(Message::ColDragStart(0, 2));
    let _ = app.update(Message::ColDrag(100.0));
    let _ = app.update(Message::ColDrag(150.0));
    assert_ne!(app.cols, before);
    let _ = app.update(Message::Escape(app.window_id()));
    assert!(app.col_drag.is_none());
    assert_eq!(app.cols, before);
}

#[test]
fn shift_f4_creates_the_file_and_puts_the_cursor_on_it() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("old.txt"), "kept").unwrap();
    // `true` as the editor: nothing opens on the desktop running the tests.
    let config = Config {
        editor: vec!["true".into()],
        ..Config::default()
    };
    let mut app = app_with(config, State::default());
    listed_at(&mut app, 0, tmp.path());
    let _ = app.update(Message::Key(Action::Down)); // old.txt
    let _ = app.update(Message::Key(Action::NewFile));
    assert!(
        matches!(&app.dialog, Some(Dialog::Input { op: InputOp::NewFile, input, .. }) if input == "old.txt")
    );
    let _ = app.update(Message::DialogSubmit); // exists: opened, not emptied
    assert_eq!(
        std::fs::read_to_string(tmp.path().join("old.txt")).unwrap(),
        "kept"
    );
    let _ = app.update(Message::Key(Action::NewFile));
    let _ = app.update(Message::DialogInput("new.md".into()));
    let _ = app.update(Message::DialogSubmit);
    assert!(tmp.path().join("new.md").is_file());
    let _ = app.update(Message::Key(Action::NewFile));
    let _ = app.update(Message::DialogInput("a/b".into())); // one name only
    let _ = app.update(Message::DialogSubmit);
    assert!(!tmp.path().join("a").exists());
}

#[test]
fn history_clear_and_keep_history_off() {
    let mut app = app_with(Config::default(), State::default());
    for p in ["/a", "/b"] {
        app.panes[0].active_mut().history.visit(Path::new(p));
    }
    let _ = app.update(Message::Key(Action::HistoryList));
    let _ = app.update(Message::ListClear);
    assert!(app.dialog.is_none());
    assert_eq!(
        app.panes[0].active().history.recent(),
        [PathBuf::from("/b")]
    );
    app.commands = vec!["ls".into()];
    let _ = app.update(Message::Key(Action::CmdHistory));
    let _ = app.update(Message::ListClear);
    assert!(app.commands.is_empty());
    // Off: kept for this run, not written.
    app.commands = vec!["ls".into()];
    app.masks = vec!["*.rs".into()];
    let _ = app.update(Message::Setting(crate::drawer::Setting::KeepHistory(false)));
    assert_eq!(app.commands, ["ls"]);
    assert!(app.saved.commands.is_empty() && app.saved.masks.is_empty());
    assert!(app.saved.panes[0].history.is_empty());
    let _ = app.update(Message::Setting(crate::drawer::Setting::KeepHistory(true)));
    assert_eq!(app.saved.commands, ["ls"]);
}

#[test]
fn regression_dir_history_survives_a_restart() {
    let mut app = app_with(Config::default(), State::default());
    for p in ["/a", "/b"] {
        app.panes[0].active_mut().history.visit(Path::new(p));
    }
    let _ = app.update(Message::Key(Action::HistoryList)); // any message saves the state
    let _ = app.update(Message::DialogCancel);
    let saved = app.saved.clone();
    assert_eq!(saved.panes[0].history.len(), saved.panes[0].tabs.len());
    let again = app_with(Config::default(), saved);
    let recent = again.panes[0].active().history.recent();
    assert!(recent.contains(&PathBuf::from("/a")), "{recent:?}");
}

#[test]
fn regression_ctrl_is_let_go_when_the_window_loses_focus() {
    let ev = cosmic::iced::Event::Window(cosmic::iced::window::Event::Unfocused);
    let id = cosmic::iced::window::Id::unique();
    let m = route_event(ev, event::Status::Ignored, id);
    assert!(matches!(m, Some(Message::Modifiers(m)) if m.is_empty()));
}

#[test]
fn the_terminal_is_chosen_in_the_menu_and_runs_shift_enter() {
    let mut app = app_with(Config::default(), State::default());
    let term = |id: &str, prog: &str| Terminal {
        id: id.into(),
        name: id.into(),
        argv: vec![prog.into()],
    };
    let list = vec![term("a.desktop", "alacritty"), term("f.desktop", "foot")];
    let _ = app.update(Message::Terminals(list, Some(0)));
    assert_eq!(app.terminal_prefix(), ["alacritty", "-e"]); // the system's default
    let pick = |app: &mut App, row: usize| {
        let _ = app.update(Message::Key(Action::PickTerminal));
        let _ = app.update(Message::ListPick(row));
    };
    pick(&mut app, 2); // .., default, a, f → "f"
    assert_eq!(app.config.terminal_app, "f.desktop");
    assert_eq!(app.terminal_prefix(), ["foot"]);
    let _ = app.update(Message::Key(Action::PickTerminal));
    match &app.dialog {
        Some(Dialog::List { cursor, items, .. }) => {
            assert_eq!(*cursor, 2); // the chosen one
            assert!(items[2].label.starts_with('✓') && !items[0].label.starts_with('✓'));
        }
        _ => panic!("no list"),
    }
    app.dialog = None;
    pick(&mut app, 0);
    assert_eq!(app.config.terminal_app, "");
    // `terminal` changed by hand (before the menu existed): it still wins.
    app.config.terminal = vec!["xterm".into(), "-e".into()];
    assert_eq!(app.terminal_prefix(), ["xterm", "-e"]);
}

#[test]
fn columns_grow_with_the_text_and_are_kept_for_the_default_size() {
    let mut app = app_with(Config::default(), State::default());
    let base = app.cols;
    app.config.panel_size = 20;
    let k = app.config.text_scale();
    assert!(k > 1.0);
    assert_eq!(app.col_widths(), base.map(|w| w * k).to_vec());
    // A drag of 20 px on screen at the bigger size: 20 / k kept.
    let _ = app.update(Message::ColDragStart(0, 3));
    let _ = app.update(Message::ColDrag(100.0));
    let _ = app.update(Message::ColDrag(120.0));
    assert!((app.cols[1] - (base[1] + 20.0 / k)).abs() < 0.01);
}

#[test]
fn found_in_archives_are_listed_and_deleted_one_archive_at_a_time() {
    use std::io::Write;
    let tmp = tempfile::tempdir().unwrap();
    let zip_at = |name: &str| {
        let a = tmp.path().join(name);
        let mut z = zip::ZipWriter::new(std::fs::File::create(&a).unwrap());
        z.start_file("f.txt", zip::write::SimpleFileOptions::default())
            .unwrap();
        z.write_all(b"x").unwrap();
        z.finish().unwrap();
        a
    };
    let (a, b) = (zip_at("a.zip"), zip_at("b.zip"));
    let disk = tmp.path().join("d.txt");
    std::fs::write(&disk, "d").unwrap();
    let found = vec![disk.clone(), a.join("f.txt"), b.join("f.txt")];
    let rows = read_listing(tmp.path(), false, Some(&found), &AtomicBool::new(false)).unwrap();
    let names: Vec<&str> = rows.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["d.txt", "a.zip/f.txt", "b.zip/f.txt"]);
    assert_eq!(
        located(tmp.path(), &rows[1].os_name),
        (a.clone(), "f.txt".into())
    );
    // F8 on all three: a's rewrite runs, b's and the disk file's wait their turn (never run
    // here: the trash is not the test's to touch).
    let mut app = app_with(Config::default(), State::default());
    let _ = app.start_delete(0, false, vec![a.join("f.txt"), b.join("f.txt"), disk]);
    assert!(app.job.is_some());
    assert_eq!(app.queue.len(), 2);
}

#[test]
fn a_nested_archive_shows_where_it_lies_and_is_never_changed() {
    let mut app = app_with(Config::default(), State::default());
    let tmp = archive::temp_root().join("1/0/inner.zip");
    app.nested
        .insert(tmp.clone(), PathBuf::from("/d/outer.zip/sub"));
    let shown = PathBuf::from("/d/outer.zip/sub/inner.zip/x");
    assert_eq!(app.shown(&tmp.join("x")), shown);
    assert_eq!(app.real(&shown), tmp.join("x"));
    assert_eq!(
        app.real(Path::new("/d/outer.zip")),
        PathBuf::from("/d/outer.zip")
    );
    // One deeper: an archive opened from inside the nested one.
    let deeper = archive::temp_root().join("1/1/core.7z");
    app.nested.insert(deeper.clone(), tmp.join("lib"));
    let far = PathBuf::from("/d/outer.zip/sub/inner.zip/lib/core.7z");
    assert_eq!(app.shown(&deeper), far);
    assert_eq!(app.real(&far.join("a")), deeper.join("a"));
    let change = shagoff_core::repack::Change::Delete(vec![PathBuf::from("x")]);
    let repack = |archive: PathBuf| Job::Repack {
        archive,
        change: change.clone(),
    };
    assert!(changes_nested(&repack(tmp)));
    assert!(!changes_nested(&repack("/d/outer.zip".into())));
}

#[test]
fn shift_enter_from_the_panel_runs_the_line_else_enters_a_dir() {
    let mut app = app_with(Config::default(), State::default());
    app.config.terminal = vec!["true".into()]; // a harmless "terminal"
    app.cmdline = "echo hi".into(); // typed, then a click on the panel
    let _ = app.update(Message::Key(Action::RunTerminal));
    assert!(app.cmdline.is_empty());
    assert_eq!(app.commands.first().map(String::as_str), Some("echo hi"));
    // Nothing typed, the cursor on a dir: Shift+Enter enters it, as Enter.
    let mut app = files_app(&[]);
    let t = app.panes[0].active();
    let (id, cwd) = (t.id, t.panel.cwd().to_path_buf());
    let _ = app.load(0, cwd.clone(), None);
    let generation = app.panes[0].active().pending.as_ref().unwrap().0;
    let mut sub = entry("sub");
    sub.kind = shagoff_core::listing::Kind::Dir;
    let _ = app.update(Message::Listed {
        tab: id,
        generation,
        path: cwd.clone(),
        focus: Some("sub".into()),
        result: Ok(vec![sub]),
        space: None,
    });
    let _ = app.update(Message::Key(Action::RunTerminal));
    assert_eq!(pending_path(&app), Some(cwd.join("sub")));
}

#[test]
fn builtin_terminal_rows_shell_and_ctrl_o() {
    let mut app = app_with(Config::default(), State::default());
    // Off: Ctrl+O does nothing (TC's console is not ours to hide panels for).
    let _ = app.update(Message::Key(Action::TermFull));
    assert!(!app.term_full);
    let _ = app.update(Message::Key(Action::ToggleTerminal));
    assert!(app.config.term_builtin);
    // Its top edge dragged up 3 lines: 3 rows more, saved on release.
    let line = (f32::from(app.config.panel_size()) * 1.4).ceil();
    let rows = app.config.term_rows;
    let _ = app.update(Message::TermDragStart);
    let _ = app.update(Message::TermDragAt(400.0));
    let _ = app.update(Message::TermDragAt(400.0 - 3.0 * line));
    assert_eq!(app.term_rows(), rows + 3);
    assert_eq!(app.config.term_rows, rows); // not yet
    let _ = app.update(Message::TermDragEnd);
    assert_eq!(app.config.term_rows, rows + 3);
    // Commands → Shell…: `$SHELL` first, a pick sets it.
    let _ = app.update(Message::Key(Action::PickShell));
    let Some(Dialog::List { items, .. }) = &app.dialog else {
        panic!("no list");
    };
    let last = items.len() - 1;
    let shell = items[last].path.clone();
    let _ = app.update(Message::ListPick(last));
    assert_eq!(PathBuf::from(&app.config.term_shell), shell);
    let _ = app.update(Message::Key(Action::ToggleTerminal));
    assert!(!app.config.term_builtin && app.term.is_none());
}

#[test]
fn regression_keys_the_focused_terminal_took_do_not_act_on_the_panels() {
    let mut app = files_app(&["a"]);
    let _ = app.update(Message::Key(Action::Down)); // on "a"
    let _ = app.update(Message::TermFocused(true));
    // F8 in mc, Ctrl+R in bash: `route_event` passes them on as field keys.
    let _ = app.update(Message::FieldKey(Action::Delete));
    let _ = app.update(Message::FieldKey(Action::Reload));
    assert!(app.dialog.is_none());
    let _ = app.update(Message::TermFocused(false));
    let _ = app.update(Message::FieldKey(Action::Delete));
    assert!(app.dialog.is_some()); // the panels' again
}

#[test]
fn mask_dialog_remembers_masks() {
    let mut app = app_with(Config::default(), State::default());
    let mask = |app: &App| match &app.dialog {
        Some(Dialog::Mask { input, .. }) => input.clone(),
        _ => panic!("no mask dialog"),
    };
    let _ = app.update(Message::Key(Action::SelectGroup));
    assert_eq!(mask(&app), "*.*"); // nothing typed yet: as TC
    app.dialog = None;
    for m in ["*.rs", "*.txt"] {
        let _ = app.update(Message::Key(Action::SelectGroup));
        let _ = app.update(Message::DialogInput(m.into()));
        let _ = app.update(Message::DialogSubmit);
    }
    assert_eq!(app.masks, ["*.txt", "*.rs"]);
    let _ = app.update(Message::Key(Action::UnselectGroup));
    assert_eq!(mask(&app), "*.txt"); // opens with the last one
    let _ = app.update(Message::FieldKey(Action::Up));
    assert_eq!(mask(&app), "*.rs");
    let _ = app.update(Message::FieldKey(Action::Down));
    assert_eq!(mask(&app), "*.txt");
    // regression: the field lets ↑ / ↓ through as plain keys, which went nowhere
    let _ = app.update(Message::Key(Action::Up));
    assert_eq!(mask(&app), "*.rs");
}

#[test]
fn regression_hotlist_add_of_a_non_utf8_dir_says_why() {
    use std::os::unix::ffi::OsStrExt;
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join(std::ffi::OsStr::from_bytes(b"caf\xe9"));
    std::fs::create_dir(&dir).unwrap();
    let mut app = app_with(Config::default(), State::default());
    listed_at(&mut app, 0, &dir);
    let _ = app.update(Message::Key(Action::Hotlist));
    let add = match &app.dialog {
        Some(Dialog::List { items, .. }) => items.iter().position(|i| i.kind == Item::Add),
        _ => None,
    };
    let _ = app.pick(add.expect("«Add»"));
    assert!(app.config.hotlist.is_empty());
    assert_eq!(app.status.as_ref().map(|s| s.kind), Some(StatusKind::Error));
}

#[test]
fn hotlist_add_once_delete_clamps() {
    let mut app = app_with(Config::default(), State::default());
    let cwd = app.panes[0].active().panel.cwd().to_path_buf();
    let _ = app.update(Message::Key(Action::Hotlist));
    let _ = app.update(Message::Key(Action::Enter)); // empty: row 0 is "add current dir"
    let paths: Vec<_> = app.config.hotlist.iter().map(|e| e.path.clone()).collect();
    assert_eq!(paths, std::slice::from_ref(&cwd));
    assert!(app.dialog.is_none());

    let _ = app.update(Message::Key(Action::Hotlist));
    let _ = app.update(Message::Key(Action::Enter)); // add again: no duplicate
    assert_eq!(app.config.hotlist.len(), 1);

    let _ = app.update(Message::Key(Action::Hotlist));
    assert_eq!(hotlist_items(&app).len(), 4); // entry, line, add, configure
    let _ = app.update(Message::Key(Action::Down)); // over the line onto "add"
    assert!(matches!(app.dialog, Some(Dialog::List { cursor: 2, .. })));
    let _ = app.update(Message::Key(Action::Delete)); // on the add row: nothing
    assert_eq!(app.config.hotlist.len(), 1);
    let _ = app.update(Message::Key(Action::Up));
    let _ = app.update(Message::Key(Action::Delete)); // remove the entry
    assert!(app.config.hotlist.is_empty());
    assert_eq!(hotlist_items(&app).len(), 2); // still open
    assert!(matches!(app.dialog, Some(Dialog::List { cursor: 0, .. })));
}

#[test]
fn hotlist_configure_edits_a_copy_saved_on_ok() {
    let mut app = app_with(Config::default(), State::default());
    let cwd = app.panes[0].active().panel.cwd().to_path_buf();
    let _ = app.update(Message::Key(Action::Hotlist));
    let _ = app.update(Message::Key(Action::Down)); // "configure…"
    let _ = app.update(Message::Key(Action::Enter));
    let _ = app.update(Message::Hot(HotMsg::Add));
    let _ = app.update(Message::Hot(HotMsg::Name("Home".into())));
    assert!(app.config.hotlist.is_empty()); // not until OK
    let _ = app.update(Message::DialogSubmit);
    assert!(app.dialog.is_none());
    assert_eq!(
        app.config.hotlist,
        [HotEntry {
            name: "Home".into(),
            path: cwd
        }]
    );
}

#[test]
fn hotlist_entry_opens_its_dir() {
    let config = Config {
        hotlist: vec![crate::config::HotEntry {
            name: "etc".into(),
            path: "/etc".into(),
        }],
        ..Config::default()
    };
    let mut app = app_with(config, State::default());
    let _ = app.update(Message::Key(Action::Hotlist)); // cursor on the first entry
    app.panes[0].active_mut().pending = None;
    let _ = app.update(Message::Key(Action::Enter));
    assert_eq!(pending_of(&app, 0), Some(PathBuf::from("/etc")));
}

#[test]
fn ctrl_h_flips_hidden_in_every_tab() {
    let mut app = app_with(Config::default(), State::default());
    let _ = app.update(Message::Key(Action::NewTab));
    let _ = app.update(Message::Key(Action::ToggleHidden));
    assert!(app.config.show_hidden);
    for side in 0..2 {
        assert!(
            app.panes[side]
                .items()
                .iter()
                .all(|t| t.panel.show_hidden())
        );
    }
}

#[test]
fn ctrl_w_on_last_tab_obeys_config() {
    let mut app = app_with(Config::default(), State::default());
    app.panes[0].active_mut().pending = None;
    let before = cwds(&app, 0);
    let _ = app.update(Message::Key(Action::CloseTab));
    assert_eq!(cwds(&app, 0), before); // Nothing: no change
    assert!(app.panes[0].active().pending.is_none());

    let config = Config {
        last_tab_close: LastTab::Home,
        home_dir: Some("/".into()),
        ..Config::default()
    };
    let mut app = app_with(config, State::default());
    app.panes[0].active_mut().pending = None;
    let _ = app.update(Message::Key(Action::CloseTab));
    assert!(
        app.panes[0].active().pending.is_some(),
        "should load home_dir"
    );
}

#[test]
fn config_change_of_show_hidden_applies_to_tabs() {
    let mut app = app_with(Config::default(), State::default());
    let _ = app.update(Message::Config(Box::new(Config {
        show_hidden: true,
        ..Config::default()
    })));
    assert!(app.panes[1].active().panel.show_hidden());
}

fn press(named: Named, code: Code, status: event::Status) -> Option<Message> {
    let event = cosmic::iced::Event::Keyboard(keyboard::Event::KeyPressed {
        key: Key::Named(named),
        modified_key: Key::Named(named),
        physical_key: Physical::Code(code),
        location: Location::Standard,
        modifiers: Modifiers::empty(),
        text: None,
        repeat: false,
    });
    route_event(event, status, cosmic::iced::window::Id::unique())
}

#[test]
fn viewer_keys_reach_the_app_but_not_from_a_text_field() {
    let free = route_event(
        cosmic::iced::Event::Keyboard(keyboard::Event::KeyPressed {
            key: Key::Character("n".into()),
            modified_key: Key::Character("n".into()),
            physical_key: Physical::Code(Code::KeyN),
            location: Location::Standard,
            modifiers: Modifiers::empty(),
            text: None,
            repeat: false,
        }),
        event::Status::Ignored,
        cosmic::iced::window::Id::unique(),
    );
    assert!(
        matches!(free, Some(Message::Letter(ListerKey::Next, 'n'))),
        "{free:?}"
    );
    // Typed into the search field: stays text.
    let typed = press_with(Key::Character("n".into()), Code::KeyN, Modifiers::empty());
    assert!(typed.is_none(), "{typed:?}");
    let prev = press_with(Key::Named(Named::F3), Code::F3, Modifiers::SHIFT);
    assert!(
        matches!(prev, Some(Message::ListerKey(ListerKey::FindPrev))),
        "{prev:?}"
    );
}

fn press_with(key: Key, code: Code, mods: Modifiers) -> Option<Message> {
    let event = cosmic::iced::Event::Keyboard(keyboard::Event::KeyPressed {
        key: key.clone(),
        modified_key: key,
        physical_key: Physical::Code(code),
        location: Location::Standard,
        modifiers: mods,
        text: None,
        repeat: false,
    });
    route_event(
        event,
        event::Status::Captured,
        cosmic::iced::window::Id::unique(),
    )
}

#[test]
fn regression_panel_keys_reach_the_app_while_a_field_has_focus() {
    // A focused text_input captures every key but Up/Down/Tab; F5 from quick search must still work.
    let f5 = press(Named::F5, Code::F5, event::Status::Captured);
    assert!(
        matches!(f5, Some(Message::FieldKey(Action::Copy))),
        "{f5:?}"
    );
    let pgdn = press(Named::PageDown, Code::PageDown, event::Status::Captured);
    assert!(
        matches!(pgdn, Some(Message::FieldKey(Action::PageDown))),
        "{pgdn:?}"
    );
    let ctrl_r = press_with(Key::Character("r".into()), Code::KeyR, Modifiers::CTRL);
    assert!(
        matches!(ctrl_r, Some(Message::FieldKey(Action::Reload))),
        "{ctrl_r:?}"
    );
}

#[test]
fn text_editing_keys_stay_in_the_field() {
    for (named, code) in [
        (Named::Backspace, Code::Backspace),
        (Named::Delete, Code::Delete),
        (Named::Home, Code::Home),
        (Named::End, Code::End),
    ] {
        let msg = press(named, code, event::Status::Captured);
        assert!(msg.is_none(), "{named:?}: {msg:?}");
    }
    let space = press_with(Key::Character(" ".into()), Code::Space, Modifiers::empty());
    assert!(space.is_none(), "{space:?}");
    let ctrl_a = press_with(Key::Character("a".into()), Code::KeyA, Modifiers::CTRL);
    assert!(ctrl_a.is_none(), "{ctrl_a:?}");
}

#[test]
fn field_key_acts_only_for_the_search_field() {
    let mut app = files_app(&["alpha", "beta"]);
    let _ = app.update(Message::Key(Action::QuickSearch('b')));
    let _ = app.update(Message::FieldKey(Action::Copy));
    assert!(app.search.is_none());
    assert!(matches!(app.dialog, Some(Dialog::Input { .. })));
    // Now a dialog text field has focus: its captured F-keys must not reach the panels.
    let _ = app.update(Message::FieldKey(Action::Delete));
    assert!(matches!(app.dialog, Some(Dialog::Input { .. })));
}

#[test]
fn escape_closes_dialog_even_when_text_input_captured_it() {
    // libcosmic's text_input captures Escape (to unfocus itself); the dialog must still close.
    let msg = press(Named::Escape, Code::Escape, event::Status::Captured);
    assert!(matches!(msg, Some(Message::Escape(_))), "{msg:?}");
}

#[test]
fn captured_keys_do_not_reach_the_panels() {
    let msg = press(Named::ArrowDown, Code::ArrowDown, event::Status::Captured);
    assert!(msg.is_none(), "{msg:?}");
    let msg = press(Named::ArrowDown, Code::ArrowDown, event::Status::Ignored);
    assert!(matches!(msg, Some(Message::Key(Action::Down))), "{msg:?}");
}

/// Point the active tab of `side` at `dir` and deliver its listing.
fn listed_at(app: &mut App, side: usize, dir: &Path) {
    let _ = app.load(side, dir.into(), None);
    let t = app.panes[side].active();
    let (id, generation) = (t.id, t.pending.as_ref().unwrap().0);
    let _ = app.update(Message::Listed {
        tab: id,
        generation,
        path: dir.into(),
        focus: None,
        result: Ok(listing::scan(dir, false).unwrap()),
        space: None,
    });
}

#[test]
fn viewer_plus_minus_change_its_text_size_and_save_it() {
    let (_tmp, mut app, _) = paste_setup();
    app.lister = Some(Box::new(Lister::new(0, String::new(), 0, 14)));
    let _ = app.update(Message::ListerKey(ListerKey::Zoom(true))); // still reading: nothing
    assert_eq!(app.config.lister_size, 14);
    let text = crate::lister::Loaded::read(&_tmp.path().join("src/a"), "a").unwrap();
    app.lister.as_mut().unwrap().set_loaded(Arc::new(Ok(text)));
    let _ = app.update(Message::ListerKey(ListerKey::Zoom(true)));
    let _ = app.update(Message::Key(Action::SelectGroup)); // Num+
    assert_eq!(app.config.lister_size, 16);
    assert_eq!(app.lister.as_ref().map(|l| l.size), Some(16));
    let _ = app.update(Message::Key(Action::UnselectGroup)); // Num-
    assert_eq!(app.config.lister_size, 15);
    // A picture: no text to size.
    app.lister.as_mut().unwrap().mode = shagoff_core::lister::Mode::Image;
    let _ = app.update(Message::ListerKey(ListerKey::Zoom(true)));
    assert_eq!(app.config.lister_size, 15);
}

#[test]
fn regression_closing_the_viewer_scrolls_the_lists_back() {
    // F3 replaces the panels; Esc rebuilds their lists at the top while the tabs keep
    // their offsets: both must be scrolled back (two more task units than without a
    // viewer), or the cursor at the end is out of view.
    let (_tmp, mut app, _) = paste_setup();
    let viewer = |quick: bool| {
        let mut l = Lister::new(0, String::new(), 0, 14);
        l.quick = quick.then(|| (PathBuf::new(), 0, std::time::UNIX_EPOCH));
        Box::new(l)
    };
    let id = app.window_id();
    // (Ctrl+Q?, the message): F3 closed by its button or Esc; Ctrl+Q, Tab moving it.
    let cases = [
        (false, Message::ListerClose),
        (false, Message::Escape(id)),
        (true, Message::Key(Action::SwitchPane)),
    ];
    for (quick, msg) in cases {
        let plain = app.update(msg.clone()).units();
        app.lister = Some(viewer(quick));
        let units = app.update(msg.clone()).units();
        assert_eq!(units, plain + 2, "{msg:?}");
    }
}

/// tmp/src/a (file) and tmp/dst/ (empty); pane 0 shows dst.
fn paste_setup() -> (tempfile::TempDir, App, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let (src, dst) = (tmp.path().join("src"), tmp.path().join("dst"));
    std::fs::create_dir_all(&src).unwrap();
    std::fs::create_dir_all(&dst).unwrap();
    std::fs::write(src.join("a"), "x").unwrap();
    let mut app = app_with(Config::default(), State::default());
    listed_at(&mut app, 0, &dst);
    (tmp, app, src.join("a"))
}

#[test]
fn paste_copy_starts_copy_into_active_cwd() {
    let (_tmp, mut app, a) = paste_setup();
    let _ = app.update(Message::Pasted(Some((ClipKind::Copy, vec![a]))));
    assert_eq!(app.job.as_ref().map(|j| j.kind), Some(OpKind::Copy));
}

#[test]
fn paste_cut_starts_move() {
    let (_tmp, mut app, a) = paste_setup();
    let _ = app.update(Message::Pasted(Some((ClipKind::Cut, vec![a]))));
    assert_eq!(app.job.as_ref().map(|j| j.kind), Some(OpKind::Move));
}

#[test]
fn paste_nothing_or_empty_does_nothing() {
    let (_tmp, mut app, _) = paste_setup();
    let _ = app.update(Message::Pasted(None));
    let _ = app.update(Message::Pasted(Some((ClipKind::Copy, Vec::new()))));
    assert!(app.job.is_none());
    assert!(app.msg().is_none());
}

#[test]
fn paste_into_source_dir_reports_same_file() {
    let (_tmp, mut app, a) = paste_setup();
    listed_at(&mut app, 0, a.parent().unwrap());
    let _ = app.update(Message::Pasted(Some((ClipKind::Cut, vec![a.clone()]))));
    assert!(app.job.is_none());
    assert_eq!(
        app.msg(),
        Some(fl!("plan-same-file", path = a.display().to_string()).as_str())
    );
}

#[test]
fn paste_ignored_while_dialog_open() {
    let (_tmp, mut app, a) = paste_setup();
    let _ = app.update(Message::Key(Action::Mkdir)); // opens the F7 dialog
    assert!(app.dialog.is_some());
    let _ = app.update(Message::Pasted(Some((ClipKind::Copy, vec![a]))));
    assert!(app.job.is_none());
}

#[test]
fn drop_opens_the_copy_dialog_aimed_at_the_dir_row_or_the_pane() {
    let (tmp, mut app, a) = paste_setup();
    let dst = tmp.path().join("dst");
    std::fs::create_dir(dst.join("sub")).unwrap();
    listed_at(&mut app, 0, &dst);
    let drop = |dir, mv| Message::Drop {
        side: 0,
        dir,
        paths: vec![a.clone()],
        mv,
    };
    let _ = app.update(drop(Some(dst.join("sub")), false));
    let Some(Dialog::Input { op, input, .. }) = app.dialog.take() else {
        panic!("no dialog")
    };
    assert_eq!((op, input), (InputOp::Copy, dir_input(&dst.join("sub"))));
    let _ = app.update(drop(None, true)); // Shift: move, into the pane's dir
    let Some(Dialog::Input { op, input, .. }) = app.dialog.take() else {
        panic!("no dialog")
    };
    assert_eq!((op, input), (InputOp::Move, dir_input(&dst)));
    // Dragged from the other pane: its marks go when the copy is done, as after F5.
    listed_at(&mut app, 1, &tmp.path().join("src"));
    let _ = app.update(drop(None, false));
    assert!(matches!(
        app.dialog.take(),
        Some(Dialog::Input { side: 1, .. })
    ));
    // Shift held while dropping: move, whatever action the compositor reports.
    let _ = app.update(Message::Modifiers(Modifiers::SHIFT));
    let _ = app.update(drop(None, false));
    assert!(matches!(
        app.dialog.take(),
        Some(Dialog::Input {
            op: InputOp::Move,
            ..
        })
    ));
    let _ = app.update(Message::Modifiers(Modifiers::empty()));
    // Back into the dir it came from: nothing.
    listed_at(&mut app, 0, &tmp.path().join("src"));
    let _ = app.update(drop(None, false));
    assert!(app.dialog.is_none());
    assert_eq!(app.drop_hover, None);
}

#[test]
fn colors_by_type_are_edited_on_a_copy_and_saved_on_ok() {
    use crate::colorrules::ColorMsg;
    let mut app = app_with(Config::default(), State::default());
    let _ = app.update(Message::Key(Action::ConfigureColors));
    assert!(matches!(app.dialog, Some(Dialog::Colors(_))));
    let _ = app.update(Message::Colors(ColorMsg::Add));
    assert_eq!(app.config.colors, shagoff_core::colors::defaults()); // not yet
    let _ = app.update(Message::DialogSubmit);
    assert_eq!(app.config.colors.len(), 3);
    let _ = app.update(Message::Key(Action::ConfigureColors));
    let _ = app.update(Message::Colors(ColorMsg::Delete));
    let _ = app.update(Message::DialogCancel); // Cancel: unchanged
    assert_eq!(app.config.colors.len(), 3);
}

#[test]
fn right_button_drop_asks_copy_move_or_link() {
    let (tmp, mut app, a) = paste_setup();
    let dst = tmp.path().join("dst");
    let drop = |paths: Vec<PathBuf>| Message::Drop {
        side: 0,
        dir: None,
        paths,
        mv: false,
    };
    let _ = app.update(Message::DragStart(Some(Arc::new(vec![a.clone()]))));
    // The window's own DnD motion (not a zone's, shifted by its scrollable) places the menu.
    let motion = cosmic::iced::Event::Dnd(DndEvent::Offer(
        None,
        OfferEvent::Motion { x: 100.0, y: 50.0 },
    ));
    let at = route_event(motion, event::Status::Captured, app.window_id());
    let _ = app.update(at.expect("motion is routed"));
    let _ = app.update(drop(vec![a.clone()]));
    let Some(Dialog::DropMenu { at, .. }) = &app.dialog else {
        panic!("no drop menu")
    };
    assert_eq!(*at, cosmic::iced::Point::new(100.0, 50.0));
    // ↑ from Copy wraps to Cancel; ↓ ↓ goes Copy → Move; Enter picks it.
    let _ = app.update(Message::Key(Action::Up));
    assert!(matches!(
        app.dialog,
        Some(Dialog::DropMenu { cursor: 3, .. })
    ));
    let _ = app.update(Message::Key(Action::Down));
    let _ = app.update(Message::Key(Action::Down));
    let _ = app.update(Message::Key(Action::Enter));
    assert!(matches!(
        app.dialog,
        Some(Dialog::Input {
            op: InputOp::Move,
            ..
        })
    ));
    // A hot letter picks at once (en labels in tests: &Copy &Move &Link).
    app.dialog = None;
    let _ = app.update(Message::DragStart(Some(Arc::new(vec![a.clone()]))));
    let _ = app.update(drop(vec![a.clone()]));
    let _ = app.update(Message::CmdType('L'));
    assert!(matches!(
        app.dialog,
        Some(Dialog::Input {
            op: InputOp::Link,
            ..
        })
    ));
    app.dialog = None;
    let _ = app.update(Message::DragStart(Some(Arc::new(vec![a.clone()]))));
    let _ = app.update(drop(vec![a.clone()]));
    let _ = app.update(Message::DropAs(InputOp::Link));
    let Some(Dialog::Input { op, input, .. }) = app.dialog.take() else {
        panic!("no dialog")
    };
    assert_eq!((op, input), (InputOp::Link, dir_input(&dst)));
    // Used once; a left drag (or files from another program) gets the copy dialog.
    let _ = app.update(drop(vec![a.clone()]));
    assert!(matches!(
        app.dialog.take(),
        Some(Dialog::Input {
            op: InputOp::Copy,
            ..
        })
    ));
    let _ = app.update(Message::DragStart(Some(Arc::new(vec![a.clone()]))));
    let other = tmp.path().join("src/other");
    let _ = app.update(drop(vec![other]));
    assert!(matches!(app.dialog, Some(Dialog::Input { .. })));
}

#[test]
fn drop_on_a_tab_or_drive_button_works_beside_search_results() {
    let (tmp, mut app, a) = paste_setup();
    let t = app.panes[0].active_mut();
    t.results = Some((tmp.path().to_path_buf(), Arc::new(Vec::new())));
    let drop = |dir: Option<PathBuf>| Message::Drop {
        side: 0,
        dir,
        paths: vec![a.clone()],
        mv: false,
    };
    let _ = app.update(drop(None)); // the results list itself: no one dir
    assert!(app.dialog.is_none());
    let other = tmp.path().join("dst");
    let _ = app.update(drop(Some(other.clone()))); // another tab's dir / a drive
    let Some(Dialog::Input { input, .. }) = app.dialog.take() else {
        panic!("no dialog")
    };
    assert_eq!(input, dir_input(&other));
}

#[test]
fn regression_paste_during_navigation_lands_in_target() {
    let (tmp, mut app, a) = paste_setup();
    let next = tmp.path().join("next");
    std::fs::create_dir(&next).unwrap();
    let _ = app.load(0, next.clone(), None); // scan still pending: rows show dst
    let _ = app.update(Message::Pasted(Some((ClipKind::Copy, vec![a]))));
    assert!(app.job.is_some());
    let copied = next.join("a");
    for _ in 0..200 {
        if copied.exists() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(copied.exists(), "pasted into the dir being left");
}

/// tmp/{a.txt, b.txt, c.txt}; pane 0 shows it with a.txt and b.txt marked (cursor on c.txt).
fn mr_setup() -> (tempfile::TempDir, App) {
    let tmp = tempfile::tempdir().unwrap();
    for n in ["a.txt", "b.txt", "c.txt"] {
        std::fs::write(tmp.path().join(n), n).unwrap();
    }
    let mut app = app_with(Config::default(), State::default());
    listed_at(&mut app, 0, tmp.path());
    for a in [Action::Down, Action::MarkDown, Action::MarkDown] {
        let _ = app.update(Message::Key(a));
    }
    let _ = app.update(Message::Key(Action::MultiRename));
    (tmp, app)
}

fn mr(app: &App) -> &dialogs::MultiRename {
    match &app.dialog {
        Some(Dialog::MultiRename(m)) => m,
        other => panic!("no multi-rename dialog: {:?}", other.is_some()),
    }
}

#[test]
fn ctrl_m_opens_dialog_with_marked_files() {
    let (_tmp, app) = mr_setup();
    let m = mr(&app);
    let names: Vec<&str> = m.files.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, ["a.txt", "b.txt"]);
    assert!(m.taken.contains("c.txt") && !m.taken.contains("a.txt"));
    assert_eq!(m.current, None); // cursor is on c.txt, which is not renamed
}

#[test]
fn mr_input_changes_preview() {
    let (_tmp, mut app) = mr_setup();
    let _ = app.update(Message::MrInput(dialogs::MrField::Name, "x[C]".into()));
    let _ = app.update(Message::MrInput(dialogs::MrField::Digits, "2".into()));
    let rows = mr(&app).rows(&TimeZone::UTC);
    let new: Vec<&str> = rows.iter().map(|r| r.new.as_str()).collect();
    assert_eq!(new, ["x01.txt", "x02.txt"]);
    let _ = app.update(Message::MrCase(Case::Upper));
    assert_eq!(mr(&app).rows(&TimeZone::UTC)[0].new, "X01.TXT");
}

#[test]
fn mr_token_buttons_append_and_help_toggles() {
    let (_tmp, mut app) = mr_setup();
    let _ = app.update(Message::MrInsert("_[C]"));
    assert_eq!(mr(&app).rows(&TimeZone::UTC)[1].new, "b_2.txt");
    assert!(!mr(&app).help);
    let _ = app.update(Message::MrHelp);
    assert!(mr(&app).help);
    let _ = app.update(Message::MrHelp);
    assert!(!mr(&app).help);
}

#[test]
fn mr_submit_with_problem_keeps_dialog() {
    let (_tmp, mut app) = mr_setup();
    let _ = app.update(Message::MrInput(dialogs::MrField::Name, "c".into()));
    let _ = app.update(Message::DialogSubmit);
    assert!(matches!(app.dialog, Some(Dialog::MultiRename(_))));
    assert!(app.job.is_none());
}

#[test]
fn mr_submit_starts_move() {
    let (_tmp, mut app) = mr_setup();
    let _ = app.update(Message::MrInput(dialogs::MrField::Name, "x[C]".into()));
    let _ = app.update(Message::DialogSubmit);
    assert!(app.dialog.is_none());
    assert_eq!(app.job.as_ref().map(|j| j.kind), Some(OpKind::Move));
}

#[test]
fn mr_unchanged_names_just_close() {
    let (_tmp, mut app) = mr_setup();
    let _ = app.update(Message::DialogSubmit);
    assert!(app.dialog.is_none() && app.job.is_none());
}

#[test]
fn mr_nothing_to_rename_opens_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let mut app = app_with(Config::default(), State::default());
    listed_at(&mut app, 0, tmp.path()); // only ".."
    let _ = app.update(Message::Key(Action::MultiRename));
    assert!(app.dialog.is_none());
}

#[test]
fn mr_skips_non_utf8_names() {
    use std::os::unix::ffi::OsStrExt;
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join(std::ffi::OsStr::from_bytes(b"bad\xff")), "").unwrap();
    let mut app = app_with(Config::default(), State::default());
    listed_at(&mut app, 0, tmp.path());
    let _ = app.update(Message::Key(Action::Down)); // cursor on the bad name
    let _ = app.update(Message::Key(Action::MultiRename));
    assert!(app.dialog.is_none());
}

fn pack_setup() -> (tempfile::TempDir, App) {
    let tmp = tempfile::tempdir().unwrap();
    let (l, r) = (tmp.path().join("l"), tmp.path().join("r"));
    std::fs::create_dir_all(&l).unwrap();
    std::fs::create_dir_all(&r).unwrap();
    std::fs::write(l.join("readme.txt"), "x").unwrap();
    let mut app = app_with(Config::default(), State::default());
    listed_at(&mut app, 0, &l);
    listed_at(&mut app, 1, &r);
    let _ = app.update(Message::Key(Action::Down)); // cursor on readme.txt
    (tmp, app)
}

#[test]
fn alt_f5_opens_pack_dialog_with_default_name() {
    let (tmp, mut app) = pack_setup();
    let _ = app.update(Message::Key(Action::Pack));
    let Some(Dialog::Pack(p)) = &app.dialog else {
        panic!("no pack dialog")
    };
    assert_eq!(PathBuf::from(&p.path), tmp.path().join("r/readme.zip"));
    assert_eq!(p.format, Format::Zip);
}

#[test]
fn pack_format_change_swaps_extension_and_is_remembered() {
    let (tmp, mut app) = pack_setup();
    let _ = app.update(Message::Key(Action::Pack));
    let _ = app.update(Message::PackFormat(Format::TarXz));
    let Some(Dialog::Pack(p)) = &app.dialog else {
        panic!()
    };
    assert_eq!(PathBuf::from(&p.path), tmp.path().join("r/readme.tar.xz"));
    assert_eq!(app.config.pack_format, "tar.xz");
}

#[test]
fn pack_submit_starts_job() {
    let (_tmp, mut app) = pack_setup();
    let _ = app.update(Message::Key(Action::Pack));
    let _ = app.update(Message::DialogSubmit);
    assert!(app.dialog.is_none());
    assert_eq!(app.job.as_ref().map(|j| j.kind), Some(OpKind::Pack));
}

#[test]
fn pack_password_must_be_typed_twice_alike() {
    let (_tmp, mut app) = pack_setup();
    let _ = app.update(Message::Key(Action::Pack));
    let _ = app.update(Message::PackFormat(Format::Zip));
    let _ = app.update(Message::Toggle(dialogs::Toggle::Encrypt));
    // «Шифровать» with no password: OK does nothing.
    let _ = app.update(Message::DialogSubmit);
    assert!(app.job.is_none() && matches!(app.dialog, Some(Dialog::Pack(_))));
    let _ = app.update(Message::PackPassword(false, "pw".into()));
    let _ = app.update(Message::PackPassword(true, "pq".into()));
    let _ = app.update(Message::DialogSubmit);
    assert!(app.job.is_none() && matches!(app.dialog, Some(Dialog::Pack(_))));
    let _ = app.update(Message::PackPassword(true, "pw".into()));
    let Some(Dialog::Pack(p)) = &app.dialog else {
        panic!()
    };
    assert_eq!(p.password(), Ok(Some("pw".into())));
    // tar: no encryption, the fields are not used.
    let _ = app.update(Message::PackFormat(Format::TarGz));
    let Some(Dialog::Pack(p)) = &app.dialog else {
        panic!()
    };
    assert_eq!(p.password(), Ok(None));
    let _ = app.update(Message::DialogSubmit);
    assert_eq!(app.job.as_ref().map(|j| j.kind), Some(OpKind::Pack));
}

#[test]
fn alt_f9_without_archives_does_nothing() {
    let (_tmp, mut app) = pack_setup();
    let _ = app.update(Message::Key(Action::Unpack));
    assert!(app.dialog.is_none());
}

#[test]
fn alt_f9_on_archive_opens_dialog_then_unpacks() {
    let (tmp, mut app) = pack_setup();
    std::fs::write(tmp.path().join("l/a.zip"), "").unwrap();
    listed_at(&mut app, 0, &tmp.path().join("l"));
    let _ = app.update(Message::Key(Action::Home));
    let _ = app.update(Message::Key(Action::Down)); // ".." → a.zip (sorted before readme.txt)
    let _ = app.update(Message::Key(Action::Unpack));
    assert!(matches!(&app.dialog, Some(Dialog::Unpack { archives, .. }) if archives.len() == 1));
    let _ = app.update(Message::Toggle(dialogs::Toggle::OwnDir));
    assert!(matches!(
        &app.dialog,
        Some(Dialog::Unpack { own_dir: true, .. })
    ));
    let _ = app.update(Message::DialogSubmit);
    assert_eq!(app.job.as_ref().map(|j| j.kind), Some(OpKind::Unpack));
}

/// tmp/a.zip with d/f.txt and top.txt; pane 0 lists tmp (cursor on a.zip), pane 1 lists tmp/out.
fn zip_setup() -> (tempfile::TempDir, App, PathBuf) {
    use std::io::Write;
    let tmp = tempfile::tempdir().unwrap();
    let a = tmp.path().join("a.zip");
    let mut z = zip::ZipWriter::new(std::fs::File::create(&a).unwrap());
    for n in ["d/f.txt", "top.txt"] {
        z.start_file(n, zip::write::SimpleFileOptions::default())
            .unwrap();
        z.write_all(b"x").unwrap();
    }
    z.finish().unwrap();
    std::fs::create_dir(tmp.path().join("out")).unwrap();
    let mut app = app_with(Config::default(), State::default());
    listed_at(&mut app, 0, tmp.path());
    listed_at(&mut app, 1, &tmp.path().join("out"));
    let _ = app.update(Message::Key(Action::End)); // "..", out/, a.zip
    (tmp, app, a)
}

#[test]
fn alt_enter_inside_an_archive_shows_and_changes_nothing() {
    let (_tmp, mut app, a) = zip_setup();
    listed_at(&mut app, 0, &a);
    let _ = app.update(Message::Key(Action::End)); // top.txt
    let _ = app.update(Message::Key(Action::Properties));
    let Some(Dialog::Props(p)) = &app.dialog else {
        panic!("no properties dialog");
    };
    assert_eq!(p.archive.as_deref(), Some(a.as_path()));
    assert_eq!(p.paths, [a.join("top.txt")]);
    assert!(p.facts.iter().any(|(_, v)| v == "top.txt"));
    let mode = p.mode;
    let _ = app.update(Message::PropsBit(0o100));
    let Some(Dialog::Props(p)) = &app.dialog else {
        panic!("closed");
    };
    assert_eq!((p.mode, p.touched), (mode, 0));
    let _ = app.update(Message::DialogSubmit); // Enter: just closes
    assert!(app.dialog.is_none());
}

#[test]
fn ctrl_b_inside_an_archive_lists_from_its_index() {
    let (_tmp, mut app, a) = zip_setup();
    listed_at(&mut app, 0, &a);
    let tab = app.panes[0].active().id;
    let _ = app.update(Message::Key(Action::Branch));
    let id = app.panes[0]
        .active()
        .branching
        .expect("not refused inside an archive");
    let paths = shagoff_core::archive::branch(&a, Path::new(""), false).ok();
    let _ = app.update(Message::BranchListed {
        tab,
        id,
        root: a.clone(),
        paths,
    });
    let (root, paths) = app.panes[0].active().results.clone().unwrap();
    let mut names: Vec<String> = read_listing(&root, false, Some(&paths), &AtomicBool::new(false))
        .unwrap()
        .into_iter()
        .map(|e| e.name)
        .collect();
    names.sort();
    assert_eq!(names, ["d/f.txt", "top.txt"]);
}

#[test]
fn enter_on_archive_goes_inside_and_back() {
    let (tmp, mut app, a) = zip_setup();
    let _ = app.update(Message::Key(Action::Enter));
    assert_eq!(app.panes[0].active().target(), a);
    listed_at(&mut app, 0, &a);
    let names: Vec<&str> = app.panes[0]
        .active()
        .panel
        .entries()
        .iter()
        .map(|e| e.name.as_str())
        .collect();
    assert!(
        names.contains(&"d") && names.contains(&"top.txt"),
        "{names:?}"
    );
    let _ = app.update(Message::Key(Action::Parent));
    assert_eq!(app.panes[0].active().target(), tmp.path());
}

#[test]
fn f8_inside_archive_asks_permanent_then_repacks() {
    let (_tmp, mut app, a) = zip_setup();
    listed_at(&mut app, 0, &a);
    let _ = app.update(Message::Key(Action::Down));
    let _ = app.update(Message::Key(Action::Delete)); // F8, not Shift+F8: still no trash
    assert!(matches!(
        app.dialog,
        Some(Dialog::ConfirmDelete {
            permanent: true,
            ..
        })
    ));
    let _ = app.update(Message::DialogSubmit);
    assert_eq!(app.job.as_ref().map(|j| j.kind), Some(OpKind::Repack));
}

#[test]
fn f7_and_rename_inside_archive_repack() {
    let (_tmp, mut app, a) = zip_setup();
    listed_at(&mut app, 0, &a);
    let _ = app.update(Message::Key(Action::Mkdir));
    let _ = app.update(Message::DialogInput("new".into()));
    let _ = app.update(Message::DialogSubmit);
    assert_eq!(app.job.as_ref().map(|j| j.kind), Some(OpKind::Repack));
    app.job = None;
    let _ = app.update(Message::Key(Action::End)); // top.txt
    let _ = app.update(Message::Key(Action::Rename));
    let _ = app.update(Message::DialogInput("renamed.txt".into()));
    let _ = app.update(Message::DialogSubmit);
    assert_eq!(app.job.as_ref().map(|j| j.kind), Some(OpKind::Repack));
}

#[test]
fn f6_out_of_archive_moves() {
    let (_tmp, mut app, a) = zip_setup();
    listed_at(&mut app, 0, &a);
    let _ = app.update(Message::Key(Action::End));
    let _ = app.update(Message::Key(Action::Move));
    let _ = app.update(Message::DialogSubmit);
    assert_eq!(app.job.as_ref().map(|j| j.kind), Some(OpKind::Move));
}

#[test]
fn regression_f6_of_the_archive_into_itself_is_refused() {
    let (_tmp, mut app, a) = zip_setup(); // cursor on a.zip
    listed_at(&mut app, 1, &a);
    let _ = app.update(Message::Key(Action::Move));
    let _ = app.update(Message::DialogSubmit);
    assert!(app.job.is_none());
    assert_eq!(app.status.as_ref().map(|s| s.kind), Some(StatusKind::Error));
    assert!(a.exists());
}

#[test]
fn rename_inside_archive_refuses_a_slash() {
    let (_tmp, mut app, a) = zip_setup();
    listed_at(&mut app, 0, &a);
    let _ = app.update(Message::Key(Action::End));
    let _ = app.update(Message::Key(Action::Rename));
    let _ = app.update(Message::DialogInput("d/x.txt".into()));
    let _ = app.update(Message::DialogSubmit);
    assert!(app.job.is_none());
    assert_eq!(app.msg(), Some(fl!("rename-bad-name").as_str()));
}

#[test]
fn ctrl_m_inside_archive_repacks() {
    let (_tmp, mut app, a) = zip_setup();
    listed_at(&mut app, 0, &a);
    let _ = app.update(Message::Key(Action::End));
    let _ = app.update(Message::Key(Action::MultiRename));
    let Some(Dialog::MultiRename(m)) = &mut app.dialog else {
        panic!("{:?}", app.msg())
    };
    m.rule.name = "x[N]".into();
    let _ = app.update(Message::DialogSubmit);
    assert_eq!(app.job.as_ref().map(|j| j.kind), Some(OpKind::Repack));
}

#[test]
fn edited_file_from_archive_asks_once_per_change() {
    let (tmp, mut app, a) = zip_setup();
    let file = tmp.path().join("out/top.txt");
    std::fs::write(&file, "x").unwrap();
    app.edited.push(Edited {
        file: file.clone(),
        archive: a,
        entry: "top.txt".into(),
        mtime: std::time::UNIX_EPOCH, // "changed since extraction"
    });
    let _ = app.update(Message::EditTick);
    assert!(matches!(app.dialog, Some(Dialog::UpdateArchive { .. })));
    let _ = app.update(Message::DialogCancel);
    let _ = app.update(Message::EditTick);
    assert!(app.dialog.is_none()); // the same change is not asked again
    app.edited[0].mtime = std::time::UNIX_EPOCH;
    let _ = app.update(Message::EditTick);
    let _ = app.update(Message::DialogSubmit);
    assert_eq!(app.job.as_ref().map(|j| j.kind), Some(OpKind::Repack));
}

#[test]
fn regression_edited_file_is_asked_again_when_putting_it_back_failed() {
    let (tmp, mut app, a) = zip_setup();
    let file = tmp.path().join("out/top.txt");
    std::fs::write(&file, "x").unwrap();
    app.edited.push(Edited {
        file,
        archive: a,
        entry: "top.txt".into(),
        mtime: std::time::UNIX_EPOCH,
    });
    let _ = app.update(Message::EditTick);
    let _ = app.update(Message::DialogSubmit); // «Обновить»
    let failed = Arc::new(Report::default()); // nothing completed: an error
    let _ = app.update(Message::Op(jobs::Event::Finished(failed)));
    let _ = app.update(Message::EditTick);
    assert!(matches!(app.dialog, Some(Dialog::UpdateArchive { .. })));
    // Put back this time: not asked again.
    let _ = app.update(Message::DialogSubmit);
    let done = Arc::new(Report {
        cancelled: false,
        completed: vec![PathBuf::from("a.zip")],
    });
    let _ = app.update(Message::Op(jobs::Event::Finished(done)));
    let _ = app.update(Message::EditTick);
    assert!(app.dialog.is_none());
}

#[test]
fn f5_inside_archive_starts_extract() {
    let (_tmp, mut app, a) = zip_setup();
    listed_at(&mut app, 0, &a);
    let _ = app.update(Message::Key(Action::Down));
    let _ = app.update(Message::Key(Action::Copy));
    assert!(matches!(app.dialog, Some(Dialog::Input { .. })));
    let _ = app.update(Message::DialogSubmit);
    assert_eq!(app.job.as_ref().map(|j| j.kind), Some(OpKind::Extract));
}

#[test]
fn drops_out_of_and_into_an_archive_work_as_f5() {
    let (tmp, mut app, a) = zip_setup();
    let drop = |side, paths, dir: Option<PathBuf>, mv| Message::Drop {
        side,
        dir,
        paths,
        mv,
    };
    // From the archive (pane 0 inside it) onto pane 1's list: extracted there.
    listed_at(&mut app, 0, &a);
    let _ = app.update(drop(1, vec![a.join("top.txt")], None, false));
    assert!(matches!(
        app.dialog,
        Some(Dialog::Input {
            op: InputOp::Copy,
            ..
        })
    ));
    let _ = app.update(Message::DialogSubmit);
    assert_eq!(app.job.as_ref().map(|j| j.kind), Some(OpKind::Extract));
    app.job = None;
    // From disk onto the archive's list: packed in.
    let x = tmp.path().join("out/x");
    std::fs::write(&x, "1").unwrap();
    let _ = app.update(drop(0, vec![x.clone()], None, false));
    let _ = app.update(Message::DialogSubmit);
    assert_eq!(app.job.as_ref().map(|j| j.kind), Some(OpKind::Repack));
    app.job = None;
    // Onto the archive's row in a list (its path as the dir): packed in too.
    let _ = app.update(drop(1, vec![x], Some(a.clone()), false));
    let _ = app.update(Message::DialogSubmit);
    assert_eq!(app.job.as_ref().map(|j| j.kind), Some(OpKind::Repack));
    app.job = None;
    // From the archive into a dir of it: the F5 dialog, then a cross-archive job.
    let _ = app.update(drop(0, vec![a.join("top.txt")], Some(a.join("d")), false));
    let _ = app.update(Message::DialogSubmit);
    assert_eq!(app.job.as_ref().map(|j| j.kind), Some(OpKind::Repack));
    app.job = None;
    // A right-button drop from inside it: no Link in the menu (no file on disk to link to).
    assert_eq!(
        dialogs::drop_ops(&[a.join("top.txt")]),
        [InputOp::Copy, InputOp::Move]
    );
    assert_eq!(dialogs::drop_ops(&[tmp.path().join("out/x")]).len(), 3);
}

#[test]
fn f5_into_archive_panel_repacks() {
    let (tmp, mut app, a) = zip_setup();
    std::fs::write(tmp.path().join("out/x"), "1").unwrap();
    listed_at(&mut app, 0, &tmp.path().join("out"));
    listed_at(&mut app, 1, &a);
    let _ = app.update(Message::Key(Action::Down)); // x
    let _ = app.update(Message::Key(Action::Copy));
    assert!(matches!(app.dialog, Some(Dialog::Input { .. })));
    let _ = app.update(Message::DialogSubmit);
    assert_eq!(app.job.as_ref().map(|j| j.kind), Some(OpKind::Repack));
}

#[test]
fn clipboard_out_of_and_into_an_archive_works_as_f5() {
    let (tmp, mut app, a) = zip_setup();
    listed_at(&mut app, 0, &a);
    let _ = app.update(Message::Key(Action::Down));
    // Ctrl+C / Ctrl+X inside only put the paths; no job, not read-only.
    let _ = app.update(Message::Key(Action::ClipCut));
    assert!(app.job.is_none() && app.status.is_none());
    // Pasted outside: a cut moves out of the archive (extract, then delete there).
    app.active = 1;
    let _ = app.update(Message::Pasted(Some((
        ClipKind::Cut,
        vec![a.join("top.txt")],
    ))));
    assert_eq!(app.job.as_ref().map(|j| j.kind), Some(OpKind::Move));
    app.job = None;
    // Pasted inside: packed into it.
    app.active = 0;
    let f = tmp.path().join("out/n.txt");
    std::fs::write(&f, "1").unwrap();
    let _ = app.update(Message::Pasted(Some((ClipKind::Copy, vec![f]))));
    assert_eq!(app.job.as_ref().map(|j| j.kind), Some(OpKind::Repack));
    app.job = None;
    // From an archive into the same dir of it: the same file, as on disk.
    let top = a.join("top.txt");
    let _ = app.update(Message::Pasted(Some((ClipKind::Copy, vec![top.clone()]))));
    assert!(app.job.is_none());
    let same = plan_error(&PlanError::SameFile(top.clone()));
    assert_eq!(app.msg(), Some(same.as_str()));
    // Into another dir of it: a cross-archive job.
    listed_at(&mut app, 0, &a.join("d"));
    let _ = app.update(Message::Pasted(Some((ClipKind::Cut, vec![top]))));
    assert_eq!(app.job.as_ref().map(|j| j.kind), Some(OpKind::Repack));
}

#[test]
fn regression_pack_onto_existing_archive_is_not_read_only() {
    let (tmp, mut app, _a) = zip_setup();
    std::fs::write(tmp.path().join("out/note.txt"), "1").unwrap();
    std::fs::copy(tmp.path().join("a.zip"), tmp.path().join("out/note.zip")).unwrap();
    listed_at(&mut app, 0, &tmp.path().join("out"));
    listed_at(&mut app, 1, tmp.path());
    let _ = app.update(Message::Key(Action::End)); // note.zip, note.txt sorted: put cursor on note.txt
    let _ = app.update(Message::Key(Action::Home));
    let _ = app.update(Message::Key(Action::Down));
    let _ = app.update(Message::Key(Action::Down)); // "..", note.txt, note.zip
    let _ = app.update(Message::Key(Action::Pack));
    let Some(Dialog::Pack(p)) = &mut app.dialog else {
        panic!("no pack dialog")
    };
    p.path = tmp.path().join("out/note.zip").display().to_string(); // an existing archive
    let _ = app.update(Message::DialogSubmit);
    assert_eq!(app.job.as_ref().map(|j| j.kind), Some(OpKind::Pack));
}

#[test]
fn space_on_dir_marks_it_and_its_counted_size_goes_into_totals() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir(tmp.path().join("sub")).unwrap();
    let mut app = app_with(Config::default(), State::default());
    listed_at(&mut app, 0, tmp.path());
    let _ = app.update(Message::Key(Action::Down)); // sub
    let _ = app.update(Message::Key(Action::Mark));
    let id = app.panes[0].active().id;
    let msg = Message::DirSize(id, tmp.path().into(), "sub".into(), 4096);
    let _ = app.update(msg);
    let p = &app.panes[0].active().panel;
    assert_eq!(p.marked_totals().bytes, 4096);
    assert_eq!(p.dir_size(&p.entries()[1]), Some(4096));
}

#[test]
fn regression_a_size_counted_lands_in_a_tab_moved_meanwhile() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir(tmp.path().join("sub")).unwrap();
    let mut app = app_with(Config::default(), State::default());
    listed_at(&mut app, 0, tmp.path());
    let id = app.panes[0].active().id;
    let _ = app.update(Message::Key(Action::SwapPanes)); // Ctrl+U while it counts
    assert_eq!(app.side_of(id), Some(1));
    let _ = app.update(Message::DirSize(id, tmp.path().into(), "sub".into(), 4096));
    let p = &app.panes[1].active().panel;
    let sub = p.entries().iter().find(|e| e.name == "sub").unwrap();
    assert_eq!(p.dir_size(sub), Some(4096));
}

#[test]
fn leaving_the_dir_stops_its_size_counts() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir(tmp.path().join("sub")).unwrap();
    let mut app = app_with(Config::default(), State::default());
    listed_at(&mut app, 0, tmp.path());
    let stop = app.panes[0].active().counting.clone();
    listed_at(&mut app, 0, tmp.path()); // a rescan keeps counting
    assert!(!stop.load(Ordering::Relaxed));
    listed_at(&mut app, 0, &tmp.path().join("sub"));
    assert!(stop.load(Ordering::Relaxed));
    assert!(!app.panes[0].active().counting.load(Ordering::Relaxed));
}

#[test]
fn regression_f5_right_after_entering_archive_copies_the_shown_rows() {
    let (tmp, mut app, a) = zip_setup();
    std::fs::write(tmp.path().join("plain.txt"), "1").unwrap();
    listed_at(&mut app, 0, tmp.path());
    let _ = app.update(Message::Key(Action::End)); // plain.txt
    let _ = app.update(Message::Key(Action::Mark));
    let _ = app.load(0, a, None); // entering the archive, listing not back yet
    let _ = app.update(Message::Key(Action::Copy));
    let _ = app.update(Message::DialogSubmit);
    assert_eq!(app.job.as_ref().map(|j| j.kind), Some(OpKind::Copy));
}

fn find_dialog(app: &mut App) -> &mut dialogs::Find {
    match &mut app.dialog {
        Some(Dialog::Find(f)) => f,
        _ => panic!("no find dialog"),
    }
}

#[test]
fn divider_drag_moves_the_split_and_is_kept() {
    let mut app = app_with(Config::default(), State::default());
    for side in 0..2 {
        let _ = app.update(Message::Resized(side, Size::new(500.0, 300.0)));
    }
    let w = app.window_id();
    let _ = app.update(Message::SplitAt(w, 300.0, 0.0)); // not held: nothing
    assert_eq!(app.split, 0.0);
    let _ = app.update(Message::WindowWidth(w, 1004.0)); // 1000 + the 4 px gap
    let _ = app.update(Message::SplitStart);
    let _ = app.update(Message::SplitAt(w, 500.0, 0.0));
    let _ = app.update(Message::SplitAt(w, 501.0, 0.0)); // a click's jitter: not moved
    assert_eq!(app.split, 0.0);
    let _ = app.update(Message::SplitAt(w, 302.0, 0.0)); // Classic: left edge + 2 px of the gap
    assert_eq!(app.saved.split, 0.0); // not written while held
    let _ = app.update(Message::SplitEnd);
    assert!((app.split - 0.3).abs() < 1e-4, "{}", app.split);
    let _ = app.update(Message::SplitAt(w, 900.0, 0.0)); // let go: no more moves
    assert!((app.saved.split - 0.3).abs() < 1e-4); // written to the state
    let _ = app.update(Message::SplitReset);
    assert_eq!(app.split, 0.5);
}

#[test]
fn sync_options_are_remembered_after_a_compare() {
    use shagoff_core::sync::Kind;
    let (_tmp, mut app) = results_app();
    let _ = app.update(Message::Key(Action::SyncDirs));
    let _ = app.update(Message::SyncOpt(SyncOpt::Content));
    let _ = app.update(Message::SyncOpt(SyncOpt::Recursive)); // off
    let _ = app.update(Message::SyncOpt(SyncOpt::Mirror)); // not kept
    let _ = app.update(Message::SyncCompare);
    // After the compare, not compared again: still kept.
    let _ = app.update(Message::SyncShow(Kind::Same)); // shown now
    let _ = app.update(Message::SyncShow(Kind::ToLeft)); // hidden
    let _ = app.update(Message::SyncMask("*.rs".into()));
    let _ = app.update(Message::DialogCancel);
    assert_eq!(app.saved.sync.mask, "*.rs"); // written to the state
    let mut app = app_with(Config::default(), app.saved.clone());
    let _ = app.update(Message::Key(Action::SyncDirs));
    let Some(Dialog::Sync(s)) = &app.dialog else {
        panic!("no dialog");
    };
    assert!(s.content && !s.recursive && !s.mirror && s.mask == "*.rs");
    assert_eq!(s.hide, HashSet::from([Kind::ToLeft]));
}

#[test]
fn regression_a_bad_regex_clears_the_last_results() {
    let (_tmp, mut app) = results_app();
    let _ = app.update(Message::Key(Action::FindFiles));
    let _ = app.update(Message::FindNameRegex);
    let _ = app.update(Message::FindInput(FindField::Mask, "(".into()));
    if let Some(Dialog::Find(f)) = &mut app.dialog {
        f.results.push(PathBuf::from("/a search before"));
    }
    let _ = app.update(Message::FindStart);
    let Some(Dialog::Find(f)) = &app.dialog else {
        panic!("no dialog");
    };
    assert!(f.error.is_some() && f.results.is_empty());
}

#[test]
fn find_filters_are_remembered_and_name_regex_checked() {
    let (_tmp, mut app) = results_app();
    let _ = app.update(Message::Key(Action::FindFiles));
    let _ = app.update(Message::FindNameRegex);
    let _ = app.update(Message::FindArchives);
    let _ = app.update(Message::FindInput(FindField::Mask, "(".into()));
    let _ = app.update(Message::FindInput(FindField::MinSize, "5".into()));
    let _ = app.update(Message::FindStart);
    let Some(Dialog::Find(f)) = &app.dialog else {
        panic!("no dialog");
    };
    assert!(f.error.is_some() && f.stop.is_none()); // a bad name regex does not start
    // `*` with the regex box on: any name, not an error
    let _ = app.update(Message::FindInput(FindField::Mask, "*".into()));
    let _ = app.update(Message::FindStart);
    assert!(find_dialog(&mut app).error.is_none());
    let _ = app.update(Message::FindInput(FindField::Mask, r"\.rs$".into()));
    let _ = app.update(Message::FindStart);
    app.dialog = None;
    app.save_state();
    let prefs = app.saved.find.clone();
    assert!(prefs.name_regex && prefs.archives);
    assert_eq!(
        (prefs.mask.as_str(), prefs.min_size.as_str()),
        (r"\.rs$", "5")
    );
    let mut app = app_with(Config::default(), app.saved.clone());
    let _ = app.update(Message::Key(Action::FindFiles));
    let Some(Dialog::Find(f)) = &app.dialog else {
        panic!("no dialog");
    };
    assert!(f.name_regex && f.archives && f.min_size == "5");
}

#[test]
fn alt_enter_shows_bits_and_facts_of_the_cursor_file() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().unwrap();
    let f = tmp.path().join("a.rs");
    std::fs::write(&f, "x").unwrap();
    std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o640)).unwrap();
    let mut app = app_with(Config::default(), State::default());
    listed_at(&mut app, 0, tmp.path());
    let _ = app.update(Message::Key(Action::Down));
    let _ = app.update(Message::Key(Action::Properties));
    let Some(Dialog::Props(p)) = &app.dialog else {
        panic!("no dialog");
    };
    assert_eq!((p.mode, p.mixed, p.has_dir), (0o640, 0, false));
    assert!(p.facts.iter().any(|(_, v)| v == "a.rs"));
    assert!(p.facts.iter().any(|(_, v)| v == "text/x-rust"));
    assert!(p.id > 0); // counting started
    let id = p.id;
    let _ = app.update(Message::PropsBit(0o100));
    let u = shagoff_core::props::Usage {
        bytes: 1,
        files: 1,
        dirs: 0,
    };
    let _ = app.update(Message::PropsUsage(id + 1, Some(u))); // stale
    let Some(Dialog::Props(p)) = &app.dialog else {
        panic!("no dialog");
    };
    assert_eq!((p.mode, p.usage), (0o740, None));
    let _ = app.update(Message::Key(Action::Enter)); // applies
    assert!(app.dialog.is_none());
    // on ".." there is nothing to show
    let _ = app.update(Message::Key(Action::Home));
    let _ = app.update(Message::Key(Action::Properties));
    assert!(app.dialog.is_none());
}

#[test]
fn alt_enter_changes_the_group_to_one_of_the_users() {
    use std::os::unix::fs::MetadataExt;
    let tmp = tempfile::tempdir().unwrap();
    let f = tmp.path().join("a");
    std::fs::write(&f, "x").unwrap();
    let mut app = app_with(Config::default(), State::default());
    listed_at(&mut app, 0, tmp.path());
    let _ = app.update(Message::Key(Action::Down));
    let _ = app.update(Message::Key(Action::Properties));
    let Some(Dialog::Props(p)) = &app.dialog else {
        panic!("no dialog");
    };
    let now = std::fs::metadata(&f).unwrap().gid();
    assert_eq!(p.group.map(|i| p.gids[i]), Some(now)); // the file's own shown
    let mine = shagoff_core::owners::my_groups();
    let other = (p.gids.iter()).position(|g| *g != now && mine.contains(g));
    let Some(i) = other else { return }; // a user with one group: nothing to pick
    let g = p.gids[i];
    assert!(p.new_group().is_none() && p.new_user().is_none());
    let me = std::fs::metadata(&f).unwrap().uid();
    assert_eq!(p.user.map(|i| p.uids[i]), Some(me)); // the file's owner shown
    let back = p.group;
    let _ = app.update(Message::PropsGroup(i));
    let Some(Dialog::Props(p)) = &mut app.dialog else {
        panic!("no dialog");
    };
    assert_eq!(p.new_group(), Some(g)); // what OK gives chgrp
    p.group = back; // picked the files' own again: nothing to do
    assert_eq!(p.new_group(), None);
}

#[test]
fn properties_of_files_with_different_modes() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().unwrap();
    for (n, m) in [("a.sh", 0o755), ("b.sh", 0o644)] {
        let f = tmp.path().join(n);
        std::fs::write(&f, "").unwrap();
        std::fs::set_permissions(&f, std::fs::Permissions::from_mode(m)).unwrap();
    }
    let mut app = app_with(Config::default(), State::default());
    listed_at(&mut app, 0, tmp.path());
    let _ = app.update(Message::Key(Action::SelectAll));
    let _ = app.update(Message::Key(Action::Properties));
    let Some(Dialog::Props(p)) = &app.dialog else {
        panic!("no dialog");
    };
    // set on both / differing
    assert_eq!((p.mode, p.mixed), (0o644, 0o111));
    let _ = app.update(Message::PropsBit(0o100)); // owner x: now on for both
    let Some(Dialog::Props(p)) = &app.dialog else {
        panic!("no dialog");
    };
    assert_eq!((p.mode & 0o100, p.touched), (0o100, 0o100));
}

#[test]
fn recursive_properties_start_every_bit_as_leave_as_is() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir(tmp.path().join("d")).unwrap();
    let mut app = app_with(Config::default(), State::default());
    listed_at(&mut app, 0, tmp.path());
    let _ = app.update(Message::Key(Action::Down));
    let _ = app.update(Message::Key(Action::Properties));
    let _ = app.update(Message::PropsRecursive);
    let props = |app: &App| match &app.dialog {
        Some(Dialog::Props(p)) => (p.mode, p.mixed, p.touched),
        _ => panic!("no dialog"),
    };
    assert_eq!(props(&app).1, 0o7777); // setuid, setgid, sticky too
    // group w: `?` → on even if the dir has it already (the contents may not)
    let _ = app.update(Message::PropsBit(0o020));
    let (m, _, touched) = props(&app);
    assert_eq!((m & 0o020, touched), (0o020, 0o020));
    let _ = app.update(Message::PropsBit(0o020)); // then a plain box: off
    assert_eq!(props(&app).0 & 0o020, 0);
    let _ = app.update(Message::PropsRecursive); // off: the dir's own state again
    assert_eq!(props(&app).1, 0);
}

#[test]
fn right_click_moves_the_cursor_and_keeps_marks_only_on_a_marked_row() {
    let mut app = files_app(&["a", "b", "c"]);
    app.config.right_marks = false; // the menu at once
    let _ = app.update(Message::Key(Action::Down));
    let _ = app.update(Message::Key(Action::MarkDown)); // "a" marked, cursor on "b"
    let _ = app.update(Message::RightClick(0, 1)); // on the marked "a"
    let _ = app.update(Message::RightEmpty(0)); // the list's own handler, same press
    assert!(app.ctx_entry);
    assert_eq!(app.panes[0].active().panel.marked_totals().files, 1);
    let _ = app.update(Message::RightClick(0, 3)); // unmarked "c": the menu is about it
    let _ = app.update(Message::RightEmpty(0));
    assert_eq!(cursor_name(&app), "c");
    assert_eq!(app.panes[0].active().panel.marked_totals().files, 0);
    // a press below the rows: the dir's menu
    let _ = app.update(Message::RightEmpty(1));
    assert!(!app.ctx_entry);
    assert_eq!(app.active, 1);
}

#[test]
fn right_drag_marks_the_rows_passed_as_tc() {
    let mut app = files_app(&["a", "b", "c", "d", "e"]); // .., a, b, c, d, e
    let names = |app: &App| -> Vec<String> {
        let p = &app.panes[0].active().panel;
        (p.entries().iter())
            .filter(|e| p.is_marked(e))
            .map(|e| e.name.clone())
            .collect()
    };
    let _ = app.update(Message::RightClick(0, 1)); // on "a"
    let _ = app.update(Message::RowEnter(0, 1)); // still on it: nothing yet
    assert!(names(&app).is_empty());
    let _ = app.update(Message::RowEnter(0, 2));
    let _ = app.update(Message::RowEnter(0, 4)); // a quick move skips "c": marked too
    assert_eq!(names(&app), ["a", "b", "c", "d"]);
    assert_eq!(cursor_name(&app), "d");
    let n = app.right_hold.unwrap().n;
    let _ = app.update(Message::RightHeld(n)); // held long while dragging: no menu
    assert!(!app.right_hold.unwrap().armed);
    let _ = app.update(Message::RightUp); // no toggle on release
    assert_eq!(names(&app), ["a", "b", "c", "d"]);
    // From a marked row: the drag unmarks.
    let _ = app.update(Message::RightClick(0, 3));
    let _ = app.update(Message::RowEnter(0, 2));
    let _ = app.update(Message::RightUp);
    assert_eq!(names(&app), ["a", "d"]);
    // Into the other pane: not this drag's rows.
    let _ = app.update(Message::RightClick(0, 5));
    let _ = app.update(Message::RowEnter(1, 0));
    let _ = app.update(Message::RightUp);
    assert_eq!(names(&app), ["a", "d", "e"]); // a plain right click on "e"
}

#[test]
fn right_button_marks_and_a_long_press_opens_the_menu() {
    let mut app = files_app(&["a", "b", "c"]);
    let marked = |app: &App| app.panes[0].active().panel.marked_totals().files;
    let held = |app: &App| app.right_hold.map(|h| h.n).unwrap();
    // quick click: the cursor goes there and the row is marked; again: unmarked
    let _ = app.update(Message::RightClick(0, 1));
    let _ = app.update(Message::RightEmpty(0)); // the list's own handler, same press
    let _ = app.update(Message::RightUp);
    assert_eq!((cursor_name(&app), marked(&app)), ("a".into(), 1));
    let _ = app.update(Message::RightClick(0, 3));
    let _ = app.update(Message::RightEmpty(0));
    let _ = app.update(Message::RightUp);
    assert_eq!(marked(&app), 2);
    // long press on the unmarked "b": the menu is about it, marks dropped, none added
    let _ = app.update(Message::RightClick(0, 2));
    let _ = app.update(Message::RightEmpty(0));
    let stale = held(&app) - 1;
    let _ = app.update(Message::RightHeld(stale)); // an earlier press's timer: ignored
    assert!(!app.right_hold.unwrap().armed);
    let _ = app.update(Message::RightHeld(held(&app)));
    assert!(app.ctx_entry && app.right_hold.unwrap().armed);
    assert_eq!(marked(&app), 2); // the marks go with the release, when the menu opens
    let _ = app.update(Message::RightUp);
    assert_eq!((marked(&app), app.right_hold), (0, None));
    // held, then dragged: the marks stay
    let _ = app.update(Message::Key(Action::SelectAll));
    let _ = app.update(Message::RightClick(0, 1));
    let _ = app.update(Message::RightEmpty(0));
    let _ = app.update(Message::RightHeld(held(&app)));
    let _ = app.update(Message::DragStart(None));
    let _ = app.update(Message::RightUp);
    assert_eq!(marked(&app), 3);
    let _ = app.update(Message::Key(Action::UnselectAll));
    // pressed, then the cursor moved by a key: the release marks nothing
    let _ = app.update(Message::RightClick(0, 1));
    let _ = app.update(Message::RightEmpty(0));
    let _ = app.update(Message::Key(Action::Down));
    let _ = app.update(Message::RightUp);
    assert_eq!(marked(&app), 0);
    // a right drag: no mark
    let _ = app.update(Message::RightClick(0, 1));
    let _ = app.update(Message::RightEmpty(0));
    let _ = app.update(Message::DragStart(None));
    let _ = app.update(Message::RightUp);
    assert_eq!(marked(&app), 0);
    // empty space: a quick click does nothing, a long one the dir's menu
    let _ = app.update(Message::RightEmpty(1));
    assert_eq!(app.active, 1);
    let _ = app.update(Message::RightHeld(held(&app)));
    assert!(!app.ctx_entry);
    let _ = app.update(Message::RightUp);
    assert_eq!(marked(&app), 0);
}

#[test]
fn regression_menu_open_opens_even_with_a_typed_command() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir(tmp.path().join("sub")).unwrap();
    let config = Config {
        right_marks: false,
        ..Config::default()
    };
    let mut app = app_with(config, State::default());
    listed_at(&mut app, 0, tmp.path());
    app.cmdline = "ls -l".into();
    let _ = app.update(Message::RightClick(0, 1));
    let _ = app.update(Message::OpenEntry);
    assert_eq!(app.panes[0].active().target(), tmp.path().join("sub"));
    assert_eq!(app.cmdline, "ls -l"); // not run
}

#[test]
fn regression_closing_a_popup_does_not_quit() {
    let app = files_app(&[]);
    let popup = cosmic::iced::window::Id::unique();
    assert!(app.on_close_requested(popup).is_none());
    assert!(matches!(
        app.on_close_requested(app.window_id()),
        Some(Message::Exit)
    ));
}

#[test]
fn found_inside_archives_by_name() {
    assert!(inside_archive(Path::new("/x/a.zip/docs/f.txt")));
    assert!(inside_archive(Path::new("/x/b.tar.gz/f")));
    assert!(!inside_archive(Path::new("/x/a.zip"))); // the archive itself is a file
    assert!(!inside_archive(Path::new("/x/notes.gz/f"))); // .gz is no folder
    assert!(!inside_archive(Path::new("/x/docs/f.txt")));
}

#[test]
fn alt_l_feeds_find_results_in_any_layout() {
    let alt_l = |c: &str, status| {
        route_event(
            cosmic::iced::Event::Keyboard(keyboard::Event::KeyPressed {
                key: Key::Character(c.into()),
                modified_key: Key::Character(c.into()),
                physical_key: Physical::Code(Code::KeyL),
                location: Location::Standard,
                modifiers: Modifiers::ALT,
                text: None,
                repeat: false,
            }),
            status,
            cosmic::iced::window::Id::unique(),
        )
    };
    // Russian layout too; in a field nothing else would go, in the panel quick search would.
    let m = alt_l("д", event::Status::Captured);
    assert!(matches!(m, Some(Message::FeedKey(None))), "{m:?}");
    let m = alt_l("l", event::Status::Ignored);
    assert!(
        matches!(m, Some(Message::FeedKey(Some(Action::QuickSearch('l'))))),
        "{m:?}"
    );
    let tmp = tempfile::tempdir().unwrap();
    let a = tmp.path().join("a.rs");
    std::fs::write(&a, "").unwrap();
    let mut app = app_with(Config::default(), State::default());
    listed_at(&mut app, 0, tmp.path());
    let _ = app.update(Message::Key(Action::FindFiles));
    let _ = app.update(Message::FindStart);
    let _ = app.update(Message::FeedKey(None)); // nothing found yet
    let id = find_dialog(&mut app).id;
    let _ = app.update(Message::Find(crate::find::FindEvent::Found(id, vec![a])));
    let mask = app.find.mask.clone();
    // The focused field types the letter too: harmless, the fields were kept at the start.
    let _ = app.update(Message::FindInput(
        dialogs::FindField::Mask,
        format!("{mask}l"),
    ));
    let _ = app.update(Message::FeedKey(None));
    assert!(app.dialog.is_none());
    assert_eq!(app.find.mask, mask);
    assert!(app.panes[0].active().results.is_some());
}

#[test]
fn find_results_to_panel_and_back() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir(tmp.path().join("sub")).unwrap();
    let (a, b) = (tmp.path().join("sub/a.rs"), tmp.path().join("b.rs"));
    std::fs::write(&a, "").unwrap();
    std::fs::write(&b, "").unwrap();
    let mut app = app_with(Config::default(), State::default());
    listed_at(&mut app, 0, tmp.path());
    let _ = app.update(Message::Key(Action::FindFiles));
    let _ = app.update(Message::FindStart);
    let id = find_dialog(&mut app).id;
    // More than the dialog shows: the panel gets them all.
    let many: Vec<PathBuf> = (0..dialogs::FIND_SHOWN + 5).map(|_| a.clone()).collect();
    let _ = app.update(Message::Find(crate::find::FindEvent::Found(id, many)));
    let _ = app.update(Message::Find(crate::find::FindEvent::Found(
        id,
        vec![b.clone()],
    )));
    let _ = app.update(Message::FindFeed);
    assert!(app.dialog.is_none());
    let t = app.panes[0].active();
    let (root, paths) = t.results.clone().unwrap();
    assert_eq!(root, tmp.path());
    assert_eq!(paths.len(), dialogs::FIND_SHOWN + 6);
    assert_eq!(t.pending.as_ref().map(|p| p.1.clone()), Some(root.clone()));
    let e = read_listing(&root, false, Some(&paths), &AtomicBool::new(false)).unwrap();
    assert!(e.iter().any(|e| e.name == "sub/a.rs"));
    // Backspace leaves the results for the dir that was searched.
    let _ = app.update(Message::Key(Action::Parent));
    let t = app.panes[0].active();
    assert!(t.results.is_none());
    assert_eq!(t.pending.as_ref().map(|p| p.1.clone()), Some(root));
}

/// Pane 0 shows `tmp` as search results holding `sub/a.rs`.
fn results_app() -> (tempfile::TempDir, App) {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir(tmp.path().join("sub")).unwrap();
    let a = tmp.path().join("sub/a.rs");
    std::fs::write(&a, "").unwrap();
    let mut app = app_with(Config::default(), State::default());
    listed_at(&mut app, 0, tmp.path());
    let root = tmp.path().to_path_buf();
    app.panes[0].active_mut().results = Some((root.clone(), Arc::new(vec![a.clone()])));
    let _ = app.reload(0, root.clone(), None);
    let t = app.panes[0].active();
    let (id, generation) = (t.id, t.pending.as_ref().unwrap().0);
    let _ = app.update(Message::Listed {
        tab: id,
        generation,
        path: root.clone(),
        focus: None,
        result: Ok(listing::entries(&[a], &root, &AtomicBool::new(false))),
        space: None,
    });
    (tmp, app)
}

#[test]
fn regression_feed_uses_the_dir_searched_not_the_edited_field() {
    let tmp = tempfile::tempdir().unwrap();
    let mut app = app_with(Config::default(), State::default());
    listed_at(&mut app, 0, tmp.path());
    let _ = app.update(Message::Key(Action::FindFiles));
    let _ = app.update(Message::FindStart);
    let id = find_dialog(&mut app).id;
    let f = tmp.path().join("f");
    let _ = app.update(Message::Find(crate::find::FindEvent::Found(id, vec![f])));
    let _ = app.update(Message::FindInput(FindField::Dir, "/typo".into()));
    let _ = app.update(Message::FindFeed);
    let root = app.panes[0].active().results.clone().unwrap().0;
    assert_eq!(root, tmp.path());
}

#[test]
fn regression_huge_days_does_not_panic() {
    let tmp = tempfile::tempdir().unwrap();
    let mut app = app_with(Config::default(), State::default());
    listed_at(&mut app, 0, tmp.path());
    let _ = app.update(Message::Key(Action::FindFiles));
    let _ = app.update(Message::FindInput(
        FindField::Days,
        "99999999999999999".into(),
    ));
    let _ = app.update(Message::FindStart);
    assert!(find_dialog(&mut app).stop.is_some());
    // Bounds the wrong way round: said, not silently nothing.
    let _ = app.update(Message::FindInput(FindField::MinSize, "10".into()));
    let _ = app.update(Message::FindInput(FindField::MaxSize, "5".into()));
    let _ = app.update(Message::FindStart);
    assert!(find_dialog(&mut app).error.is_some());
}

#[test]
fn results_refuse_multi_rename_and_compare_lists() {
    let (_tmp, mut app) = results_app();
    for a in [Action::MultiRename, Action::CompareLists] {
        let _ = app.update(Message::Key(a));
        assert!(app.dialog.is_none(), "{a:?}");
        assert_eq!(
            app.msg(),
            Some(fl!("results-unsupported").as_str()),
            "{a:?}"
        );
    }
}

#[test]
fn regression_going_to_the_searched_dir_leaves_results() {
    let (tmp, mut app) = results_app();
    let _ = app.update(Message::Drive(0, tmp.path().to_path_buf()));
    assert!(app.panes[0].active().results.is_none());
}

#[test]
fn find_refuses_bad_filters_and_regex() {
    let tmp = tempfile::tempdir().unwrap();
    let mut app = app_with(Config::default(), State::default());
    listed_at(&mut app, 0, tmp.path());
    let _ = app.update(Message::Key(Action::FindFiles));
    let _ = app.update(Message::FindInput(FindField::MinSize, "abc".into()));
    let _ = app.update(Message::FindStart);
    let f = find_dialog(&mut app);
    assert!(f.stop.is_none() && f.error.is_some());
    let _ = app.update(Message::FindInput(FindField::MinSize, "10".into()));
    // An attribute letter that is none of h r s x: said, not searched.
    let _ = app.update(Message::FindInput(FindField::Attrs, "hq".into()));
    let _ = app.update(Message::FindStart);
    let f = find_dialog(&mut app);
    assert!(f.stop.is_none() && f.error.is_some());
    let _ = app.update(Message::FindInput(FindField::Attrs, "x".into()));
    let _ = app.update(Message::FindInput(FindField::Text, "(".into()));
    let _ = app.update(Message::FindRegex);
    // «Искать текст» off: the field is not used, names only — no error.
    let _ = app.update(Message::FindStart);
    let f = find_dialog(&mut app);
    assert!(f.error.is_none());
    let _ = app.update(Message::FindTextOn);
    let _ = app.update(Message::FindStart);
    let f = find_dialog(&mut app);
    assert!(f.error.is_some());
    let _ = app.update(Message::FindInput(FindField::Text, "a+".into()));
    let _ = app.update(Message::FindStart);
    let f = find_dialog(&mut app);
    assert!(f.stop.is_some() && f.error.is_none());
}

#[test]
fn alt_f7_opens_find_with_panel_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let mut app = app_with(Config::default(), State::default());
    listed_at(&mut app, 0, tmp.path());
    let _ = app.update(Message::Key(Action::FindFiles));
    let f = find_dialog(&mut app);
    assert_eq!(PathBuf::from(&f.dir), tmp.path());
    assert_eq!(f.mask, "*");
}

#[test]
fn found_appends_results_and_stale_events_are_ignored() {
    let tmp = tempfile::tempdir().unwrap();
    let mut app = app_with(Config::default(), State::default());
    listed_at(&mut app, 0, tmp.path());
    let _ = app.update(Message::Key(Action::FindFiles));
    let id = find_dialog(&mut app).id;
    let _ = app.update(Message::Find(crate::find::FindEvent::Found(
        id,
        vec!["/a".into(), "/b".into()],
    )));
    let _ = app.update(Message::Find(crate::find::FindEvent::Found(
        id + 1,
        vec!["/stale".into()],
    )));
    let f = find_dialog(&mut app);
    assert_eq!(f.results, [PathBuf::from("/a"), PathBuf::from("/b")]);
    assert_eq!(f.total, 2);
}

#[test]
fn pick_goes_to_file() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir(tmp.path().join("sub")).unwrap();
    std::fs::write(tmp.path().join("sub/x.txt"), "").unwrap();
    let mut app = app_with(Config::default(), State::default());
    listed_at(&mut app, 0, tmp.path());
    let _ = app.update(Message::Key(Action::FindFiles));
    let id = find_dialog(&mut app).id;
    let _ = app.update(Message::Find(crate::find::FindEvent::Found(
        id,
        vec![tmp.path().join("sub/x.txt")],
    )));
    let _ = app.update(Message::Key(Action::Enter));
    assert!(app.dialog.is_none());
    assert_eq!(app.panes[0].active().target(), tmp.path().join("sub"));
}

#[test]
fn regression_enter_after_moving_into_results_goes_to_file() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir(tmp.path().join("sub")).unwrap();
    let mut app = app_with(Config::default(), State::default());
    listed_at(&mut app, 0, tmp.path());
    let _ = app.update(Message::Key(Action::FindFiles));
    let id = find_dialog(&mut app).id;
    let found = vec![tmp.path().join("a"), tmp.path().join("sub/b")];
    let _ = app.update(Message::Find(crate::find::FindEvent::Found(id, found)));
    let _ = app.update(Message::Key(Action::Down));
    // The mask field still has focus: Enter arrives as its submit, not as a key.
    let _ = app.update(Message::FindSubmit);
    assert!(app.dialog.is_none());
    assert_eq!(app.panes[0].active().target(), tmp.path().join("sub"));
}

#[test]
fn find_submit_after_typing_starts_a_search() {
    let tmp = tempfile::tempdir().unwrap();
    let mut app = app_with(Config::default(), State::default());
    listed_at(&mut app, 0, tmp.path());
    let _ = app.update(Message::Key(Action::FindFiles));
    let id = find_dialog(&mut app).id;
    let _ = app.update(Message::Find(crate::find::FindEvent::Found(
        id,
        vec![tmp.path().join("a")],
    )));
    let _ = app.update(Message::Key(Action::Down));
    let _ = app.update(Message::FindInput(FindField::Mask, "*.rs".into()));
    let _ = app.update(Message::FindSubmit);
    let f = find_dialog(&mut app);
    assert!(f.stop.is_some() || f.results.is_empty()); // restarted
    assert_ne!(f.id, id);
}

#[test]
fn find_settings_are_remembered() {
    let tmp = tempfile::tempdir().unwrap();
    let mut app = app_with(Config::default(), State::default());
    listed_at(&mut app, 0, tmp.path());
    let _ = app.update(Message::Key(Action::FindFiles));
    let _ = app.update(Message::FindInput(FindField::Mask, "*.ini".into()));
    let _ = app.update(Message::FindInput(FindField::Text, "port".into()));
    let _ = app.update(Message::FindCase);
    let _ = app.update(Message::FindStart);
    let _ = app.update(Message::DialogCancel);
    assert_eq!(app.saved.find.mask, "*.ini"); // written to the state
    // A new run starts from the saved state.
    let mut app = app_with(Config::default(), app.saved.clone());
    listed_at(&mut app, 0, tmp.path());
    let _ = app.update(Message::Key(Action::FindFiles));
    let f = find_dialog(&mut app);
    assert_eq!(
        (f.mask.as_str(), f.text.as_str(), f.case_sensitive),
        ("*.ini", "port", true)
    );
}

/// tmp/l: a (new), only_l; tmp/r: a (old), only_r. Pane 0 = l, pane 1 = r.
fn sync_setup() -> (tempfile::TempDir, App) {
    let tmp = tempfile::tempdir().unwrap();
    let (l, r) = (tmp.path().join("l"), tmp.path().join("r"));
    std::fs::create_dir_all(&l).unwrap();
    std::fs::create_dir_all(&r).unwrap();
    let old = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1000);
    for (p, t) in [
        (l.join("a"), None),
        (r.join("a"), Some(old)),
        (l.join("only_l"), None),
        (r.join("only_r"), None),
    ] {
        std::fs::write(&p, "x").unwrap();
        if let Some(t) = t {
            std::fs::File::options()
                .write(true)
                .open(&p)
                .unwrap()
                .set_modified(t)
                .unwrap();
        }
    }
    let mut app = app_with(Config::default(), State::default());
    listed_at(&mut app, 0, &l);
    listed_at(&mut app, 1, &r);
    (tmp, app)
}

fn marked(app: &App, side: usize) -> Vec<String> {
    let p = &app.panes[side].active().panel;
    p.entries()
        .iter()
        .filter(|e| p.is_marked(e))
        .map(|e| e.name.clone())
        .collect()
}

#[test]
fn shift_f2_marks_both_panels() {
    let (_tmp, mut app) = sync_setup();
    let _ = app.update(Message::Key(Action::CompareLists));
    assert_eq!(marked(&app, 0), ["a", "only_l"]);
    assert_eq!(marked(&app, 1), ["only_r"]);
}

fn sync_dlg(app: &mut App) -> &mut dialogs::SyncDlg {
    match &mut app.dialog {
        Some(Dialog::Sync(s)) => s,
        _ => panic!("no sync dialog"),
    }
}

fn compared(app: &mut App) {
    let s = sync_dlg(app);
    let o = s.options();
    let rows = shagoff_core::sync::compare(
        &s.left,
        &s.right,
        &o,
        &std::sync::atomic::AtomicBool::new(false),
    );
    let id = s.id;
    let _ = app.update(Message::SyncCompared(id, rows));
}

#[test]
fn ctrl_shift_s_opens_sync_and_runs() {
    let (tmp, mut app) = sync_setup();
    let _ = app.update(Message::Key(Action::SyncDirs));
    assert_eq!(sync_dlg(&mut app).left, tmp.path().join("l"));
    compared(&mut app);
    assert_eq!(sync_dlg(&mut app).rows.len(), 3);
    let _ = app.update(Message::SyncRun);
    assert!(app.dialog.is_none());
    assert_eq!(app.job.as_ref().map(|j| j.kind), Some(OpKind::Sync));
}

#[test]
fn sync_show_buttons_toggle_a_kind() {
    use shagoff_core::sync::Kind;
    let mut app = app_with(Config::default(), State::default());
    let _ = app.update(Message::Key(Action::SyncDirs));
    let hidden = |app: &App, k| match &app.dialog {
        Some(Dialog::Sync(s)) => s.hide.contains(&k),
        _ => panic!("no sync dialog"),
    };
    assert!(hidden(&app, Kind::Same) && !hidden(&app, Kind::ToRight)); // = off at first
    let _ = app.update(Message::SyncShow(Kind::ToRight));
    let _ = app.update(Message::SyncShow(Kind::Same));
    assert!(hidden(&app, Kind::ToRight) && !hidden(&app, Kind::Same));
}

#[test]
fn mirror_asks_before_deleting() {
    use shagoff_core::sync::Dir;
    let (_tmp, mut app) = sync_setup();
    let _ = app.update(Message::Key(Action::SyncDirs));
    let _ = app.update(Message::SyncOpt(SyncOpt::Mirror));
    compared(&mut app);
    let s = sync_dlg(&mut app);
    let only_r = s
        .rows
        .iter()
        .find(|r| r.rel == Path::new("only_r"))
        .unwrap();
    assert_eq!(only_r.dir, Dir::Delete);
    // First press only asks (never run here: deleting goes to the user's real trash).
    let _ = app.update(Message::SyncRun);
    assert!(app.job.is_none());
    assert!(sync_dlg(&mut app).confirm);
    // A new compare drops the confirmation.
    compared(&mut app);
    assert!(!sync_dlg(&mut app).confirm);
}

#[test]
fn regression_editing_the_mask_drops_rows_and_confirmation() {
    let (_tmp, mut app) = sync_setup();
    let _ = app.update(Message::Key(Action::SyncDirs));
    let _ = app.update(Message::SyncOpt(SyncOpt::Mirror));
    compared(&mut app);
    let _ = app.update(Message::SyncRun); // armed
    let _ = app.update(Message::SyncMask("*.rs".into()));
    let s = sync_dlg(&mut app);
    assert!(!s.confirm && s.rows.is_empty());
}

#[test]
fn sync_mask_limits_rows() {
    let (_tmp, mut app) = sync_setup();
    let _ = app.update(Message::Key(Action::SyncDirs));
    let _ = app.update(Message::SyncMask("only_*".into()));
    compared(&mut app);
    let names: Vec<String> = sync_dlg(&mut app)
        .rows
        .iter()
        .map(|r| r.rel.display().to_string())
        .collect();
    assert_eq!(names, ["only_l", "only_r"]);
}

#[test]
fn stale_compare_ignored_and_flip_cycles() {
    let (_tmp, mut app) = sync_setup();
    let _ = app.update(Message::Key(Action::SyncDirs));
    let id = sync_dlg(&mut app).id;
    let _ = app.update(Message::SyncCompared(id + 1, vec![]));
    assert!(sync_dlg(&mut app).running.is_some() || sync_dlg(&mut app).rows.is_empty());
    compared(&mut app);
    use shagoff_core::sync::Dir;
    assert_eq!(sync_dlg(&mut app).rows[0].dir, Dir::ToRight); // "a": left newer
    let _ = app.update(Message::SyncFlip(0));
    assert_eq!(sync_dlg(&mut app).rows[0].dir, Dir::ToLeft);
    let _ = app.update(Message::SyncFlip(0));
    assert_eq!(sync_dlg(&mut app).rows[0].dir, Dir::None);
    let _ = app.update(Message::SyncFlip(0));
    assert_eq!(sync_dlg(&mut app).rows[0].dir, Dir::ToRight);
}

#[test]
fn sync_with_an_archive_panel_opens_with_two_it_does_not() {
    let (_tmp, mut app, a) = zip_setup();
    listed_at(&mut app, 1, &a);
    let _ = app.update(Message::Key(Action::SyncDirs));
    assert!(matches!(app.dialog, Some(Dialog::Sync(_))));
    app.dialog = None;
    listed_at(&mut app, 0, &a.join("d"));
    let _ = app.update(Message::Key(Action::SyncDirs));
    assert!(app.dialog.is_none());
    assert!(app.msg().is_some());
}

#[test]
fn shift_f2_through_the_event_route_marks() {
    let (_tmp, mut app) = sync_setup();
    let event = cosmic::iced::Event::Keyboard(keyboard::Event::KeyPressed {
        key: Key::Named(Named::F2),
        modified_key: Key::Named(Named::F2),
        physical_key: Physical::Code(Code::F2),
        location: Location::Standard,
        modifiers: Modifiers::SHIFT,
        text: None,
        repeat: false,
    });
    let msg = route_event(
        event,
        event::Status::Ignored,
        cosmic::iced::window::Id::unique(),
    );
    assert!(
        matches!(msg, Some(Message::Key(Action::CompareLists))),
        "{msg:?}"
    );
    let _ = app.update(msg.unwrap());
    assert_eq!(marked(&app, 0), ["a", "only_l"]);
}

#[test]
fn shift_f2_on_identical_dirs_says_so() {
    let tmp = tempfile::tempdir().unwrap();
    let (l, r) = (tmp.path().join("l"), tmp.path().join("r"));
    for d in [&l, &r] {
        std::fs::create_dir_all(d).unwrap();
        std::fs::write(d.join("f"), "x").unwrap();
        let t = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1000);
        std::fs::File::options()
            .write(true)
            .open(d.join("f"))
            .unwrap()
            .set_modified(t)
            .unwrap();
    }
    let mut app = app_with(Config::default(), State::default());
    listed_at(&mut app, 0, &l);
    listed_at(&mut app, 1, &r);
    let _ = app.update(Message::Key(Action::CompareLists));
    assert!(marked(&app, 0).is_empty() && marked(&app, 1).is_empty());
    assert!(app.msg().is_some()); // "identical" note in the status line
}

fn diff_dlg(app: &mut App) -> &mut dialogs::DiffDlg {
    match &mut app.dialog {
        Some(Dialog::Diff(d)) => d,
        _ => panic!("no diff dialog"),
    }
}

#[test]
fn ctrl_shift_d_two_marked_in_one_panel() {
    let (tmp, mut app) = sync_setup(); // l: a, only_l
    let _ = app.update(Message::Key(Action::Down));
    let _ = app.update(Message::Key(Action::MarkDown));
    let _ = app.update(Message::Key(Action::MarkDown));
    let _ = app.update(Message::Key(Action::CompareFiles));
    let d = diff_dlg(&mut app);
    assert_eq!(
        (d.left.clone(), d.right.clone()),
        (tmp.path().join("l/a"), tmp.path().join("l/only_l"))
    );
}

#[test]
fn ctrl_shift_d_cursor_pair() {
    let (tmp, mut app) = sync_setup();
    let _ = app.update(Message::Key(Action::Down)); // left: a
    app.active = 1;
    let _ = app.update(Message::Key(Action::Down)); // right: a
    let _ = app.update(Message::Key(Action::CompareFiles));
    let d = diff_dlg(&mut app);
    assert_eq!(
        (d.left.clone(), d.right.clone()),
        (tmp.path().join("l/a"), tmp.path().join("r/a"))
    );
}

#[test]
fn ctrl_shift_d_without_files_says_so() {
    let (_tmp, mut app) = sync_setup(); // cursors on ".."
    let _ = app.update(Message::Key(Action::CompareFiles));
    assert!(app.dialog.is_none());
    assert!(app.msg().is_some());
}

#[test]
fn diff_next_prev_clamp_and_stale_ignored() {
    let (_tmp, mut app) = sync_setup();
    let _ = app.update(Message::Key(Action::Down));
    app.active = 1;
    let _ = app.update(Message::Key(Action::Down));
    let _ = app.update(Message::Key(Action::CompareFiles));
    let id = diff_dlg(&mut app).id;
    let t = shagoff_core::diff::rows("a\nb\nc\nd\n", "A\nb\nC\nd\n", 100);
    let out = Arc::new(Ok(shagoff_core::diff::Outcome::Text(t)));
    let _ = app.update(Message::DiffReady(id + 1, out.clone()));
    assert!(diff_dlg(&mut app).result.is_none());
    let _ = app.update(Message::DiffReady(id, out));
    let _ = app.update(Message::Key(Action::Up));
    assert_eq!(diff_dlg(&mut app).block, 0);
    let _ = app.update(Message::Key(Action::Down));
    let _ = app.update(Message::Key(Action::Down));
    assert_eq!(diff_dlg(&mut app).block, 1); // two blocks: clamped
}

#[test]
fn diff_scroll_makes_the_block_in_view_current() {
    let (_tmp, mut app) = sync_setup();
    let _ = app.update(Message::Key(Action::Down));
    app.active = 1;
    let _ = app.update(Message::Key(Action::Down));
    let _ = app.update(Message::Key(Action::CompareFiles));
    let id = diff_dlg(&mut app).id;
    let same = "s\n".repeat(40);
    let t = shagoff_core::diff::rows(&format!("x\n{same}x\n"), &format!("y\n{same}y\n"), 100);
    let _ = app.update(Message::DiffReady(
        id,
        Arc::new(Ok(shagoff_core::diff::Outcome::Text(t))),
    ));
    assert_eq!(diff_dlg(&mut app).block, 0);
    let _ = app.update(Message::DiffScrolled(0, 0.0, 30.0 * dialogs::DIFF_ROW_H));
    assert_eq!(diff_dlg(&mut app).block, 1);
}

#[test]
fn diff_sides_scroll_together() {
    let (tmp, mut app) = sync_setup();
    std::fs::write(tmp.path().join("l/a"), "x\n").unwrap();
    std::fs::write(tmp.path().join("r/a"), "y\n").unwrap();
    let _ = app.update(Message::Key(Action::Down));
    app.active = 1;
    let _ = app.update(Message::Key(Action::Down));
    let _ = app.update(Message::Key(Action::CompareFiles));
    let _ = app.update(Message::DiffScrolled(0, 120.0, 44.0)); // the left one scrolled
    let d = diff_dlg(&mut app);
    assert_eq!((d.x, d.offset), (120.0, 44.0)); // the right one is told to follow
    // ... and reports the same back: nothing more happens (no ping-pong).
    let _ = app.update(Message::DiffScrolled(1, 120.0, 44.0));
    assert_eq!(
        (diff_dlg(&mut app).x, diff_dlg(&mut app).offset),
        (120.0, 44.0)
    );
}

#[test]
fn regression_closing_the_window_warns_of_unsaved_diff_copies() {
    let (tmp, mut app) = sync_setup();
    std::fs::write(tmp.path().join("l/a"), "x\n").unwrap();
    std::fs::write(tmp.path().join("r/a"), "y\n").unwrap();
    let _ = app.update(Message::Key(Action::Down));
    app.active = 1;
    let _ = app.update(Message::Key(Action::Down));
    let _ = app.update(Message::Key(Action::CompareFiles));
    diff_dlg(&mut app).dirty = (false, true); // a block copied, not saved
    // × of the window: warns as the first Esc, the copies are not lost.
    assert!(matches!(app.on_app_exit(), Some(Message::Quit)));
    let _ = app.update(Message::Quit);
    assert!(diff_dlg(&mut app).confirm_close);
}

#[test]
fn diff_edit_a_side_then_it_is_unsaved_and_undoable() {
    let (tmp, mut app) = sync_setup();
    let (l, r) = (tmp.path().join("l/a"), tmp.path().join("r/a"));
    std::fs::write(&l, "x\n").unwrap();
    std::fs::write(&r, "y\n").unwrap();
    let _ = app.update(Message::Key(Action::Down));
    app.active = 1;
    let _ = app.update(Message::Key(Action::Down));
    let _ = app.update(Message::Key(Action::CompareFiles));
    let id = diff_dlg(&mut app).id;
    let t = shagoff_core::diff::rows("x\n", "y\n", 100);
    let _ = app.update(Message::DiffReady(id, Arc::new(Ok(Outcome::Text(t)))));
    let _ = app.update(Message::DiffEdit(1));
    assert!(diff_dlg(&mut app).editing.is_some());
    // Esc leaves the editor, not the dialog.
    let _ = app.update(Message::Escape(app.window_id()));
    assert!(diff_dlg(&mut app).editing.is_none());
    let _ = app.update(Message::DiffEdit(1));
    let typed = text_editor::Action::Edit(text_editor::Edit::Insert('z'));
    let _ = app.update(Message::DiffEditAct(typed));
    let _ = app.update(Message::DiffEditDone(true));
    let d = diff_dlg(&mut app);
    assert!(d.editing.is_none() && d.result.is_none()); // comparing the edited text
    let id = d.id;
    let t = shagoff_core::diff::rows("x\n", "zy\n", 100);
    let _ = app.update(Message::DiffReady(id, Arc::new(Ok(Outcome::Text(t)))));
    let d = diff_dlg(&mut app);
    assert_eq!((d.dirty, d.undo.len()), ((false, true), 1));
    assert_eq!(std::fs::read_to_string(&r).unwrap(), "y\n"); // not saved yet
}

#[test]
fn diff_copy_block_then_save_and_esc_asks_first() {
    let (tmp, mut app) = sync_setup();
    let (l, r) = (tmp.path().join("l/a"), tmp.path().join("r/a"));
    std::fs::write(&l, "x\nNEW\ny\n").unwrap();
    std::fs::write(&r, "x\nold\ny\n").unwrap();
    let _ = app.update(Message::Key(Action::Down));
    app.active = 1;
    let _ = app.update(Message::Key(Action::Down));
    let _ = app.update(Message::Key(Action::CompareFiles));
    let id = diff_dlg(&mut app).id;
    let t = shagoff_core::diff::rows("x\nNEW\ny\n", "x\nold\ny\n", 100);
    let _ = app.update(Message::DiffReady(id, Arc::new(Ok(Outcome::Text(t)))));
    let _ = app.update(Message::DiffCopy(true)); // block 0, left → right
    let _ = app.update(Message::DiffSave); // still copying: nothing to save yet
    assert_eq!(std::fs::read_to_string(&r).unwrap(), "x\nold\ny\n");
    // What the background copy delivers.
    let id = diff_dlg(&mut app).id;
    let t = shagoff_core::diff::rows("x\nNEW\ny\n", "x\nNEW\ny\n", 100);
    let _ = app.update(Message::DiffOpt(dialogs::DiffOpt::Case)); // ignored while copying
    assert!(!diff_dlg(&mut app).opts.ignore_case);
    let _ = app.update(Message::DiffReady(id, Arc::new(Ok(Outcome::Text(t)))));
    assert_eq!(diff_dlg(&mut app).dirty, (false, true)); // changed, unsaved
    let shown = diff_dlg(&mut app).id;
    let _ = app.update(Message::DiffReread); // would drop the unsaved copy: refused
    assert_eq!(diff_dlg(&mut app).id, shown);
    let _ = app.update(Message::DialogCancel);
    assert!(app.dialog.is_some()); // first Esc only warns
    let _ = app.update(Message::DiffSave);
    assert_eq!(std::fs::read_to_string(&r).unwrap(), "x\nNEW\ny\n");
    assert_eq!(std::fs::read_to_string(&l).unwrap(), "x\nNEW\ny\n");
    assert_eq!(diff_dlg(&mut app).dirty, (false, false));
    let _ = app.update(Message::DiffReread); // saved: reads both files again
    assert_ne!(diff_dlg(&mut app).id, shown);
    let _ = app.update(Message::DialogCancel);
    assert!(app.dialog.is_none());
}

#[test]
fn diff_undo_restores_text_and_dirty_until_saved() {
    let (tmp, mut app) = sync_setup();
    let (l, r) = (tmp.path().join("l/a"), tmp.path().join("r/a"));
    std::fs::write(&l, "x\nNEW\ny\n").unwrap();
    std::fs::write(&r, "x\nold\ny\n").unwrap();
    let _ = app.update(Message::Key(Action::Down));
    app.active = 1;
    let _ = app.update(Message::Key(Action::Down));
    let _ = app.update(Message::Key(Action::CompareFiles));
    let id = diff_dlg(&mut app).id;
    let t = shagoff_core::diff::rows("x\nNEW\ny\n", "x\nold\ny\n", 100);
    let _ = app.update(Message::DiffReady(id, Arc::new(Ok(Outcome::Text(t)))));
    let _ = app.update(Message::DiffUndo); // nothing copied yet
    assert_eq!(diff_dlg(&mut app).id, id);
    let _ = app.update(Message::DiffCopy(true));
    let id = diff_dlg(&mut app).id;
    let _ = app.update(Message::DiffUndo); // still copying: refused
    assert_eq!(diff_dlg(&mut app).id, id);
    let t = shagoff_core::diff::rows("x\nNEW\ny\n", "x\nNEW\ny\n", 100);
    let _ = app.update(Message::DiffReady(id, Arc::new(Ok(Outcome::Text(t)))));
    assert_eq!(diff_dlg(&mut app).dirty, (false, true));
    let _ = app.update(Message::DiffUndo);
    let d = diff_dlg(&mut app);
    assert_eq!(d.dirty, (false, false));
    assert!(d.undo.is_empty());
    assert!(d.result.is_none()); // re-comparing the texts from before the copy
    // Save drops the history: undoing after it would not match the disk.
    let t = shagoff_core::diff::rows("x\nNEW\ny\n", "x\nold\ny\n", 100);
    let id = diff_dlg(&mut app).id;
    let _ = app.update(Message::DiffReady(id, Arc::new(Ok(Outcome::Text(t)))));
    let _ = app.update(Message::DiffCopy(true));
    let id = diff_dlg(&mut app).id;
    let t = shagoff_core::diff::rows("x\nNEW\ny\n", "x\nNEW\ny\n", 100);
    let _ = app.update(Message::DiffReady(id, Arc::new(Ok(Outcome::Text(t)))));
    assert_eq!(diff_dlg(&mut app).undo.len(), 1);
    let _ = app.update(Message::DiffSave);
    assert!(diff_dlg(&mut app).undo.is_empty());
}

#[test]
fn regression_a_failed_diff_save_keeps_undo_and_the_close_warning() {
    use std::os::unix::fs::PermissionsExt;
    let (tmp, mut app) = sync_setup();
    let (l, r) = (tmp.path().join("l/a"), tmp.path().join("r/a"));
    std::fs::write(&l, "x\nNEW\ny\n").unwrap();
    std::fs::write(&r, "x\nold\ny\n").unwrap();
    let _ = app.update(Message::Key(Action::Down));
    app.active = 1;
    let _ = app.update(Message::Key(Action::Down));
    let _ = app.update(Message::Key(Action::CompareFiles));
    let id = diff_dlg(&mut app).id;
    let t = shagoff_core::diff::rows("x\nNEW\ny\n", "x\nold\ny\n", 100);
    let _ = app.update(Message::DiffReady(id, Arc::new(Ok(Outcome::Text(t)))));
    let _ = app.update(Message::DiffCopy(true));
    let id = diff_dlg(&mut app).id;
    let t = shagoff_core::diff::rows("x\nNEW\ny\n", "x\nNEW\ny\n", 100);
    let _ = app.update(Message::DiffReady(id, Arc::new(Ok(Outcome::Text(t)))));
    let ro = |mode| {
        let p = std::fs::Permissions::from_mode(mode);
        std::fs::set_permissions(tmp.path().join("r"), p.clone()).unwrap();
        std::fs::set_permissions(&r, p).unwrap();
    };
    ro(0o555);
    let _ = app.update(Message::DiffSave); // the right side cannot be written
    ro(0o755);
    assert_eq!(std::fs::read_to_string(&r).unwrap(), "x\nold\ny\n");
    let d = diff_dlg(&mut app);
    assert_eq!((d.dirty, d.undo.len()), ((false, true), 1)); // still undoable
    let _ = app.update(Message::DialogCancel);
    assert!(app.dialog.is_some()); // and Esc still warns first
}

#[test]
fn regression_diff_with_three_marked_does_not_fall_back_to_cursors() {
    let (_tmp, mut app) = sync_setup();
    let _ = app.update(Message::Key(Action::Down));
    let _ = app.update(Message::Key(Action::MarkDown)); // only one marked
    app.active = 1;
    let _ = app.update(Message::Key(Action::Down));
    app.active = 0;
    let _ = app.update(Message::Key(Action::CompareFiles));
    assert!(app.dialog.is_none());
    assert!(app.msg().is_some());
}

#[test]
fn regression_diff_inside_archive_explains() {
    let (_tmp, mut app, a) = zip_setup();
    listed_at(&mut app, 0, &a);
    let _ = app.update(Message::Key(Action::Down));
    let _ = app.update(Message::Key(Action::CompareFiles));
    assert!(app.dialog.is_none());
    assert_eq!(app.msg(), Some(fl!("diff-in-archive").as_str()));
}

#[test]
fn menu_item_does_what_its_key_does() {
    use cosmic::widget::menu::Action as _;
    let mut app = app_with(Config::default(), State::default());
    let _ = app.update(crate::menu::MenuAct::Key(Action::Mkdir).message());
    assert!(matches!(
        app.dialog,
        Some(Dialog::Input {
            op: InputOp::Mkdir,
            ..
        })
    ));
}

#[test]
fn window_close_goes_the_exit_way() {
    // Alt+F4 / the title bar's ×: through `Message::Exit`, as the Exit button and menu item
    // (its temp cleanup is there).
    let app = app_with(Config::default(), State::default());
    assert!(matches!(
        app.on_close_requested(app.window_id()),
        Some(Message::Exit)
    ));
}

mod drawer_tests {
    use super::*;
    use crate::config::{AppTheme, LastTab};
    use crate::drawer::{Drawer, Setting};

    fn form(app: &App) -> crate::drawer::SettingsForm {
        match &app.drawer {
            Some(Drawer::Settings(f)) => f.clone(),
            d => panic!("no settings: {d:?}"),
        }
    }

    #[test]
    fn f1_toggles_help_and_about_replaces_it() {
        let mut app = app_with(Config::default(), State::default());
        let _ = app.update(Message::Key(Action::Help));
        assert_eq!(app.drawer, Some(Drawer::Help));
        let _ = app.update(Message::Key(Action::About));
        assert_eq!(app.drawer, Some(Drawer::About));
        let _ = app.update(Message::Key(Action::About));
        assert_eq!(app.drawer, None);
    }

    #[test]
    fn donate_has_its_own_drawer() {
        let mut app = app_with(Config::default(), State::default());
        let _ = app.update(Message::Key(Action::Donate));
        assert_eq!(app.drawer, Some(Drawer::Donate));
    }

    #[test]
    fn f9_copies_names_and_says_so() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("a.txt"), "").unwrap();
        let mut app = app_with(Config::default(), State::default());
        let entries = listing::scan(d.path(), false).unwrap();
        let panel = &mut app.panes[0].active_mut().panel;
        panel.set_listing(d.path().to_path_buf(), entries, Some("a.txt"));
        let _ = app.update(Message::Key(Action::CopyNames));
        assert!(
            app.msg().is_some_and(|m| m.contains("a.txt")),
            "{:?}",
            app.msg()
        );
    }

    #[test]
    fn messages_go_to_the_window_bar_not_the_panel() {
        let mut app = app_with(Config::default(), State::default());
        let _ = app.update(Message::Key(Action::CompareLists)); // both panels empty: identical
        assert_eq!(app.status.as_ref().map(|s| s.kind), Some(StatusKind::Info));
        assert_eq!(app.panes[0].active().error, None);
        let _ = app.update(Message::Key(Action::Down));
        assert_eq!(app.status, None); // gone with the next action
    }

    #[test]
    fn mount_result_keeps_a_newer_message() {
        let mut app = app_with(Config::default(), State::default());
        app.say(StatusKind::Busy, fl!("mounting"));
        app.say(StatusKind::Info, fl!("copied", n = 3)); // F9 while mounting
        let _ = app.update(Message::Mounted(0, Ok(PathBuf::from("/media/x"))));
        assert_eq!(app.msg(), Some(fl!("copied", n = 3).as_str()));
        let _ = app.update(Message::Unmounted("/media/x".into(), Ok(())));
        assert_eq!(app.msg(), Some(fl!("copied", n = 3).as_str()));
    }

    #[test]
    fn working_message_outlives_the_next_key() {
        let mut app = app_with(Config::default(), State::default());
        app.say(StatusKind::Busy, fl!("connecting"));
        let _ = app.update(Message::Key(Action::Down));
        assert_eq!(app.msg(), Some(fl!("connecting").as_str()));
        let _ = app.update(Message::Mounted(0, Err(mount::Error::WrongPassword)));
        assert_eq!(app.status.as_ref().map(|s| s.kind), Some(StatusKind::Error));
    }

    #[test]
    fn escape_closes_the_drawer_first() {
        let mut app = app_with(Config::default(), State::default());
        app.panes[0].active_mut().panel.set_filter(Some("x".into()));
        let _ = app.update(Message::Key(Action::Help));
        let _ = app.update(Message::DialogCancel);
        assert_eq!(app.drawer, None);
        assert!(app.panes[0].active().panel.filter().is_some()); // filter kept
    }

    #[test]
    fn settings_form_shows_config() {
        let config = Config {
            viewer: vec!["code".into(), "--wait".into()],
            home_dir: Some("/srv".into()),
            ..Config::default()
        };
        let mut app = app_with(config, State::default());
        let _ = app.update(Message::Key(Action::Settings));
        let f = form(&app);
        assert_eq!(
            (f.viewer.as_str(), f.editor.as_str(), f.home.as_str()),
            ("code --wait", "cosmic-edit", "/srv")
        );
    }

    #[test]
    fn f12_runs_what_is_set_and_opens_the_settings_when_nothing_is() {
        let mut app = app_with(Config::default(), State::default());
        let _ = app.update(Message::Key(Action::F12));
        assert!(matches!(app.drawer, Some(Drawer::Settings(_))));
        let _ = app.update(Message::Key(Action::F12)); // already open: stays open
        assert!(matches!(app.drawer, Some(Drawer::Settings(_))));
        let brief = (crate::toolbar::builtins().iter())
            .position(|b| b.action == Action::ViewBrief)
            .unwrap();
        let _ = app.update(Message::Setting(Setting::F12(brief + 1)));
        assert_eq!(app.config.f12.cmd, "cm_SrcShort");
        let _ = app.update(Message::CloseDrawer);
        let _ = app.update(Message::Key(Action::F12));
        assert!(app.panes[app.active].active().brief);
        // "Own program": the internal command goes, the typed one stays
        let _ = app.update(Message::Setting(Setting::F12(0)));
        assert_eq!(app.config.f12, ToolButton::default());
        let _ = app.update(Message::Setting(Setting::F12Cmd("echo".into())));
        let _ = app.update(Message::Setting(Setting::F12Params("%N".into())));
        let _ = app.update(Message::Setting(Setting::F12(0)));
        assert_eq!(
            (app.config.f12.cmd.as_str(), app.config.f12.params.as_str()),
            ("echo", "%N")
        );
    }

    #[test]
    fn view_menu_toggles_the_divider_line_and_the_frame() {
        let mut app = app_with(Config::default(), State::default());
        let _ = app.update(Message::Key(Action::ToggleDividerLine));
        let _ = app.update(Message::Key(Action::ToggleActiveFrame));
        assert!(app.config.divider_line && app.config.active_frame);
        let _ = app.update(Message::Key(Action::ToggleDividerLine));
        assert!(!app.config.divider_line);
    }

    #[test]
    fn view_menu_hides_window_parts_and_widens_a_panel() {
        let mut app = app_with(Config::default(), State::default());
        assert!(config::Part::ALL.iter().all(|&p| app.config.shows(p)));
        for p in config::Part::ALL {
            let _ = app.update(Message::Key(Action::Vis(p)));
        }
        assert!(config::Part::ALL.iter().all(|&p| !app.config.shows(p)));
        assert!(!app.config.show_fkeys && !app.config.show_cmdline && !app.config.show_path);
        let _ = app.update(Message::Key(Action::Vis(config::Part::Path)));
        assert!(app.config.show_path);
        // 100 %: kept in the state; 50 %: both panes back, the divider in the middle.
        app.split = 0.3;
        let _ = app.update(Message::Key(Action::Wide));
        assert!(app.wide && app.saved.wide);
        let _ = app.update(Message::Key(Action::Wide));
        assert!(!app.wide);
        let _ = app.update(Message::Key(Action::Wide));
        let _ = app.update(Message::Key(Action::Split50));
        assert!(!app.wide && app.split == 0.5 && app.saved.split == 0.5);
        let back = app_with(Config::default(), app.saved.clone());
        assert!(!back.wide);
    }

    #[test]
    fn thumbnail_zoom_steps_within_its_range_and_drops_the_pictures() {
        use crate::thumbs::{SIZE, SIZE_MAX, SIZE_MIN, SIZE_STEP};
        let mut app = app_with(Config::default(), State::default());
        app.thumbs
            .insert("/x".into(), (std::time::UNIX_EPOCH, None));
        let _ = app.update(Message::Key(Action::ThumbZoomIn));
        assert_eq!(app.config.thumb_size, SIZE + SIZE_STEP);
        assert!(app.thumbs.is_empty());
        for _ in 0..20 {
            let _ = app.update(Message::Key(Action::ThumbZoomIn));
        }
        assert_eq!(app.config.thumb_size, SIZE_MAX);
        for _ in 0..20 {
            let _ = app.update(Message::Key(Action::ThumbZoomOut));
        }
        assert_eq!(app.config.thumb_size, SIZE_MIN);
        crate::thumbs::set_size(SIZE); // a global: the other tests expect the default
    }

    #[test]
    fn view_menu_stacks_the_panels_and_keeps_it() {
        let mut app = app_with(Config::default(), State::default());
        let _ = app.update(Message::Key(Action::VerticalPanels));
        assert!(app.vertical && app.saved.vertical);
        assert!(app_with(Config::default(), app.saved.clone()).vertical);
        let _ = app.update(Message::Key(Action::VerticalPanels));
        assert!(!app.vertical);
    }

    #[test]
    fn side_tree_follows_the_active_panel_and_a_click_goes_there() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        let mut app = app_with(Config::default(), State::default());
        let _ = app.update(Message::Key(Action::SideTree));
        assert!(app.side_tree.is_some() && app.saved.side_tree);
        // The tree shows the panel's dir after its read; a click on a row sends the panel there.
        let root = std::path::Path::new("/");
        let tree = &mut app.side_tree.as_mut().unwrap().tree;
        tree.fill(root, vec![dir.path().file_name().unwrap().into()]);
        let i = app.side_tree.as_ref().unwrap().tree.rows.len() - 1;
        let _ = app.update(Message::SideTreeRow(i));
        assert_eq!(
            app.panes[app.active].active().target(),
            app.side_tree.as_ref().unwrap().tree.rows[i].path
        );
        let _ = app.update(Message::Key(Action::SideTree));
        assert!(app.side_tree.is_none());
    }

    #[test]
    fn view_menu_picks_the_columns_of_the_full_view() {
        use shagoff_core::columns::Col;
        let mut app = app_with(Config::default(), State::default());
        assert_eq!(app.shown_cols().len(), 4);
        let _ = app.update(Message::Key(Action::ToggleCol(Col::Owner)));
        let _ = app.update(Message::Key(Action::ToggleCol(Col::Ext)));
        assert_eq!(
            app.shown_cols(),
            [Col::Size, Col::Date, Col::Attr, Col::Owner]
        );
        // One width a column: those without a stored one are fixed.
        assert_eq!(app.col_widths().len(), 4);
        let _ = app.update(Message::Key(Action::ToggleCol(Col::Owner)));
        assert_eq!(app.shown_cols().len(), 3);
    }

    #[test]
    fn view_modes_apply_their_columns_and_a_saved_one_joins_the_list() {
        let mut app = app_with(Config::default(), State::default());
        let n = app.config.view_modes.len();
        let _ = app.update(Message::Key(Action::ViewMode(1)));
        assert_eq!(app.shown_cols().len(), 2); // Short: size, date
        let _ = app.update(Message::Key(Action::ViewMode(n))); // not there: nothing
        assert_eq!(app.shown_cols().len(), 2);
        let _ = app.update(Message::Key(Action::SaveViewMode));
        assert!(matches!(
            &app.dialog,
            Some(Dialog::Input {
                op: InputOp::ViewMode,
                ..
            })
        ));
        if let Some(Dialog::Input { input, .. }) = &mut app.dialog {
            *input = "Mine".into();
        }
        let _ = app.update(Message::DialogSubmit);
        assert_eq!(app.config.view_modes.len(), n + 1);
        assert_eq!(app.config.view_modes[n].cols, ["size", "date"]);
    }

    #[test]
    fn set_date_changes_the_ticked_times_of_the_file_under_the_cursor() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("a.txt");
        std::fs::write(&f, "x").unwrap();
        let mut app = app_with(Config::default(), State::default());
        app.panes[0].active_mut().panel.set_listing(
            dir.path().to_path_buf(),
            shagoff_core::listing::scan(dir.path(), true).unwrap(),
            Some("a.txt"),
        );
        let _ = app.update(Message::Key(Action::SetDate));
        let _ = app.update(Message::DateText(0, "2001-09-09 01:46:40".into()));
        let _ = app.update(Message::DateText(1, "2002-01-01 00:00:00".into()));
        let _ = app.update(Message::DateChange(1, false)); // the access time stays
        let _ = app.update(Message::DialogSubmit);
        let meta = std::fs::metadata(&f).unwrap();
        let want = format::parse_date("2001-09-09 01:46:40", &app.tz).unwrap();
        assert_eq!(meta.modified().unwrap(), want);
        assert_ne!(
            meta.accessed().unwrap(),
            format::parse_date("2002-01-01 00:00:00", &app.tz).unwrap()
        );
        // Not a date: nothing is changed, an error is said.
        let _ = app.update(Message::Key(Action::SetDate));
        let _ = app.update(Message::DateText(0, "soon".into()));
        let _ = app.update(Message::DialogSubmit);
        assert_eq!(std::fs::metadata(&f).unwrap().modified().unwrap(), want);
        assert!(app.status.is_some());
    }

    #[test]
    fn transfer_and_focus_actions_pick_the_side_by_name() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "x").unwrap();
        let mut app = app_with(Config::default(), State::default());
        app.panes[1].active_mut().panel.set_listing(
            dir.path().to_path_buf(),
            shagoff_core::listing::scan(dir.path(), true).unwrap(),
            Some("a.txt"),
        );
        // Right panel active or not, «to the left» copies from the right one.
        app.active = 0;
        let _ = app.update(Message::Key(Action::TransferLeft));
        assert_eq!(app.active, 1);
        assert!(matches!(
            &app.dialog,
            Some(Dialog::Input {
                op: InputOp::Copy,
                side: 1,
                ..
            })
        ));
        let _ = app.update(Message::DialogCancel);
        let _ = app.update(Message::Key(Action::FocusLeft));
        assert_eq!(app.active, 0);
        let _ = app.update(Message::Key(Action::FocusRight));
        assert_eq!(app.active, 1);
        // «To the right» copies from the left one; nothing there: refused, the active pane kept.
        let _ = app.update(Message::Key(Action::TransferRight));
        assert_eq!(app.active, 1);
        assert!(app.dialog.is_none());
        app.active = 1;
        // Not asked for while a dialog is up.
        let _ = app.update(Message::Key(Action::FocusCmdLine));
    }

    #[test]
    fn shrink_and_exchange_selection_work_on_the_panels_marks() {
        let dir = tempfile::tempdir().unwrap();
        for n in ["a.txt", "b.rs", "c.txt"] {
            std::fs::write(dir.path().join(n), "x").unwrap();
        }
        let mut app = app_with(Config::default(), State::default());
        for s in 0..2 {
            app.panes[s].active_mut().panel.set_listing(
                dir.path().to_path_buf(),
                shagoff_core::listing::scan(dir.path(), true).unwrap(),
                None,
            );
        }
        app.panes[0].active_mut().panel.mark_all(true);
        let _ = app.update(Message::Key(Action::ShrinkSelection));
        if let Some(Dialog::Mask { input, .. }) = &mut app.dialog {
            *input = "*.txt".into();
        }
        let _ = app.update(Message::DialogSubmit);
        assert_eq!(
            app.panes[0].active().panel.marked_names(),
            ["a.txt", "c.txt"]
        );
        let _ = app.update(Message::Key(Action::ExchangeSelection));
        assert_eq!(
            app.panes[1].active().panel.marked_names(),
            ["a.txt", "c.txt"]
        );
        assert!(app.panes[0].active().panel.marked_names().is_empty());
    }

    #[test]
    fn alt_f3_opens_the_built_in_viewer_when_f3_runs_the_external_one() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "hello").unwrap();
        // F3 set to an external program: Alt+F3 is the built-in viewer, and the setting stays.
        let cfg = Config {
            internal_viewer: false,
            ..Config::default()
        };
        let mut app = app_with(cfg, State::default());
        app.panes[0].active_mut().panel.set_listing(
            dir.path().to_path_buf(),
            shagoff_core::listing::scan(dir.path(), true).unwrap(),
            Some("a.txt"),
        );
        let _ = app.update(Message::Key(Action::ViewOther));
        assert!(app.lister.is_some());
        assert!(!app.config.internal_viewer);
    }

    #[test]
    fn stacked_panes_divider_follows_the_pointer_height() {
        let mut app = app_with(Config::default(), State::default());
        let w = app.window_id();
        app.vertical = true;
        let _ = app.update(Message::SplitStart);
        let _ = app.update(Message::SplitAt(w, 0.0, 300.0)); // grabbed
        let _ = app.update(Message::SplitAt(w, 0.0, 301.0)); // jitter: not moved
        assert_eq!(app.split, 0.0);
        let _ = app.update(Message::SplitAt(w, 0.0, 340.0)); // lower: the top pane grows
        assert!(app.split > 0.5, "{}", app.split);
        let _ = app.update(Message::SplitEnd);
        let _ = app.update(Message::SplitReset);
        assert_eq!(app.split, 0.5);
    }

    #[test]
    fn show_menu_switches_sizes_between_bytes_and_kb_mb_gb() {
        let mut app = app_with(Config::default(), State::default());
        // Not chosen: as the skin has them.
        assert_eq!(
            app.config.human_sizes(),
            app.config.skin == config::Skin::Modern
        );
        let was = app.config.human_sizes();
        let _ = app.update(Message::Key(Action::ToggleHumanSizes));
        assert_eq!(app.config.human_sizes, Some(!was));
        let _ = app.update(Message::Key(Action::ToggleHumanSizes));
        assert_eq!(app.config.human_sizes, Some(was));
        app.config.human_sizes = None;
        app.config.skin = config::Skin::Modern;
        assert!(app.config.human_sizes());
    }

    #[test]
    fn settings_change_config() {
        let mut app = app_with(Config::default(), State::default());
        let _ = app.update(Message::Key(Action::Settings));
        let set = |app: &mut App, s| {
            let _ = app.update(Message::Setting(s));
        };
        set(&mut app, Setting::Show(crate::config::Part::Fkeys, false));
        set(&mut app, Setting::ShowHidden(true));
        set(&mut app, Setting::LastTabHome(true));
        set(&mut app, Setting::PackFormat(1));
        set(&mut app, Setting::Theme(2));
        set(&mut app, Setting::Language(1)); // "en": tests share the fallback loader
        set(&mut app, Setting::Viewer("code ".into()));
        set(&mut app, Setting::HomeDir("  ".into()));
        let c = &app.config;
        assert!(!c.show_fkeys && c.show_hidden);
        assert_eq!(c.last_tab_close, LastTab::Home);
        assert_eq!(c.pack_format, Format::PACK[1].ext());
        assert_eq!((c.app_theme, c.language.as_str()), (AppTheme::Dark, "en"));
        assert_eq!(c.viewer, ["code"]);
        assert_eq!(c.home_dir, None);
        assert_eq!(form(&app).viewer, "code "); // the space being typed stays
        assert!(app.panes[0].active().panel.show_hidden());
    }
}

mod mount_tests {
    use super::*;
    use shagoff_core::mount::{Error as MountError, Volume};

    fn drive(label: &str, path: &str) -> Drive {
        Drive {
            label: label.into(),
            path: path.into(),
        }
    }

    fn volume(name: &str, device: &str, mount: Option<&str>) -> Volume {
        Volume {
            name: name.into(),
            device: device.into(),
            mount: mount.map(String::from),
        }
    }

    fn list_len(app: &App) -> usize {
        match &app.dialog {
            Some(Dialog::List { items, .. }) => items.len(),
            _ => 0,
        }
    }

    #[test]
    fn drive_list_gets_unmounted_volumes_of_its_side_only() {
        let mut app = app_with(Config::default(), State::default());
        app.drives = vec![drive("/", "/"), drive("~", "/tmp")];
        let _ = app.update(Message::Key(Action::Drives(0)));
        let vols = vec![
            volume("sys", "/dev/nvme0n1p3", Some("file:///media/sys")),
            volume("STICK", "/dev/sda1", None),
        ];
        let _ = app.update(Message::Volumes(1, vols.clone())); // the other side's list
        assert_eq!(list_len(&app), 2);
        let _ = app.update(Message::Volumes(0, vols.clone()));
        assert_eq!(list_len(&app), 3); // only the unmounted one
        // Alt+F1, Esc, Alt+F1 before gio answered: two replies for one list (review).
        let _ = app.update(Message::Volumes(0, vols));
        assert_eq!(list_len(&app), 3);
        let Some(Dialog::List { items, .. }) = &app.dialog else {
            panic!("no list")
        };
        assert!(items[2].kind == Item::Mount && items[2].path == Path::new("/dev/sda1"));
    }

    #[test]
    fn unmounted_volumes_join_the_drive_list_and_a_pick_mounts() {
        let mut app = app_with(Config::default(), State::default());
        let vols = vec![
            volume("sys", "/dev/nvme0n1p3", Some("file:///media/sys")),
            volume("STICK", "/dev/sda1", None),
        ];
        let _ = app.update(Message::AllVolumes(vols));
        assert_eq!(app.volumes, [volume("STICK", "/dev/sda1", None)]);
        let _ = app.update(Message::MountVolume(0, "/dev/sda1".into()));
        assert_eq!(app.msg(), Some(fl!("mounting").as_str()));
    }

    #[test]
    fn volumes_after_list_closed_are_dropped() {
        let mut app = app_with(Config::default(), State::default());
        let _ = app.update(Message::Volumes(0, vec![volume("S", "/dev/sda1", None)]));
        assert!(app.dialog.is_none());
    }

    #[test]
    fn enter_on_volume_mounts_instead_of_going() {
        let mut app = app_with(Config::default(), State::default());
        app.drives = vec![drive("/", "/")];
        let _ = app.update(Message::Key(Action::Drives(0)));
        let _ = app.update(Message::Volumes(0, vec![volume("S", "/dev/sda1", None)]));
        app.panes[0].active_mut().pending = None;
        let _ = app.update(Message::ListPick(1));
        assert_eq!(pending_path(&app), None);
        assert_eq!(app.msg(), Some(fl!("mounting").as_str()));
    }

    #[test]
    fn mounted_goes_there_or_shows_error() {
        let mut app = app_with(Config::default(), State::default());
        app.panes[0].active_mut().pending = None;
        let _ = app.update(Message::Mounted(0, Err(MountError::WrongPassword)));
        assert_eq!(app.msg(), Some(fl!("mount-wrong-password").as_str()));
        app.say(StatusKind::Busy, fl!("connecting")); // the retry
        let _ = app.update(Message::Mounted(0, Ok(PathBuf::from("/media/x"))));
        assert_eq!(pending_path(&app), Some(PathBuf::from("/media/x")));
        assert_eq!(app.msg(), None);
    }

    #[test]
    fn ctrl_f_opens_connect_and_submit_reports_progress() {
        let mut app = app_with(Config::default(), State::default());
        let _ = app.update(Message::Key(Action::Connect));
        assert!(matches!(app.dialog, Some(Dialog::Connect { .. })));
        let _ = app.update(Message::DialogInput("sftp://nas/".into()));
        let _ = app.update(Message::ConnectPassword("pw".into()));
        let Some(Dialog::Connect { url, password, .. }) = &app.dialog else {
            panic!("no dialog")
        };
        assert_eq!((url.as_str(), password.as_str()), ("sftp://nas/", "pw"));
        let _ = app.update(Message::DialogSubmit);
        // Stays open, as in TC: the outcome is shown in it.
        let note = |app: &App| match &app.dialog {
            Some(Dialog::Connect { note, .. }) => note.clone(),
            _ => panic!("dialog closed"),
        };
        assert_eq!(note(&app), Some((false, fl!("connecting"))));
        let bad = Err(MountError::Failed("gio: no such host".into()));
        let _ = app.update(Message::Connected(0, "sftp://nas/".into(), bad));
        assert_eq!(note(&app), Some((true, "gio: no such host".into())));
        assert!(app.connecting.is_none());
    }

    #[test]
    fn connect_keep_is_off_until_ticked() {
        let mut app = app_with(Config::default(), State::default());
        let _ = app.update(Message::Key(Action::Connect));
        let keep = |app: &App| matches!(app.dialog, Some(Dialog::Connect { keep: true, .. }));
        assert!(!keep(&app));
        let _ = app.update(Message::ConnectKeep(true));
        assert!(keep(&app));
        // Connected: the dialog goes (the keyring task is never run in tests).
        let _ = app.update(Message::ConnectPassword("pw".into()));
        let _ = app.update(Message::Connected(0, "sftp://nas/".into(), Ok("/n".into())));
        assert!(app.dialog.is_none());
    }

    #[test]
    fn connect_remembers_the_address_and_esc_cancels() {
        let mut app = app_with(Config::default(), State::default());
        app.panes[0].active_mut().pending = None;
        let _ = app.update(Message::Key(Action::Connect));
        let _ = app.update(Message::DialogInput("sftp://bob:pw@nas/".into()));
        let _ = app.update(Message::DialogSubmit); // the gio task is never run in tests
        // A drive mounted meanwhile (Alt+F1) is not the address.
        let _ = app.update(Message::Mounted(0, Ok(PathBuf::from("/media/stick"))));
        assert!(app.config.connections.is_empty());
        let url = "sftp://bob:pw@nas/".to_string();
        let _ = app.update(Message::Connected(0, url, Ok(PathBuf::from("/run/gvfs/x"))));
        assert_eq!(app.config.connections, ["sftp://bob@nas/"]);
        // Saved addresses are offered; picking one fills the field.
        let _ = app.update(Message::Key(Action::Connect));
        let Some(Dialog::Connect { saved, .. }) = &app.dialog else {
            panic!("no dialog")
        };
        assert_eq!(saved, &["sftp://bob@nas/"]);
        let _ = app.update(Message::ConnectPick("smb://x/".into()));
        assert!(matches!(&app.dialog, Some(Dialog::Connect { url, .. }) if url == "smb://x/"));
        // Esc while connecting stops it.
        let _ = app.update(Message::DialogSubmit);
        let flag = app.connecting.as_ref().unwrap().1.clone();
        // A second submit meanwhile does nothing (its flag would replace this one).
        let _ = app.update(Message::DialogSubmit);
        assert!(Arc::ptr_eq(&flag, &app.connecting.as_ref().unwrap().1));
        let _ = app.update(Message::DialogCancel);
        assert!(flag.load(Ordering::Relaxed));
        let gone = Err(MountError::Cancelled);
        let _ = app.update(Message::Connected(0, "smb://x/".into(), gone));
        assert_eq!(app.msg(), Some(fl!("connect-cancelled").as_str()));
        assert_eq!(app.config.connections, ["sftp://bob@nas/"]); // not remembered
        assert!(app.connecting.is_none());
    }

    #[test]
    fn connect_forget_and_browse_results() {
        let config = Config {
            connections: vec!["smb://a/".into(), "smb://b/".into()],
            ..Config::default()
        };
        let mut app = app_with(config, State::default());
        let _ = app.update(Message::Key(Action::Connect));
        let _ = app.update(Message::ConnectForget("smb://a/".into()));
        assert_eq!(app.config.connections, ["smb://b/"]);
        let found = vec![("nas".to_string(), "sftp://nas.local/".to_string())];
        let _ = app.update(Message::ConnectBrowse(None));
        let Some(Dialog::Connect {
            browsing: Some(stop),
            ..
        }) = &app.dialog
        else {
            panic!("not browsing")
        };
        let _ = app.update(Message::Browsed(stop.clone(), Ok(found.clone())));
        let Some(Dialog::Connect {
            saved, found: f, ..
        }) = &app.dialog
        else {
            panic!("no dialog")
        };
        assert_eq!(
            (saved.as_slice(), f),
            (&["smb://b/".to_string()][..], &found)
        );
    }

    #[test]
    fn browse_stops_by_its_button_or_by_closing_the_dialog() {
        let mut app = app_with(Config::default(), State::default());
        let stop = |app: &App| match &app.dialog {
            Some(Dialog::Connect { browsing, .. }) => browsing.clone(),
            _ => panic!("no dialog"),
        };
        let _ = app.update(Message::Key(Action::Connect));
        let _ = app.update(Message::ConnectBrowse(None));
        let first = stop(&app).expect("browsing");
        let _ = app.update(Message::ConnectBrowse(None)); // pressed again: stop
        assert!(first.load(Ordering::Relaxed) && stop(&app).is_none());
        // Its "cancelled" lands later: no error shown.
        let _ = app.update(Message::Browsed(first, Err(MountError::Cancelled)));
        assert!(matches!(
            &app.dialog,
            Some(Dialog::Connect { note: None, .. })
        ));
        let _ = app.update(Message::ConnectBrowse(None));
        let second = stop(&app).expect("browsing");
        let _ = app.update(Message::DialogCancel);
        assert!(second.load(Ordering::Relaxed));
    }

    #[test]
    fn browse_goes_inside_and_drops_a_stale_result() {
        let mut app = app_with(Config::default(), State::default());
        let place = |app: &App| match &app.dialog {
            Some(Dialog::Connect {
                at,
                found,
                browsing,
                ..
            }) => (at.clone(), found.clone(), browsing.clone()),
            _ => panic!("no dialog"),
        };
        let _ = app.update(Message::Key(Action::Connect));
        let _ = app.update(Message::ConnectBrowse(None));
        let net = vec![("Windows Network".to_string(), "smb:///".to_string())];
        let stop = place(&app).2.expect("browsing");
        let _ = app.update(Message::Browsed(stop, Ok(net.clone())));
        assert_eq!(place(&app).1, net);
        // Deeper: the old list goes at once; deeper again before it answers.
        let _ = app.update(Message::ConnectBrowse(Some("smb:///".into())));
        let first = place(&app).2.expect("browsing");
        let _ = app.update(Message::ConnectBrowse(Some("smb://WG/".into())));
        let (at, found, second) = place(&app);
        assert_eq!((at.as_str(), found.len()), ("smb://WG/", 0));
        assert!(first.load(Ordering::Relaxed));
        // The first one finishing late anyway is not shown, the second still stoppable.
        let wg = vec![("WG".to_string(), "smb://WG/".to_string())];
        let _ = app.update(Message::Browsed(first, Ok(wg)));
        assert!(place(&app).1.is_empty() && place(&app).2.is_some());
        let nas = vec![("NAS".to_string(), "smb://nas/".to_string())];
        let _ = app.update(Message::Browsed(second.expect("browsing"), Ok(nas.clone())));
        let (_, found, browsing) = place(&app);
        assert_eq!((found, browsing.is_none()), (nas, true));
    }

    #[test]
    fn disconnect_refuses_root_home_and_fixed_partitions() {
        let mut app = app_with(Config::default(), State::default());
        let cwd = app.panes[0].active().panel.cwd().to_path_buf();
        app.drives = vec![drive("/", "/"), drive("~", cwd.to_str().unwrap())];
        let _ = app.update(Message::Key(Action::Disconnect));
        assert_eq!(app.msg(), None);
        // A partition outside /media (like /home): udisks would ask for the admin password.
        app.drives = vec![
            drive("/", "/"),
            drive("~", "/x"),
            drive("tmp", cwd.to_str().unwrap()),
        ];
        let _ = app.update(Message::Key(Action::Disconnect));
        assert_eq!(app.msg(), None);
    }

    #[test]
    fn unmounted_sends_panes_inside_home() {
        let mut app = app_with(Config::default(), State::default());
        let cwd = app.panes[0].active().panel.cwd().to_path_buf();
        app.panes[0].active_mut().pending = None;
        let _ = app.update(Message::Unmounted("/nonexistent".into(), Ok(())));
        assert_eq!(pending_path(&app), None);
        let _ = app.update(Message::Unmounted(cwd, Ok(())));
        assert_eq!(pending_path(&app), Some(app.home.clone()));
        let failed = MountError::Failed("target is busy".into());
        let _ = app.update(Message::Unmounted("/media/x".into(), Err(failed)));
        assert_eq!(app.msg(), Some("target is busy"));
    }
}

mod cmdline {
    use super::*;

    /// A key no widget took (the panel has focus).
    fn typed(key: &str, code: Code, mods: Modifiers) -> Option<Message> {
        let event = cosmic::iced::Event::Keyboard(keyboard::Event::KeyPressed {
            key: Key::Character(key.into()),
            modified_key: Key::Character(key.into()),
            physical_key: Physical::Code(code),
            location: Location::Standard,
            modifiers: mods,
            text: None,
            repeat: false,
        });
        route_event(
            event,
            event::Status::Ignored,
            cosmic::iced::window::Id::unique(),
        )
    }

    fn at(dir: &Path) -> App {
        let mut app = app_with(Config::default(), State::default());
        listed_at(&mut app, 0, dir);
        app
    }

    #[test]
    fn letters_go_to_the_command_line_space_only_when_it_has_text() {
        assert!(matches!(
            typed("l", Code::KeyL, Modifiers::empty()),
            Some(Message::CmdType('l'))
        ));
        assert!(matches!(
            typed("L", Code::KeyL, Modifiers::SHIFT),
            Some(Message::CmdType('L'))
        ));
        assert!(typed("y", Code::KeyY, Modifiers::CTRL).is_none());
        let tmp = tempfile::tempdir().unwrap();
        let mut app = at(tmp.path());
        let _ = app.update(Message::Key(Action::Mark)); // empty: Space marks
        assert_eq!(app.cmdline, "");
        let _ = app.update(Message::CmdType('l'));
        let _ = app.update(Message::Key(Action::Mark));
        let _ = app.update(Message::CmdType('s'));
        assert_eq!(app.cmdline, "l s");
    }

    #[test]
    fn hidden_command_line_takes_nothing() {
        let config = Config {
            show_cmdline: false,
            ..Config::default()
        };
        let mut app = app_with(config, State::default());
        let _ = app.update(Message::CmdType('l'));
        let _ = app.update(Message::Key(Action::CmdCwd));
        assert_eq!(app.cmdline, "");
    }

    #[test]
    fn cd_changes_the_panel_dir_and_is_remembered() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("sub")).unwrap();
        let mut app = at(tmp.path());
        let _ = app.update(Message::CmdInput("cd sub".into()));
        let _ = app.update(Message::CmdEnter);
        assert_eq!(app.panes[0].active().target(), tmp.path().join("sub"));
        assert_eq!(app.cmdline, "");
        assert_eq!(app.commands, ["cd sub"]);
    }

    #[test]
    fn enter_in_the_panel_runs_a_typed_line() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("sub")).unwrap();
        let mut app = at(tmp.path());
        let _ = app.update(Message::CmdInput("cd sub".into()));
        let _ = app.update(Message::Key(Action::Enter));
        assert_eq!(app.panes[0].active().target(), tmp.path().join("sub"));
    }

    #[test]
    fn a_command_runs_in_the_panel_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = at(tmp.path());
        let _ = app.update(Message::CmdInput("touch made".into()));
        let _ = app.update(Message::CmdEnter);
        let made = tmp.path().join("made");
        for _ in 0..100 {
            if made.exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(made.exists());
        assert_eq!(app.msg(), None);
    }

    #[test]
    fn ctrl_enter_is_for_the_name_not_a_submit() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("a b.txt"), "").unwrap();
        let mut app = at(tmp.path());
        let _ = app.update(Message::Key(Action::Down)); // off ".."
        let _ = app.update(Message::CmdInput("cat".into()));
        let _ = app.update(Message::Modifiers(Modifiers::CTRL));
        let _ = app.update(Message::CmdEnter); // the field reports Ctrl+Enter as Enter
        assert_eq!(app.cmdline, "cat 'a b.txt'");
        let _ = app.update(Message::Modifiers(Modifiers::CTRL | Modifiers::SHIFT));
        let _ = app.update(Message::CmdEnter);
        let full = tmp.path().join("a b.txt").display().to_string();
        assert_eq!(app.cmdline, format!("cat 'a b.txt' '{full}'"));
    }

    #[test]
    fn parent_row_inserts_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = at(tmp.path());
        let _ = app.update(Message::Key(Action::CmdName));
        assert_eq!(app.cmdline, "");
    }

    #[test]
    fn names_not_in_utf8_go_in_as_a_glob() {
        use std::os::unix::ffi::OsStrExt;
        let tmp = tempfile::tempdir().unwrap();
        let bad = std::ffi::OsStr::from_bytes(b"caf\xe9");
        std::fs::write(tmp.path().join(bad), "").unwrap();
        let mut app = app_with(Config::default(), State::default());
        listed_at(&mut app, 0, tmp.path());
        let panel = &mut app.panes[0].active_mut().panel;
        let i = (panel.entries().iter())
            .position(|e| e.os_name == bad)
            .unwrap();
        panel.set_cursor(i);
        let _ = app.update(Message::Key(Action::CmdName));
        assert_eq!(app.cmdline, "caf?");
        // The same pattern fits "cafe" too (by characters): not inserted.
        app.cmdline.clear();
        std::fs::write(tmp.path().join("café"), "").unwrap();
        listed_at(&mut app, 0, tmp.path());
        let panel = &mut app.panes[0].active_mut().panel;
        let i = (panel.entries().iter())
            .position(|e| e.os_name == bad)
            .unwrap();
        panel.set_cursor(i);
        let _ = app.update(Message::Key(Action::CmdName));
        assert_eq!(app.cmdline, "");
        assert_eq!(app.msg(), Some(fl!("cmd-name-ambiguous").as_str()));
    }

    #[test]
    fn ctrl_e_and_alt_f8_bring_back_commands() {
        let tmp = tempfile::tempdir().unwrap();
        let state = State {
            commands: vec!["make".into(), "ls".into()],
            ..State::default()
        };
        let mut app = app_with(Config::default(), state);
        listed_at(&mut app, 0, tmp.path());
        let _ = app.update(Message::Key(Action::CmdPrevious));
        assert_eq!(app.cmdline, "make");
        let _ = app.update(Message::FieldKey(Action::CmdPrevious));
        assert_eq!(app.cmdline, "ls");
        // Ctrl+X in the line: the next command, at the newest it stays.
        let _ = app.update(Message::FieldKey(Action::ClipCut));
        assert_eq!(app.cmdline, "make");
        let _ = app.update(Message::FieldKey(Action::ClipCut));
        assert_eq!(app.cmdline, "make");
        // A typed line stays; the panel's Ctrl+X / menu «Cut» never step the history.
        app.cmdline = "mak".into();
        let _ = app.update(Message::FieldKey(Action::ClipCut));
        assert_eq!(app.cmdline, "mak");
        app.cmdline = "ls".into();
        let _ = app.update(Message::Key(Action::ClipCut));
        assert_eq!(app.cmdline, "ls");
        let _ = app.update(Message::FieldKey(Action::CmdHistory));
        let _ = app.update(Message::Key(Action::Down));
        let _ = app.update(Message::Key(Action::Enter));
        assert!(app.dialog.is_none());
        assert_eq!(app.cmdline, "ls");
        let _ = app.update(Message::ListPick(0));
        assert_eq!(app.cmdline, "ls"); // no dialog: nothing picked
    }

    #[test]
    fn regression_viewer_letters_start_the_line_when_no_viewer() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = at(tmp.path());
        for (k, code) in [
            ("n", Code::KeyN),
            ("т", Code::KeyN),
            ("p", Code::KeyP),
            ("1", Code::Digit1),
        ] {
            if let Some(m) = typed(k, code, Modifiers::empty()) {
                let _ = app.update(m);
            }
        }
        assert_eq!(app.cmdline, "nтp1");
    }

    #[test]
    fn regression_double_click_and_quick_search_open_not_run() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("sub")).unwrap();
        std::fs::create_dir(tmp.path().join("sub/in")).unwrap();
        let mut app = at(tmp.path());
        let _ = app.update(Message::CmdInput("cd /".into()));
        let _ = app.update(Message::DoubleClick(0, 1));
        assert_eq!(app.panes[0].active().target(), tmp.path().join("sub"));
        listed_at(&mut app, 0, &tmp.path().join("sub"));
        let _ = app.update(Message::Key(Action::QuickSearch('i')));
        let _ = app.update(Message::SearchSubmit);
        assert_eq!(app.panes[0].active().target(), tmp.path().join("sub/in"));
        assert_eq!(app.cmdline, "cd /");
    }

    #[test]
    fn regression_terminal_line_with_a_comment_keeps_the_shell() {
        let argv = shagoff_core::cmdline::argv("ls # x", Path::new("/d"), Some(&[]));
        assert!(argv[4].ends_with("\nexec \"${SHELL:-sh}\""), "{argv:?}");
    }

    #[test]
    fn shift_space_does_not_start_the_line_with_a_space() {
        let event = cosmic::iced::Event::Keyboard(keyboard::Event::KeyPressed {
            key: Key::Character(" ".into()),
            modified_key: Key::Character(" ".into()),
            physical_key: Physical::Code(Code::Space),
            location: Location::Standard,
            modifiers: Modifiers::SHIFT,
            text: None,
            repeat: false,
        });
        let m = route_event(
            event,
            event::Status::Ignored,
            cosmic::iced::window::Id::unique(),
        );
        let mut app = app_with(Config::default(), State::default());
        if let Some(m) = m {
            let _ = app.update(m);
        }
        assert_eq!(app.cmdline, "");
    }

    #[test]
    fn escape_with_a_hidden_line_clears_the_filter() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = at(tmp.path());
        let _ = app.update(Message::CmdInput("x".into()));
        app.config.show_cmdline = false;
        app.panes[0].active_mut().panel.set_filter(Some("a".into()));
        let _ = app.update(Message::DialogCancel);
        assert_eq!(app.panes[0].active().panel.filter(), None);
    }

    #[test]
    fn escape_clears_the_line() {
        let mut app = app_with(Config::default(), State::default());
        let _ = app.update(Message::CmdInput("rm -rf x".into()));
        let _ = app.update(Message::DialogCancel);
        assert_eq!(app.cmdline, "");
    }

    #[test]
    fn f_keys_from_the_command_line_reach_the_panel() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("f"), "").unwrap();
        let mut app = at(tmp.path());
        let _ = app.update(Message::Key(Action::Down));
        let _ = app.update(Message::CmdInput("x".into()));
        let _ = app.update(Message::FieldKey(Action::Copy));
        assert!(matches!(app.dialog, Some(Dialog::Input { .. })));
    }
}

mod tabs_more {
    use super::*;

    /// tmp with dirs `a`, `a/in` and file `f`; pane 0 shows tmp, cursor on `a`.
    fn setup() -> (tempfile::TempDir, App) {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("a/in")).unwrap();
        std::fs::write(tmp.path().join("f"), "").unwrap();
        let mut app = app_with(Config::default(), State::default());
        listed_at(&mut app, 0, tmp.path());
        let _ = app.update(Message::Key(Action::Down));
        (tmp, app)
    }

    fn key(app: &mut App, a: Action) {
        let _ = app.update(Message::Key(a));
    }

    #[test]
    fn ctrl_up_opens_the_dir_under_the_cursor_in_a_new_tab() {
        let (tmp, mut app) = setup();
        key(&mut app, Action::TabOpen);
        assert_eq!(
            cwds_or_targets(&app, 0),
            [tmp.path().into(), tmp.path().join("a")]
        );
        assert_eq!(app.panes[0].active_index(), 1);
        // On "..": the parent; on a file: the dir itself.
        let (tmp2, mut app) = setup();
        key(&mut app, Action::Up);
        key(&mut app, Action::TabOpen);
        let parent = tmp2.path().parent().unwrap().to_path_buf();
        assert_eq!(app.panes[0].active().target(), parent);
        let (tmp3, mut app) = setup();
        key(&mut app, Action::End);
        key(&mut app, Action::TabOpen);
        assert_eq!(app.panes[0].active().target(), tmp3.path());
    }

    #[test]
    fn regression_ctrl_up_on_dotdot_in_results_opens_the_searched_dir() {
        let (tmp, mut app) = setup();
        let f = tmp.path().join("f");
        let t = app.panes[0].active_mut();
        t.results = Some((tmp.path().to_path_buf(), Arc::new(vec![f.clone()])));
        let rows =
            read_listing(tmp.path(), false, Some(&vec![f]), &AtomicBool::new(false)).unwrap();
        t.panel.set_listing(tmp.path().to_path_buf(), rows, None);
        key(&mut app, Action::Home); // ".."
        key(&mut app, Action::TabOpen);
        // As Backspace there: the dir that was searched, not its parent.
        assert_eq!(app.panes[0].active().target(), tmp.path());
    }

    fn cwds_or_targets(app: &App, side: usize) -> Vec<PathBuf> {
        app.panes[side].items().iter().map(Tab::target).collect()
    }

    #[test]
    fn ctrl_shift_up_opens_it_in_the_other_panel_and_keeps_focus() {
        let (tmp, mut app) = setup();
        key(&mut app, Action::TabOpenOther);
        assert_eq!(app.panes[1].items().len(), 2);
        assert_eq!(app.panes[1].active().target(), tmp.path().join("a"));
        assert_eq!(app.panes[0].items().len(), 1);
        assert_eq!(app.active, 0);
    }

    #[test]
    fn copy_and_move_a_tab_to_the_other_panel() {
        let (tmp, mut app) = setup();
        key(&mut app, Action::TabCopyOther);
        assert_eq!(app.panes[0].items().len(), 1);
        assert_eq!(app.panes[1].items().len(), 2);
        assert_eq!(app.panes[1].active().target(), tmp.path());
        assert_eq!(app.active, 1);
        // The last tab of a panel stays.
        key(&mut app, Action::SwitchPane);
        key(&mut app, Action::TabMoveOther);
        assert_eq!(app.panes[0].items().len(), 1);
        assert!(app.msg().is_some());
        // Pane 1 has two: one moves over, with its lock and name.
        key(&mut app, Action::SwitchPane);
        key(&mut app, Action::TabLock);
        let id = app.panes[1].active().id;
        key(&mut app, Action::TabMoveOther);
        assert_eq!(app.panes[1].items().len(), 1);
        assert_eq!(app.panes[0].items().len(), 2);
        assert_eq!(app.panes[0].active().id, id);
        assert!(app.panes[0].active().locked);
        assert_eq!(app.active, 0);
    }

    #[test]
    fn locked_tab_stays_and_navigation_opens_a_new_one() {
        let (tmp, mut app) = setup();
        key(&mut app, Action::NewTab);
        key(&mut app, Action::TabLock);
        assert!(app.panes[0].active().locked);
        key(&mut app, Action::CloseTab);
        assert_eq!(app.panes[0].items().len(), 2);
        assert!(app.msg().is_some());
        let i = app.panes[0].active_index();
        let _ = app.update(Message::CloseTabAt(0, i));
        assert_eq!(app.panes[0].items().len(), 2);
        key(&mut app, Action::Enter); // cursor on `a`
        assert_eq!(app.panes[0].items().len(), 3);
        assert_eq!(app.panes[0].active().target(), tmp.path().join("a"));
        assert!(!app.panes[0].active().locked);
        let locked = &app.panes[0].items()[1];
        assert!(locked.locked);
        assert_eq!(locked.target(), tmp.path());
        // Unlock: a plain tab again.
        app.panes[0].select(1);
        key(&mut app, Action::TabLock);
        assert!(!app.panes[0].active().locked);
    }

    #[test]
    fn a_tab_locked_with_changes_moves_and_goes_back_when_left() {
        let (tmp, mut app) = setup();
        key(&mut app, Action::TabRoam);
        let t = app.panes[0].active();
        assert_eq!((t.home.as_deref(), t.locked), (Some(tmp.path()), false));
        key(&mut app, Action::CloseTab); // not closed, as locked
        assert_eq!(app.panes[0].items().len(), 1);
        key(&mut app, Action::Enter); // into `a`: in this very tab
        assert_eq!(app.panes[0].items().len(), 1);
        listed_at(&mut app, 0, &tmp.path().join("a"));
        assert!(app.panes[0].active().title().starts_with('!'));
        // Another tab shown: this one is read in its dir again.
        key(&mut app, Action::NewTab);
        let first = &app.panes[0].items()[0];
        assert_eq!(first.target(), tmp.path());
        // Saved: it opens there, still with dir changes allowed.
        app.panes[0].select(0);
        listed_at(&mut app, 0, &tmp.path().join("a"));
        app.save_state();
        let saved = &app.saved.panes[0];
        assert_eq!((saved.tabs[0].as_path(), saved.roam[0]), (tmp.path(), true));
        // Plain lock and this one exclude each other.
        key(&mut app, Action::TabLock);
        assert!(app.panes[0].active().locked && app.panes[0].active().home.is_none());
    }

    #[test]
    fn ctrl_up_keeps_the_sort_of_its_tab() {
        let (_tmp, mut app) = setup();
        key(&mut app, Action::Sort(shagoff_core::sort::SortKey::Size));
        let sort = app.panes[0].active().panel.sort();
        key(&mut app, Action::TabOpen);
        assert_eq!(app.panes[0].active().panel.sort(), sort);
    }

    #[test]
    fn rename_and_reset_a_tab_caption() {
        let (_tmp, mut app) = setup();
        key(&mut app, Action::TabRename);
        let shown = format::dir_title(app.panes[0].active().panel.cwd());
        assert!(matches!(&app.dialog, Some(Dialog::Input { input, .. }) if *input == shown));
        let _ = app.update(Message::DialogInput("Work".into()));
        let _ = app.update(Message::DialogSubmit);
        assert_eq!(app.panes[0].active().name.as_deref(), Some("Work"));
        key(&mut app, Action::TabRename); // the own caption is offered
        assert!(matches!(&app.dialog, Some(Dialog::Input { input, .. }) if input == "Work"));
        let _ = app.update(Message::DialogInput("  ".into()));
        let _ = app.update(Message::DialogSubmit);
        assert_eq!(app.panes[0].active().name, None);
    }

    #[test]
    fn close_others_keeps_active_and_locked() {
        let (_tmp, mut app) = setup();
        key(&mut app, Action::NewTab);
        key(&mut app, Action::TabLock);
        let locked = app.panes[0].active().id;
        key(&mut app, Action::NewTab);
        key(&mut app, Action::NewTab);
        let active = app.panes[0].active().id;
        key(&mut app, Action::CloseOtherTabs);
        let ids: Vec<u64> = app.panes[0].items().iter().map(|t| t.id).collect();
        assert_eq!(ids, [locked, active]);
        assert_eq!(app.panes[0].active().id, active);
    }

    #[test]
    fn regression_find_feed_leaves_a_locked_tab_alone() {
        let (tmp, mut app) = setup();
        key(&mut app, Action::TabLock);
        key(&mut app, Action::FindFiles);
        let _ = app.update(Message::FindStart);
        let id = find_dialog(&mut app).id;
        let found = vec![tmp.path().join("f")];
        let _ = app.update(Message::Find(crate::find::FindEvent::Found(id, found)));
        let _ = app.update(Message::FindFeed);
        assert_eq!(app.panes[0].items().len(), 2);
        assert!(app.panes[0].items()[0].results.is_none());
        assert!(app.panes[0].active().results.is_some());
    }

    #[test]
    fn regression_scan_in_flight_does_not_move_a_tab_locked_meanwhile() {
        let (tmp, mut app) = setup();
        key(&mut app, Action::Enter); // into `a`, not listed yet
        key(&mut app, Action::TabLock);
        let t = app.panes[0].active();
        assert!(t.pending.is_none());
        assert_eq!(t.target(), tmp.path());
    }

    #[test]
    fn regression_history_step_keeps_the_locked_tabs_place() {
        let (tmp, mut app) = setup();
        key(&mut app, Action::Enter);
        listed_at(&mut app, 0, &tmp.path().join("a"));
        key(&mut app, Action::TabLock);
        key(&mut app, Action::HistoryBack);
        assert_eq!(app.panes[0].items().len(), 2);
        assert_eq!(app.panes[0].active().target(), tmp.path());
        let locked = &app.panes[0].items()[0];
        assert_eq!(
            locked.history.clone().back(),
            Some(tmp.path().to_path_buf())
        );
    }

    #[test]
    fn regression_startup_path_does_not_repoint_a_locked_tab() {
        let tmp = tempfile::tempdir().unwrap();
        let state = State {
            panes: [
                PaneState {
                    tabs: vec!["/".into()],
                    active: 0,
                    locked: vec![true],
                    ..PaneState::default()
                },
                PaneState::default(),
            ],
            ..State::default()
        };
        let app = App::build(
            Core::default(),
            Config::default(),
            state,
            Some(tmp.path().into()),
            "/".into(),
        )
        .0;
        let tabs: Vec<PathBuf> = app.panes[0].items().iter().map(Tab::target).collect();
        assert_eq!(
            tabs,
            [PathBuf::from("/"), tmp.path().canonicalize().unwrap()]
        );
        assert!(app.panes[0].items()[0].locked);
        assert_eq!(app.panes[0].active_index(), 1);
    }

    #[test]
    fn brief_view_survives_a_restart() {
        let (_tmp, mut app) = setup();
        key(&mut app, Action::NewTab);
        key(&mut app, Action::ViewBrief);
        app.save_state();
        let state = app.saved.clone();
        assert_eq!(state.panes[0].brief, [false, true]);
        let app = app_with(Config::default(), state);
        let brief: Vec<bool> = app.panes[0].items().iter().map(|t| t.brief).collect();
        assert_eq!(brief, [false, true]);
    }

    #[test]
    fn lock_and_caption_survive_a_restart() {
        let (tmp, mut app) = setup();
        key(&mut app, Action::NewTab);
        key(&mut app, Action::TabLock);
        key(&mut app, Action::TabRename);
        let _ = app.update(Message::DialogInput("W".into()));
        let _ = app.update(Message::DialogSubmit);
        app.save_state();
        let state = app.saved.clone();
        assert_eq!(state.panes[0].locked, [false, true]);
        assert_eq!(state.panes[0].names, ["", "W"]);
        let app = app_with(Config::default(), state);
        let t = &app.panes[0].items()[1];
        assert!(t.locked);
        assert_eq!(t.name.as_deref(), Some("W"));
        assert_eq!(t.target(), tmp.path());
        assert!(!app.panes[0].items()[0].locked);
    }
}

#[test]
fn tabs_listed_opened_behind_and_closed_as_duplicates() {
    let mut app = app_with(Config::default(), State::default());
    let _ = app.update(Message::Key(Action::NewTabBg));
    assert_eq!(app.panes[0].items().len(), 2);
    assert_eq!(app.panes[0].active_index(), 0); // stayed in this one
    for t in app.panes[0].items_mut() {
        t.pending = None; // the start-up scan
        t.panel.set_listing("/x".into(), Vec::new(), None);
    }
    // The list: a row per tab, the active one under the cursor; Enter goes to the chosen one.
    let _ = app.update(Message::Key(Action::TabsList));
    let Some(Dialog::List {
        kind,
        cursor,
        items,
        ..
    }) = &mut app.dialog
    else {
        panic!("no list");
    };
    assert_eq!((*kind, items.len(), *cursor), (ListKind::Tabs, 2, 0));
    *cursor = 1;
    let _ = app.update(Message::DialogSubmit);
    assert_eq!(app.panes[0].active_index(), 1);
    // Both show the same dir: the active one stays, the other goes.
    let _ = app.update(Message::Key(Action::CloseDuplicateTabs));
    assert_eq!(app.panes[0].items().len(), 1);
    // A copy in the other panel, the keys staying here.
    let n = app.panes[1].items().len();
    let _ = app.update(Message::Key(Action::NewTabOther));
    assert_eq!((app.panes[1].items().len(), app.active), (n + 1, 0));
    // All locked, with dir changes, and back.
    let _ = app.update(Message::Key(Action::NewTabBg));
    app.panes[0].active_mut().home = Some("/own".into()); // moving about already: its dir kept
    let _ = app.update(Message::Key(Action::RoamAllTabs));
    assert!(
        app.panes[0]
            .items()
            .iter()
            .all(|t| t.home.is_some() && !t.locked)
    );
    assert_eq!(app.panes[0].active().home, Some(PathBuf::from("/own")));
    let _ = app.update(Message::Key(Action::LockAllTabs));
    assert!(
        app.panes[0]
            .items()
            .iter()
            .all(|t| t.locked && t.home.is_none())
    );
    let _ = app.update(Message::Key(Action::CloseAllTabs)); // the locked ones stay
    assert_eq!(app.panes[0].items().len(), 2);
    let _ = app.update(Message::Key(Action::UnlockAllTabs));
    assert!(!app.panes[0].active().pinned());
    let _ = app.update(Message::Key(Action::CloseAllTabs));
    assert_eq!(app.panes[0].items().len(), 1);
}

#[test]
fn a_new_file_from_a_template_is_its_copy_never_over_a_file() {
    let d = tempfile::tempdir().unwrap();
    let template = d.path().join("tpl.txt");
    std::fs::write(&template, "hello").unwrap();
    let mut app = app_with(Config::default(), State::default());
    let p = &mut app.panes[0].active_mut().panel;
    p.set_listing(d.path().to_path_buf(), Vec::new(), None);
    let ask = |name: &str| Dialog::Input {
        op: InputOp::FromTemplate,
        side: 0,
        sources: vec![template.clone()],
        input: name.into(),
    };
    app.dialog = Some(ask("note.txt"));
    let _ = app.update(Message::DialogSubmit);
    assert_eq!(
        std::fs::read_to_string(d.path().join("note.txt")).unwrap(),
        "hello"
    );
    std::fs::write(d.path().join("note.txt"), "mine").unwrap();
    app.dialog = Some(ask("note.txt"));
    let _ = app.update(Message::DialogSubmit);
    assert_eq!(
        std::fs::read_to_string(d.path().join("note.txt")).unwrap(),
        "mine"
    );
    assert!(
        app.status
            .as_ref()
            .is_some_and(|s| s.kind == StatusKind::Error)
    );
}

#[test]
fn split_offers_the_other_panel_and_combine_only_takes_parts() {
    let (d, o) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    std::fs::write(d.path().join("big.bin"), [1u8; 10]).unwrap();
    std::fs::write(d.path().join("big.bin.001"), [1u8; 10]).unwrap();
    let mut app = app_with(Config::default(), State::default());
    for (side, dir) in [(0, d.path()), (1, o.path())] {
        let tab = app.panes[side].active_mut();
        tab.pending = None;
        let entries = listing::scan(dir, false).unwrap();
        tab.panel
            .set_listing(dir.to_path_buf(), entries, Some("big.bin"));
    }
    let _ = app.update(Message::Key(Action::SplitFile));
    let Some(Dialog::Split { path, size, .. }) = &app.dialog else {
        panic!("{:?}", app.dialog.is_some());
    };
    assert_eq!(
        (path.as_str(), size.as_str()),
        (dir_input(o.path()).as_str(), SPLIT_SIZE)
    );
    let _ = app.update(Message::SplitSize("0".into()));
    let _ = app.update(Message::DialogSubmit);
    assert!(
        app.status
            .as_ref()
            .is_some_and(|s| s.kind == StatusKind::Error)
    );
    // Not a part under the cursor: said, no dialog.
    app.dialog = None;
    let _ = app.update(Message::Key(Action::CombineFiles));
    assert!(app.dialog.is_none());
    let p = &mut app.panes[0].active_mut().panel;
    let entries = listing::scan(d.path(), false).unwrap();
    p.set_listing(d.path().to_path_buf(), entries, Some("big.bin.001"));
    let _ = app.update(Message::Key(Action::CombineFiles));
    assert!(matches!(
        &app.dialog,
        Some(Dialog::Input { op: InputOp::Combine, sources, .. }) if sources.len() == 1
    ));
}

#[test]
fn move_to_archive_opens_alt_f5_with_delete_sources_on() {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("a.txt"), "a").unwrap();
    let mut app = app_with(Config::default(), State::default());
    let tab = app.panes[0].active_mut();
    tab.pending = None;
    let entries = listing::scan(d.path(), false).unwrap();
    tab.panel
        .set_listing(d.path().to_path_buf(), entries, Some("a.txt"));
    for (action, delete) in [(Action::Pack, false), (Action::MoveToArchive, true)] {
        app.dialog = None;
        let _ = app.update(Message::Key(action));
        let Some(Dialog::Pack(p)) = &app.dialog else {
            panic!("{action:?}: no pack dialog");
        };
        assert_eq!(p.move_after, delete, "{action:?}");
    }
}

#[test]
fn compare_with_subdirs_marks_dirs_and_diff_only_hides_the_same() {
    let (l, r) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    for d in [l.path(), r.path()] {
        std::fs::write(d.join("same.txt"), "s").unwrap();
    }
    std::fs::write(l.path().join("only.txt"), "o").unwrap();
    std::fs::create_dir(l.path().join("sub")).unwrap();
    let mut app = app_with(Config::default(), State::default());
    let list = |app: &mut App| {
        for (side, dir) in [(0, l.path()), (1, r.path())] {
            let tab = app.panes[side].active_mut();
            tab.pending = None;
            let entries = listing::scan(dir, false).unwrap();
            tab.panel.set_listing(dir.to_path_buf(), entries, None);
        }
    };
    let marked = |app: &App| app.panes[0].active().panel.marked_names().len();
    let shown = |app: &App| app.panes[0].active().panel.entries().len();
    list(&mut app);
    let _ = app.update(Message::Key(Action::CompareLists));
    assert_eq!(marked(&app), 1);
    let _ = app.update(Message::Key(Action::CompareListsSubdirs));
    assert_eq!(marked(&app), 2);
    let all = shown(&app);
    let _ = app.update(Message::Key(Action::CompareListsDiffOnly));
    assert_eq!(shown(&app), all - 1, "same.txt hidden, sub not");
    // Nothing differs on the right: all of it hidden but `..`.
    let right = &app.panes[1].active().panel;
    assert!(right.entries().iter().all(|e| e.name == ".."));
    // Again: what was hidden is compared too, not taken for missing.
    let _ = app.update(Message::Key(Action::CompareLists));
    assert_eq!(app.panes[1].active().panel.marked_names().len(), 0);
}

#[test]
fn switches_flip_their_settings_and_the_ignore_list_hides_its_names() {
    let d = tempfile::tempdir().unwrap();
    for f in ["keep.txt", "ignored-188.tmp"] {
        std::fs::write(d.path().join(f), "").unwrap();
    }
    let config = Config {
        // A name no other test has: the list is program-wide.
        ignore_list: "ignored-188.tmp".into(),
        ..Config::default()
    };
    let mut app = app_with(config, State::default());
    let tab = app.panes[0].active_mut();
    tab.pending = None;
    let entries = listing::scan(d.path(), false).unwrap();
    tab.panel.set_listing(d.path().to_path_buf(), entries, None);
    let names = |app: &App| app.panes[0].active().panel.entries().len();
    let all = names(&app);
    let _ = app.update(Message::Key(Action::SwitchIgnoreList));
    assert!(app.config.ignore_on);
    assert_eq!(names(&app), all - 1);
    let _ = app.update(Message::Key(Action::SwitchIgnoreList));
    assert_eq!(names(&app), all);
    let _ = app.update(Message::Key(Action::SwitchWatchDirs));
    let _ = app.update(Message::Key(Action::SwitchTypeColors));
    assert!(!app.config.watch_dirs && !app.config.type_colors);
}
