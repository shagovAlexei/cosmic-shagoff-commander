# 79. Branch: stop a long read

ROADMAP «Ветвь, дальше: остановка долгого чтения».

- `listing::branch(dirs, hidden, stop)` checks `stop` before each dir and gives `None` once set.
- `Tab::branch_stop` (a fresh `Arc<AtomicBool>` per Ctrl+B): set by Esc while the active tab
  reads (status «Branch: stopped»), by leaving the dir (`App::load`), and by a new Ctrl+B.
- Esc order: drawer, search, job, viewer, connect, **branch read**, command line, filter.
- Stopped = nothing shown (no partial list); the status hint while reading names Esc.
