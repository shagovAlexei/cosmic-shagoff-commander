# 79. Branch: stop a long read

ROADMAP «Ветвь, дальше: остановка долгого чтения».

- `listing::branch(dirs, hidden, stop)` checks `stop` at each entry and gives `None` once set.
- `Tab::branch_stop` (a fresh `Arc<AtomicBool>` per Ctrl+B): set by Esc while any tab reads
  (status «Branch: stopped»), by leaving the dir (`App::load`), by a new Ctrl+B, by «Feed to
  panel» from Alt+F7, and when the tab is dropped (closed).
- Not stopped: the second step, stat of every found path (`listing::entries`) once the walk is in.
- Esc order: drawer, search, job, viewer, connect, **branch read**, command line, filter.
- Stopped = nothing shown (no partial list); the status hint while reading names Esc.
