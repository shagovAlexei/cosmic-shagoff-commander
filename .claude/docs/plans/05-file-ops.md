# Фаза 5: файловые операции — план реализации

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** TC file operations: F5 copy, F6 move, Shift+F6/F2 rename, F7 mkdir, F8/Del trash, Shift+F8/Shift+Del permanent delete. Each has a progress dialog, a conflict dialog and an error dialog.

**Architecture:**
- `crates/core/src/ops.rs` is a synchronous engine (`plan`, `transfer`, `delete`). It is driven through a `Handler` trait and tested on temp dirs with a scripted handler.
- The app runs the engine on a worker thread. Events reach the UI over a futures channel with `Task::run`. Answers to conflicts and errors go back over `std::sync::mpsc`. Cancel is an `Arc<AtomicBool>`.
- All dialogs become one `Dialog` enum, kept in `dialogs.rs`. The worker-thread glue goes in `jobs.rs`.

**Tech Stack:** Rust 2024, std::fs, crate `trash = "5"` (core), libcosmic (pinned rev).

**Spec:** `.claude/docs/specs/05-file-ops.md`

## Global Constraints

- Data safety first. A file is written to `<name>.shagoff-part` and renamed into place. A source is deleted only after its copy succeeded. Symlinks are never followed when copying or deleting.
- `crates/core` has no UI dependencies. The only new crate allowed is `trash = "5"`.
- `..` is never an operation target.
- Every new UI string goes into both `en` and `ru`.
- `just verify` must be clean. Branch: `feat/file-ops`.

## Review Focus

1. **Cancel during "Replace".** The existing target must survive intact and no `.shagoff-part` may be left behind. Tested by `cancel_mid_file_keeps_existing_target` in Task 2.
2. **Move across devices, or into an existing dir.** A source may be removed only after it was copied, and skipped files must stay in the source. Tested by `move_merges_into_existing_dir` and `move_keeps_skipped_files_in_source` in Task 3. For cross-device moves the same copy+delete path is used; check it manually with `/tmp` → `~` if they are on different filesystems.
3. **Copy a directory into its own subdirectory, or a file onto itself.** Both must be refused before anything is touched. Tested by `plan_rejects_into_itself` and `plan_rejects_same_file` in Task 2.
4. **Symlink to a directory outside the tree.** Copying must copy the link and leave the target alone. Permanent delete must remove the link and leave the target alone. Tested by `copy_tree_keeps_symlinks` and `delete_permanent_does_not_follow_symlinks` in Task 3.
5. **Worker thread blocked on a conflict answer when the UI closes the dialog any other way** (Escape, window close). The worker must get `Cancel`, never hang. Covered by the Escape routing and the `Sender` drop in Task 6, plus a manual check.

---

### Task 1: core — `targets`, `unmark`; app — new tab without marks

**Files:**
- Modify: `crates/core/src/panel.rs`
- Modify: `crates/app/src/app.rs` (`Tab::duplicate`)

**Interfaces:**
- Produces: `Panel::targets(&self) -> Vec<PathBuf>`, `Panel::unmark(&mut self, paths: &[PathBuf])`.

- [ ] **Step 1: Write the failing tests** (panel.rs; add `todo!()` stubs for both methods):
  ```rust
  #[test]
  fn targets_are_marked_or_cursor_never_parent() {
      let mut p = loaded("/x", vec![f("a", 1), f("b", 1)]);
      assert!(p.targets().is_empty()); // cursor on ".."
      p.set_cursor(1);
      assert_eq!(p.targets(), [PathBuf::from("/x/a")]);
      p.set_cursor(2);
      p.toggle_mark(); // mark b; cursor stays on b
      p.set_cursor(1); // cursor on a, but marks win
      assert_eq!(p.targets(), [PathBuf::from("/x/b")]);
  }

  #[test]
  fn unmark_removes_given_paths() {
      let mut p = loaded("/x", vec![f("a", 1), f("b", 1)]);
      p.mark_all(true);
      p.unmark(&[PathBuf::from("/x/a")]);
      assert_eq!(marked_names(&p), ["b"]);
  }
  ```

- [ ] **Step 2: Run the tests and confirm they fail**

  Run: `cargo test -p shagoff-core --lib panel`

  Expected: 2 tests fail with `not yet implemented`.

- [ ] **Step 3: Implement**
  ```rust
      /// What an operation acts on: marked entries (in list order), else the row under the cursor; never `..`.
      pub fn targets(&self) -> Vec<PathBuf> {
          let marked: Vec<PathBuf> = self
              .entries
              .iter()
              .filter(|e| self.is_marked(e))
              .map(|e| self.cwd.join(&e.os_name))
              .collect();
          if !marked.is_empty() {
              return marked;
          }
          self.current()
              .filter(|e| e.name != PARENT)
              .map(|e| vec![self.cwd.join(&e.os_name)])
              .unwrap_or_default()
      }

      /// Drop marks of processed entries (by file name).
      pub fn unmark(&mut self, paths: &[PathBuf]) {
          for name in paths.iter().filter_map(|p| p.file_name()) {
              self.marked.remove(name);
          }
      }
  ```
  In app.rs, change `Tab::duplicate` to clone the panel and then call `panel.mark_all(false)`, so a new tab starts with no marks.

- [ ] **Step 4: Run the tests and confirm they pass**

  Run: `cargo test -p shagoff-core --lib`

  Expected: 61 passed.

- [ ] **Step 5: Commit**
  ```bash
  cargo fmt --all && cargo clippy --all-targets -- -D warnings
  git add crates && git commit -m "core: operation targets and unmark; new tabs start unmarked"
  ```

---

### Task 2: core — `ops`: plan, copy, conflicts, cancel

**Files:**
- Create: `crates/core/src/ops.rs`
- Modify: `crates/core/src/lib.rs` (`pub mod ops;`)
- Modify: `crates/core/Cargo.toml` (`trash = "5"`; it is used in Task 3, but add it now so `Cargo.lock` changes once)

**Interfaces:**
- Produces:
  - Types: `Method { Copy, Move }`, `Resolution { Replace, Skip, ReplaceAll, SkipAll, ReplaceOlder, Cancel }`, `ErrorChoice { Retry, Skip, Cancel }`, `FileInfo { path, size, mtime }`, `Report { cancelled: bool, completed: Vec<PathBuf> }`, `PlanError { Empty, IntoItself(PathBuf), SameFile(PathBuf) }`.
  - `trait Handler`.
  - `plan(&[PathBuf], &Path) -> Result<Vec<(PathBuf, PathBuf)>, PlanError>`.
  - `transfer(Method, &[(PathBuf, PathBuf)], &mut dyn Handler) -> Report`.

- [ ] **Step 1: Write the failing tests**

  First write `ops.rs` with all types and the trait, and `todo!()` bodies for `plan` and `transfer`. The full implementation is in Step 3. Then add the test module:
  ```rust
  #[cfg(test)]
  mod tests {
      use super::*;
      use std::os::unix::fs::{PermissionsExt, symlink};
      use std::time::Duration;

      /// Scripted answers; records what the engine asked.
      #[derive(Default)]
      pub(super) struct Script {
          pub conflicts: Vec<Resolution>,
          pub errors: Vec<ErrorChoice>,
          pub asked: usize,
          pub errored: Vec<PathBuf>,
          pub cancel_after_progress: bool,
          pub progressed: bool,
      }

      impl Handler for Script {
          fn progress(&mut self, _done: u64, _total: u64, _current: &Path) {
              self.progressed = true;
          }
          fn conflict(&mut self, _src: &FileInfo, _dst: &FileInfo) -> Resolution {
              self.asked += 1;
              if self.conflicts.is_empty() { Resolution::Cancel } else { self.conflicts.remove(0) }
          }
          fn error(&mut self, path: &Path, _err: &io::Error) -> ErrorChoice {
              self.errored.push(path.to_path_buf());
              if self.errors.is_empty() { ErrorChoice::Cancel } else { self.errors.remove(0) }
          }
          fn cancelled(&self) -> bool {
              self.cancel_after_progress && self.progressed
          }
      }

      pub(super) fn write(p: &Path, s: &str) {
          fs::create_dir_all(p.parent().unwrap()).unwrap();
          fs::write(p, s).unwrap();
      }
      pub(super) fn read(p: &Path) -> String {
          fs::read_to_string(p).unwrap()
      }
      fn copy(srcs: &[PathBuf], dest: &Path, h: &mut Script) -> Report {
          transfer(Method::Copy, &plan(srcs, dest).unwrap(), h)
      }

      #[test]
      fn plan_into_existing_dir_and_new_name() {
          let d = tempfile::tempdir().unwrap();
          let (a, to) = (d.path().join("a"), d.path().join("to"));
          write(&a, "x");
          fs::create_dir(&to).unwrap();
          assert_eq!(plan(&[a.clone()], &to).unwrap(), [(a.clone(), to.join("a"))]);
          let new = d.path().join("b");
          assert_eq!(plan(&[a.clone()], &new).unwrap(), [(a, new)]);
      }

      #[test]
      fn plan_many_into_missing_dir() {
          let d = tempfile::tempdir().unwrap();
          let (a, b, to) = (d.path().join("a"), d.path().join("b"), d.path().join("new"));
          write(&a, "1");
          write(&b, "2");
          let pairs = plan(&[a.clone(), b.clone()], &to).unwrap();
          assert_eq!(pairs, [(a, to.join("a")), (b, to.join("b"))]);
      }

      #[test]
      fn plan_rejects_into_itself() {
          let d = tempfile::tempdir().unwrap();
          let src = d.path().join("dir");
          fs::create_dir_all(src.join("sub")).unwrap();
          assert_eq!(plan(&[src.clone()], &src.join("sub")), Err(PlanError::IntoItself(src.clone())));
          assert_eq!(plan(&[src.clone()], &src), Err(PlanError::IntoItself(src)));
      }

      #[test]
      fn plan_rejects_same_file() {
          let d = tempfile::tempdir().unwrap();
          let a = d.path().join("a");
          write(&a, "x");
          assert_eq!(plan(&[a.clone()], d.path()), Err(PlanError::SameFile(a.clone())));
          assert_eq!(plan(&[], d.path()), Err(PlanError::Empty));
      }

      #[test]
      fn copy_file_keeps_content_mode_and_mtime() {
          let d = tempfile::tempdir().unwrap();
          let (a, to) = (d.path().join("a"), d.path().join("to"));
          write(&a, "hello");
          fs::set_permissions(&a, fs::Permissions::from_mode(0o640)).unwrap();
          let t = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
          File::options().write(true).open(&a).unwrap().set_modified(t).unwrap();
          fs::create_dir(&to).unwrap();
          let r = copy(&[a.clone()], &to, &mut Script::default());
          let c = to.join("a");
          assert_eq!(read(&c), "hello");
          assert_eq!(fs::metadata(&c).unwrap().permissions().mode() & 0o777, 0o640);
          assert_eq!(fs::metadata(&c).unwrap().modified().unwrap(), t);
          assert_eq!(r, Report { cancelled: false, completed: vec![a.clone()] });
          assert_eq!(read(&a), "hello"); // copy keeps the source
      }

      #[test]
      fn copy_tree_keeps_symlinks() {
          let d = tempfile::tempdir().unwrap();
          let src = d.path().join("src");
          write(&src.join("x/deep.txt"), "deep");
          let outside = d.path().join("outside");
          fs::create_dir(&outside).unwrap();
          symlink(&outside, src.join("link")).unwrap();
          let to = d.path().join("to");
          fs::create_dir(&to).unwrap();
          copy(&[src.clone()], &to, &mut Script::default());
          assert_eq!(read(&to.join("src/x/deep.txt")), "deep");
          let link = to.join("src/link");
          assert!(fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
          assert_eq!(fs::read_link(&link).unwrap(), outside);
      }

      /// `to/a` exists with "old"; copy `a` ("new") over it with the scripted answers.
      fn conflict_case(answers: Vec<Resolution>) -> (tempfile::TempDir, Script, Report) {
          let d = tempfile::tempdir().unwrap();
          write(&d.path().join("a"), "new");
          write(&d.path().join("to/a"), "old");
          let mut h = Script { conflicts: answers, ..Default::default() };
          let r = copy(&[d.path().join("a")], &d.path().join("to"), &mut h);
          (d, h, r)
      }

      #[test]
      fn conflict_replace_and_skip() {
          let (d, _, r) = conflict_case(vec![Resolution::Replace]);
          assert_eq!(read(&d.path().join("to/a")), "new");
          assert!(!r.cancelled);
          let (d, _, r) = conflict_case(vec![Resolution::Skip]);
          assert_eq!(read(&d.path().join("to/a")), "old");
          assert!(r.completed.is_empty());
      }

      #[test]
      fn conflict_cancel_stops() {
          let (d, _, r) = conflict_case(vec![Resolution::Cancel]);
          assert_eq!(read(&d.path().join("to/a")), "old");
          assert!(r.cancelled);
      }

      #[test]
      fn replace_all_and_skip_all_ask_once() {
          for (answer, expect) in [(Resolution::ReplaceAll, "new"), (Resolution::SkipAll, "old")] {
              let d = tempfile::tempdir().unwrap();
              for n in ["a", "b"] {
                  write(&d.path().join(n), "new");
                  write(&d.path().join("to").join(n), "old");
              }
              let mut h = Script { conflicts: vec![answer], ..Default::default() };
              copy(&[d.path().join("a"), d.path().join("b")], &d.path().join("to"), &mut h);
              assert_eq!(h.asked, 1);
              assert_eq!(read(&d.path().join("to/a")), expect);
              assert_eq!(read(&d.path().join("to/b")), expect);
          }
      }

      #[test]
      fn replace_older_replaces_only_older_targets() {
          let d = tempfile::tempdir().unwrap();
          let old_t = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000);
          let new_t = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
          let set = |p: &Path, t| File::options().write(true).open(p).unwrap().set_modified(t).unwrap();
          // a: source newer → replaced; b: source older → kept
          write(&d.path().join("a"), "src");
          set(&d.path().join("a"), new_t);
          write(&d.path().join("to/a"), "dst");
          set(&d.path().join("to/a"), old_t);
          write(&d.path().join("b"), "src");
          set(&d.path().join("b"), old_t);
          write(&d.path().join("to/b"), "dst");
          set(&d.path().join("to/b"), new_t);
          let mut h = Script { conflicts: vec![Resolution::ReplaceOlder], ..Default::default() };
          copy(&[d.path().join("a"), d.path().join("b")], &d.path().join("to"), &mut h);
          assert_eq!(read(&d.path().join("to/a")), "src");
          assert_eq!(read(&d.path().join("to/b")), "dst");
          assert_eq!(h.asked, 1);
      }

      #[test]
      fn cancel_mid_file_keeps_existing_target() {
          let d = tempfile::tempdir().unwrap();
          let big = d.path().join("big");
          fs::write(&big, vec![7u8; 3 << 20]).unwrap(); // 3 chunks of 1 MiB
          write(&d.path().join("to/big"), "precious");
          let mut h = Script { conflicts: vec![Resolution::Replace], cancel_after_progress: true, ..Default::default() };
          let r = copy(&[big], &d.path().join("to"), &mut h);
          assert!(r.cancelled);
          assert_eq!(read(&d.path().join("to/big")), "precious");
          assert!(!d.path().join("to/big.shagoff-part").exists());
      }

      #[test]
      fn unreadable_file_skip_copies_the_rest() {
          let d = tempfile::tempdir().unwrap();
          let (bad, good) = (d.path().join("bad"), d.path().join("good"));
          write(&bad, "secret");
          write(&good, "ok");
          fs::set_permissions(&bad, fs::Permissions::from_mode(0o000)).unwrap();
          if File::open(&bad).is_ok() {
              return; // running as root: permissions don't apply
          }
          let to = d.path().join("to");
          fs::create_dir(&to).unwrap();
          let mut h = Script { errors: vec![ErrorChoice::Skip], ..Default::default() };
          let r = copy(&[bad.clone(), good.clone()], &to, &mut h);
          assert_eq!(h.errored, [bad]);
          assert_eq!(read(&to.join("good")), "ok");
          assert!(!to.join("bad").exists() && !to.join("bad.shagoff-part").exists());
          assert_eq!(r.completed, [good]);
      }
  }
  ```

- [ ] **Step 2: Run the tests and confirm they fail**

  Run: `cargo test -p shagoff-core --lib ops`

  Expected: 11 tests fail with `not yet implemented`.

- [ ] **Step 3: Implement** — the full `ops.rs` body above the tests:
  ```rust
  //! File operations: copy / move / delete. Synchronous — run it on a worker thread. The UI is reached
  //! only through `Handler`, so tests drive the engine with scripted answers.

  use std::fs::{self, File, Metadata};
  use std::io::{self, Read, Write};
  use std::path::{Path, PathBuf};
  use std::time::SystemTime;

  #[derive(Clone, Copy, Debug, PartialEq, Eq)]
  pub enum Method { Copy, Move }

  #[derive(Clone, Copy, Debug, PartialEq, Eq)]
  pub enum Resolution { Replace, Skip, ReplaceAll, SkipAll, ReplaceOlder, Cancel }

  #[derive(Clone, Copy, Debug, PartialEq, Eq)]
  pub enum ErrorChoice { Retry, Skip, Cancel }

  /// What the conflict dialog shows about each side.
  #[derive(Clone, Debug, PartialEq, Eq)]
  pub struct FileInfo { pub path: PathBuf, pub size: u64, pub mtime: SystemTime }

  pub trait Handler {
      /// Bytes for `transfer`, items for `delete`.
      fn progress(&mut self, done: u64, total: u64, current: &Path);
      fn conflict(&mut self, src: &FileInfo, dst: &FileInfo) -> Resolution;
      fn error(&mut self, path: &Path, err: &io::Error) -> ErrorChoice;
      fn cancelled(&self) -> bool;
  }

  #[derive(Debug, Default, PartialEq, Eq)]
  pub struct Report {
      pub cancelled: bool,
      /// Sources processed completely (for unmarking).
      pub completed: Vec<PathBuf>,
  }

  #[derive(Debug, PartialEq, Eq)]
  pub enum PlanError { Empty, IntoItself(PathBuf), SameFile(PathBuf) }

  const PART: &str = ".shagoff-part";

  /// (source, destination) pairs, TC rules: existing dir → inside it; one source and no such path →
  /// that name; several sources → inside `dest` (created on demand).
  pub fn plan(sources: &[PathBuf], dest: &Path) -> Result<Vec<(PathBuf, PathBuf)>, PlanError> {
      if sources.is_empty() {
          return Err(PlanError::Empty);
      }
      let into_dir = dest.is_dir() || sources.len() > 1;
      let mut pairs = Vec::with_capacity(sources.len());
      for src in sources {
          let dst = match (into_dir, src.file_name()) {
              (true, Some(name)) => dest.join(name),
              _ => dest.to_path_buf(),
          };
          let (s, d) = (absolute(src), absolute(&dst));
          if s == d {
              return Err(PlanError::SameFile(src.clone()));
          }
          let src_is_dir = fs::symlink_metadata(src).is_ok_and(|m| m.is_dir());
          if src_is_dir && d.starts_with(&s) {
              return Err(PlanError::IntoItself(src.clone()));
          }
          pairs.push((src.clone(), dst));
      }
      Ok(pairs)
  }

  pub fn transfer(method: Method, pairs: &[(PathBuf, PathBuf)], h: &mut dyn Handler) -> Report {
      let total = pairs.iter().map(|(s, _)| tree_size(s)).sum();
      let mut t = Transfer { method, h, done: 0, total, policy: None, approved: None };
      let mut report = Report::default();
      for (src, dst) in pairs {
          match t.entry(src, dst) {
              Step::Done => report.completed.push(src.clone()),
              Step::Skipped => {}
              Step::Cancel => {
                  report.cancelled = true;
                  break;
              }
          }
      }
      report
  }

  enum Step { Done, Skipped, Cancel }
  enum Decision { Replace, Skip, Cancel }

  struct Transfer<'a> {
      method: Method,
      h: &'a mut dyn Handler,
      done: u64,
      total: u64,
      /// Sticky answer from "… all" / "Replace older".
      policy: Option<Resolution>,
      /// Target already approved for replacing when a move falls back from rename to copy.
      approved: Option<PathBuf>,
  }

  impl Transfer<'_> {
      fn entry(&mut self, src: &Path, dst: &Path) -> Step {
          if self.h.cancelled() {
              return Step::Cancel;
          }
          let meta = match self.retry(src, || fs::symlink_metadata(src)) {
              Ok(m) => m,
              Err(s) => return s,
          };
          if let Some(parent) = dst.parent()
              && let Err(s) = self.retry(parent, || fs::create_dir_all(parent))
          {
              return s;
          }
          if self.method == Method::Move
              && let Some(step) = self.rename(src, dst, &meta)
          {
              return step;
          }
          if meta.is_dir() {
              self.dir(src, dst, &meta)
          } else if meta.file_type().is_symlink() {
              self.symlink(src, dst, &meta)
          } else {
              self.file(src, dst, &meta)
          }
      }

      /// Move fast path. `None` = fall back to copy + delete (other device, or merging dirs).
      fn rename(&mut self, src: &Path, dst: &Path, meta: &Metadata) -> Option<Step> {
          let mut replacing = false;
          if let Ok(dm) = fs::symlink_metadata(dst) {
              if meta.is_dir() || dm.is_dir() {
                  return None;
              }
              match self.decide(src, meta, dst, &dm) {
                  Decision::Replace => replacing = true,
                  Decision::Skip => {
                      self.done += meta.len();
                      return Some(Step::Skipped);
                  }
                  Decision::Cancel => return Some(Step::Cancel),
              }
          }
          let size = tree_size(src);
          loop {
              match fs::rename(src, dst) {
                  Ok(()) => {
                      self.done += size;
                      self.h.progress(self.done, self.total, src);
                      return Some(Step::Done);
                  }
                  Err(e) if e.kind() == io::ErrorKind::CrossesDevices => {
                      if replacing {
                          self.approved = Some(dst.to_path_buf());
                      }
                      return None;
                  }
                  Err(e) => match self.h.error(src, &e) {
                      ErrorChoice::Retry => {}
                      ErrorChoice::Skip => {
                          self.done += size;
                          return Some(Step::Skipped);
                      }
                      ErrorChoice::Cancel => return Some(Step::Cancel),
                  },
              }
          }
      }

      fn dir(&mut self, src: &Path, dst: &Path, meta: &Metadata) -> Step {
          match fs::symlink_metadata(dst) {
              Ok(dm) if dm.is_dir() => {} // merge into the existing dir
              Ok(_) => return self.clash(dst, tree_size(src)),
              Err(_) => {
                  if let Err(s) = self.retry(dst, || fs::create_dir(dst)) {
                      return s;
                  }
              }
          }
          let names = match self.retry(src, || {
              fs::read_dir(src)?.map(|e| e.map(|e| e.file_name())).collect::<io::Result<Vec<_>>>()
          }) {
              Ok(n) => n,
              Err(s) => return s,
          };
          let mut complete = true;
          for name in names {
              match self.entry(&src.join(&name), &dst.join(&name)) {
                  Step::Done => {}
                  Step::Skipped => complete = false,
                  Step::Cancel => return Step::Cancel,
              }
          }
          // After the children: a read-only dir would have blocked writing them.
          let _ = fs::set_permissions(dst, meta.permissions());
          if !complete {
              return Step::Skipped;
          }
          if self.method == Method::Move
              && let Err(s) = self.retry(src, || fs::remove_dir(src))
          {
              return s;
          }
          Step::Done
      }

      fn symlink(&mut self, src: &Path, dst: &Path, meta: &Metadata) -> Step {
          let target = match self.retry(src, || fs::read_link(src)) {
              Ok(t) => t,
              Err(s) => return s,
          };
          if let Some(step) = self.resolve_existing(src, meta, dst, 0) {
              return step;
          }
          // Link at the part path, then rename over: replacing stays atomic.
          let part = part_path(dst);
          let _ = fs::remove_file(&part);
          if let Err(s) = self.retry(dst, || std::os::unix::fs::symlink(&target, &part)) {
              return s;
          }
          if let Err(s) = self.retry(dst, || fs::rename(&part, dst)) {
              let _ = fs::remove_file(&part);
              return s;
          }
          if self.method == Method::Move
              && let Err(s) = self.retry(src, || fs::remove_file(src))
          {
              return s;
          }
          Step::Done
      }

      fn file(&mut self, src: &Path, dst: &Path, meta: &Metadata) -> Step {
          let size = meta.len();
          if let Some(step) = self.resolve_existing(src, meta, dst, size) {
              return step;
          }
          let part = part_path(dst);
          let start = self.done;
          loop {
              self.done = start;
              match self.copy_contents(src, &part, meta) {
                  Ok(true) => break,
                  Ok(false) => {
                      let _ = fs::remove_file(&part);
                      return Step::Cancel;
                  }
                  Err(e) => {
                      let _ = fs::remove_file(&part);
                      match self.h.error(src, &e) {
                          ErrorChoice::Retry => {}
                          ErrorChoice::Skip => {
                              self.done = start + size;
                              return Step::Skipped;
                          }
                          ErrorChoice::Cancel => return Step::Cancel,
                      }
                  }
              }
          }
          if let Err(s) = self.retry(dst, || fs::rename(&part, dst)) {
              let _ = fs::remove_file(&part);
              return s;
          }
          self.h.progress(self.done, self.total, src);
          if self.method == Method::Move
              && let Err(s) = self.retry(src, || fs::remove_file(src))
          {
              return s;
          }
          Step::Done
      }

      /// If `dst` exists: ask (or apply the sticky policy). `None` = go ahead and replace.
      fn resolve_existing(&mut self, src: &Path, meta: &Metadata, dst: &Path, size: u64) -> Option<Step> {
          let dm = fs::symlink_metadata(dst).ok()?;
          if dm.is_dir() {
              return Some(self.clash(dst, size));
          }
          if self.approved.take_if(|p| p == dst).is_some() {
              return None;
          }
          match self.decide(src, meta, dst, &dm) {
              Decision::Replace => None,
              Decision::Skip => {
                  self.done += size;
                  Some(Step::Skipped)
              }
              Decision::Cancel => Some(Step::Cancel),
          }
      }

      /// Copy bytes, then mtime and permissions, into `part`. `Ok(false)` = cancelled.
      fn copy_contents(&mut self, src: &Path, part: &Path, meta: &Metadata) -> io::Result<bool> {
          let mut r = File::open(src)?;
          let mut w = File::create(part)?;
          let mut buf = vec![0; 1 << 20];
          loop {
              if self.h.cancelled() {
                  return Ok(false);
              }
              let n = match r.read(&mut buf) {
                  Ok(0) => break,
                  Ok(n) => n,
                  Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                  Err(e) => return Err(e),
              };
              w.write_all(&buf[..n])?;
              self.done += n as u64;
              self.h.progress(self.done, self.total, src);
          }
          w.set_modified(meta.modified()?)?;
          w.set_permissions(meta.permissions())?;
          Ok(true)
      }

      fn decide(&mut self, src: &Path, sm: &Metadata, dst: &Path, dm: &Metadata) -> Decision {
          let answer = match self.policy {
              Some(p) => p,
              None => {
                  let r = self.h.conflict(&info(src, sm), &info(dst, dm));
                  if matches!(r, Resolution::ReplaceAll | Resolution::SkipAll | Resolution::ReplaceOlder) {
                      self.policy = Some(r);
                  }
                  r
              }
          };
          match answer {
              Resolution::Replace | Resolution::ReplaceAll => Decision::Replace,
              Resolution::Skip | Resolution::SkipAll => Decision::Skip,
              Resolution::ReplaceOlder if sm.modified().ok() > dm.modified().ok() => Decision::Replace,
              Resolution::ReplaceOlder => Decision::Skip,
              Resolution::Cancel => Decision::Cancel,
          }
      }

      /// A file/link meets a directory of the same name (or the reverse): report it; Retry acts as Skip.
      fn clash(&mut self, dst: &Path, size: u64) -> Step {
          let e = io::Error::new(io::ErrorKind::AlreadyExists, "an entry of another type has this name");
          match self.h.error(dst, &e) {
              ErrorChoice::Cancel => Step::Cancel,
              _ => {
                  self.done += size;
                  Step::Skipped
              }
          }
      }

      fn retry<T>(&mut self, path: &Path, mut op: impl FnMut() -> io::Result<T>) -> Result<T, Step> {
          loop {
              match op() {
                  Ok(v) => return Ok(v),
                  Err(e) => match self.h.error(path, &e) {
                      ErrorChoice::Retry => {}
                      ErrorChoice::Skip => return Err(Step::Skipped),
                      ErrorChoice::Cancel => return Err(Step::Cancel),
                  },
              }
          }
      }
  }

  fn info(path: &Path, m: &Metadata) -> FileInfo {
      FileInfo { path: path.to_path_buf(), size: m.len(), mtime: m.modified().unwrap_or(SystemTime::UNIX_EPOCH) }
  }

  fn part_path(dst: &Path) -> PathBuf {
      let mut name = dst.file_name().unwrap_or_default().to_os_string();
      name.push(PART);
      dst.with_file_name(name)
  }

  /// Canonical parent + own name: resolves `..` and symlinked parents, not the entry itself.
  fn absolute(p: &Path) -> PathBuf {
      match (p.parent().and_then(|d| d.canonicalize().ok()), p.file_name()) {
          (Some(dir), Some(name)) => dir.join(name),
          _ => p.to_path_buf(),
      }
  }

  /// Bytes under `p`, not following symlinks (for the progress bar).
  fn tree_size(p: &Path) -> u64 {
      match fs::symlink_metadata(p) {
          Ok(m) if m.is_dir() => fs::read_dir(p)
              .map(|rd| rd.flatten().map(|e| tree_size(&e.path())).sum())
              .unwrap_or(0),
          Ok(m) if m.is_file() => m.len(),
          _ => 0,
      }
  }
  ```
  Notes:
  - `copy_tree_keeps_symlinks` compares `read_link` with `outside`. The symlink was created with that absolute path, so the comparison holds.
  - If `plan_rejects_into_itself` fails on `tempdir` paths, the likely cause is `/tmp` canonicalization: the src is compared after canonicalizing its parent, so this should hold.

- [ ] **Step 4: Run the tests and confirm they pass**

  Run: `cargo test -p shagoff-core --lib ops`

  Expected: 11 passed.

- [ ] **Step 5: Commit**
  ```bash
  cargo fmt --all && cargo clippy -p shagoff-core --all-targets -- -D warnings
  git add crates/core Cargo.lock && git commit -m "core: file copy engine (plan, conflicts, cancel, part files)"
  ```

---

### Task 3: core — move and delete

**Files:**
- Modify: `crates/core/src/ops.rs`

**Interfaces:**
- Produces: `pub fn delete(paths: &[PathBuf], permanent: bool, h: &mut dyn Handler) -> Report`. Move goes through `transfer(Method::Move, ..)`, which is already implemented in Task 2.

- [ ] **Step 1: Write the failing tests** (add a `todo!()` stub `delete`):
  ```rust
  #[test]
  fn move_renames_on_same_fs() {
      let d = tempfile::tempdir().unwrap();
      let a = d.path().join("a");
      write(&a, "x");
      let to = d.path().join("to");
      fs::create_dir(&to).unwrap();
      let r = transfer(Method::Move, &plan(&[a.clone()], &to).unwrap(), &mut Script::default());
      assert!(!a.exists());
      assert_eq!(read(&to.join("a")), "x");
      assert_eq!(r.completed, [a]);
  }

  #[test]
  fn move_merges_into_existing_dir() {
      let d = tempfile::tempdir().unwrap();
      write(&d.path().join("src/dir/new.txt"), "new");
      write(&d.path().join("to/dir/keep.txt"), "keep");
      let src = d.path().join("src/dir");
      transfer(Method::Move, &plan(&[src.clone()], &d.path().join("to")).unwrap(), &mut Script::default());
      assert!(!src.exists());
      assert_eq!(read(&d.path().join("to/dir/new.txt")), "new");
      assert_eq!(read(&d.path().join("to/dir/keep.txt")), "keep");
  }

  #[test]
  fn move_keeps_skipped_files_in_source() {
      let d = tempfile::tempdir().unwrap();
      write(&d.path().join("src/dir/a"), "new-a");
      write(&d.path().join("src/dir/b"), "new-b");
      write(&d.path().join("to/dir/a"), "old-a");
      let src = d.path().join("src/dir");
      let mut h = Script { conflicts: vec![Resolution::Skip], ..Default::default() };
      let r = transfer(Method::Move, &plan(&[src.clone()], &d.path().join("to")).unwrap(), &mut h);
      assert_eq!(read(&src.join("a")), "new-a"); // skipped → still in source
      assert!(!src.join("b").exists()); // moved
      assert_eq!(read(&d.path().join("to/dir/b")), "new-b");
      assert_eq!(read(&d.path().join("to/dir/a")), "old-a");
      assert!(r.completed.is_empty()); // the dir was not fully moved
  }

  #[test]
  fn delete_permanent_does_not_follow_symlinks() {
      let d = tempfile::tempdir().unwrap();
      let outside = d.path().join("outside");
      write(&outside.join("keep.txt"), "keep");
      let victim = d.path().join("victim");
      write(&victim.join("sub/x"), "x");
      symlink(&outside, victim.join("link")).unwrap();
      let file = d.path().join("f");
      write(&file, "f");
      let r = delete(&[victim.clone(), file.clone()], true, &mut Script::default());
      assert!(!victim.exists() && !file.exists());
      assert_eq!(read(&outside.join("keep.txt")), "keep");
      assert_eq!(r.completed, [victim, file]);
  }

  #[test]
  fn delete_error_skip_continues() {
      let d = tempfile::tempdir().unwrap();
      let missing = d.path().join("missing");
      let file = d.path().join("f");
      write(&file, "f");
      let mut h = Script { errors: vec![ErrorChoice::Skip], ..Default::default() };
      let r = delete(&[missing.clone(), file.clone()], true, &mut h);
      assert_eq!(h.errored, [missing]);
      assert_eq!(r.completed, [file]);
  }
  ```

- [ ] **Step 2: Run the tests and confirm they fail**

  Run: `cargo test -p shagoff-core --lib ops`

  Expected: the 2 `delete_*` tests fail (`todo!()`). The 3 `move_*` tests should already pass, because move was built in Task 2. That is expected: they lock in the behaviour.

- [ ] **Step 3: Implement**
  ```rust
  /// F8 (trash) / Shift+F8 (permanent, never following symlinks). Progress counts items.
  pub fn delete(paths: &[PathBuf], permanent: bool, h: &mut dyn Handler) -> Report {
      let mut report = Report::default();
      let total = paths.len() as u64;
      for (i, p) in paths.iter().enumerate() {
          if h.cancelled() {
              report.cancelled = true;
              break;
          }
          loop {
              let r = if permanent { remove(p) } else { trash::delete(p).map_err(io::Error::other) };
              match r {
                  Ok(()) => {
                      report.completed.push(p.clone());
                      break;
                  }
                  Err(e) => match h.error(p, &e) {
                      ErrorChoice::Retry => {}
                      ErrorChoice::Skip => break,
                      ErrorChoice::Cancel => {
                          report.cancelled = true;
                          return report;
                      }
                  },
              }
          }
          h.progress(i as u64 + 1, total, p);
      }
      report
  }

  /// `remove_dir_all` does not follow symlinks (std ≥ 1.58); a link is removed as a file.
  fn remove(p: &Path) -> io::Result<()> {
      if fs::symlink_metadata(p)?.is_dir() { fs::remove_dir_all(p) } else { fs::remove_file(p) }
  }
  ```

- [ ] **Step 4: Run the tests and confirm they pass**

  Run: `cargo test -p shagoff-core --lib`

  Expected: 61 + 11 + 5 = 77 passed.

- [ ] **Step 5: Commit**
  ```bash
  cargo fmt --all && cargo clippy -p shagoff-core --all-targets -- -D warnings
  git add crates/core && git commit -m "core: move (rename or copy+delete) and delete (trash/permanent)"
  ```

---

### Task 4: keymap — operation keys

**Files:**
- Modify: `crates/app/src/keymap.rs`
- Modify: `crates/app/src/app.rs` (temporary no-op arm)

**Interfaces:**
- Produces: `Action::{Copy, Move, Rename, Mkdir, Delete, DeletePermanent}`.

- [ ] **Step 1: Write the failing test**
  ```rust
  #[test]
  fn operation_keys() {
      const SHIFT: Modifiers = Modifiers::SHIFT;
      assert_eq!(named(Named::F5, NONE), Some(Action::Copy));
      assert_eq!(named(Named::F6, NONE), Some(Action::Move));
      assert_eq!(named(Named::F6, SHIFT), Some(Action::Rename));
      assert_eq!(named(Named::F2, NONE), Some(Action::Rename));
      assert_eq!(named(Named::F7, NONE), Some(Action::Mkdir));
      assert_eq!(named(Named::F8, NONE), Some(Action::Delete));
      assert_eq!(named(Named::Delete, NONE), Some(Action::Delete));
      assert_eq!(named(Named::F8, SHIFT), Some(Action::DeletePermanent));
      assert_eq!(named(Named::Delete, SHIFT), Some(Action::DeletePermanent));
  }
  ```

- [ ] **Step 2: Run the test and confirm it fails**

  Run: `cargo test -p shagoff-commander keymap`

  Expected: `operation_keys` fails.

- [ ] **Step 3: Implement** — add these rows to the named-key match:
  ```rust
              (Named::F5, false, false) => Action::Copy,
              (Named::F6, false, false) => Action::Move,
              (Named::F6, false, true) | (Named::F2, false, false) => Action::Rename,
              (Named::F7, false, false) => Action::Mkdir,
              (Named::F8 | Named::Delete, false, false) => Action::Delete,
              (Named::F8 | Named::Delete, false, true) => Action::DeletePermanent,
  ```
  Also add the temporary no-op arm `Action::Copy | ... | Action::DeletePermanent => {}` to `App::act`.

- [ ] **Step 4: Run the tests and confirm they pass**

  Run: `cargo test -p shagoff-commander keymap`

  Expected: 10 passed.

- [ ] **Step 5: Commit**
  ```bash
  cargo fmt --all && cargo clippy --all-targets -- -D warnings
  git add crates/app && git commit -m "app: TC operation keys (F5-F8, Shift+F6, F2, Delete)"
  ```

---

### Task 5: app — the `Dialog` machine, input/confirm dialogs, F7

**Files:**
- Create: `crates/app/src/dialogs.rs` (the `Dialog` enum and its `view`)
- Modify: `crates/app/src/app.rs`, `crates/app/src/main.rs` (`mod dialogs;`), `crates/app/src/view.rs` (F-key buttons enabled)
- Modify: the i18n files

**Interfaces:**
- Produces:
  - `pub enum Dialog { Mask { side, select, input }, Input { op: InputOp, side, sources: Vec<PathBuf>, input: String }, ConfirmDelete { side, permanent, paths: Vec<PathBuf> }, Conflict { .. }, Error { .. } }`. The `Conflict` and `Error` variants are added in Task 6.
  - `pub enum InputOp { Copy, Move, Mkdir, Rename }`.
  - `App.dialog: Option<Dialog>` replaces `mask_dialog`.
  - Messages: `DialogInput(String)`, `DialogSubmit`, `DialogCancel`. `MaskInput`, `MaskSubmit` and `MaskCancel` are renamed to these, and `route_event` now sends `DialogCancel` on Escape.

UI glue. It is verified by `just verify`, the existing route_event tests (renamed message) and the manual checklist.

- [ ] **Step 1: i18n** (en / ru). Add these keys:
  - `copy-to = Copy { $what } to:` / `Копировать { $what } в:`
  - `move-to = Move { $what } to:` / `Переместить { $what } в:`
  - `n-files = { $n } files` / `{ $n } файлов` (count, used when there are 2 or more)
  - `mkdir = New folder` / `Новый каталог`
  - `rename = Rename` / `Переименовать`
  - `delete-trash = Move { $what } to trash?` / `Удалить { $what } в корзину?`
  - `delete-permanent = Delete { $what } permanently? This cannot be undone.` / `Удалить { $what } безвозвратно? Это нельзя отменить.`
  - `delete = Delete` / `Удалить`

- [ ] **Step 2: Move the dialog state into `dialogs.rs`**
  - `Dialog` and `InputOp` as listed above.
  - `pub fn view<'a>(d: &'a Dialog, input_id: &widget::Id) -> Element<'a, Message>`. It builds a `widget::dialog()` for each variant:
    - **Mask:** as today.
    - **Input:** the title from `InputOp` (copy-to / move-to with `what` = the source file name if there is one, else `n-files`; mkdir; rename), a text input with the given id, then OK and Cancel.
    - **ConfirmDelete:** the body is `delete-trash` or `delete-permanent`. The primary button is `button::destructive(fl!("delete"))` when permanent, otherwise `button::suggested`. Then Cancel.
  - Move the existing mask view code out of `App::dialog()` into this function. `App::dialog()` becomes `self.dialog.as_ref().map(|d| dialogs::view(d, &self.input_id))`. Rename `mask_input_id` to `input_id`.

- [ ] **Step 3: app.rs — open dialogs from actions**
  - `Copy` / `Move`: `let sources = panel.targets()`; if empty → no-op. Open `Input { op, side, sources, input: <other pane active cwd> + "/" }` and focus the input.
  - `Mkdir` → `Input { op: Mkdir, input: "" }`.
  - `Rename` → `Input { op: Rename, sources: vec![current path], input: current name }`, only if the current entry exists and is not `..`.
  - `Delete` / `DeletePermanent`: if `targets()` is not empty, open `ConfirmDelete`.
  - **Gate** in `Message::Key`: while `self.dialog.is_some()`, ignore actions, except that `Action::Enter` on `ConfirmDelete` means `DialogSubmit`.
  - `DialogSubmit` for each variant:
    - **Mask** → as before.
    - **Input Mkdir** → `fs::create_dir_all(cwd.join(input.trim()))`. On error, put the text in `tab.error`. On success, `load(side, cwd, Some(first path component of input))`.
    - **Input Copy / Move / Rename** and **ConfirmDelete** → `self.start_job(...)` (Task 6). In this task, call a stub `start_job` that does nothing and returns `Task::none()`.
  - `DialogCancel` → `self.dialog = None` for now. Task 6 extends it.
  - In `view.rs`, `fkey_bar` turns F5–F8 into `button::standard(..).on_press(Message::Key(Action::Copy))` and so on. F3 and F4 stay disabled (phase 6).

- [ ] **Step 4: Verify**

  Run: `just verify`

  Expected: clean, and the route_event tests are updated to `DialogCancel`.

  Then take a screenshot with an uncommitted patch that opens the Copy input dialog at startup, and revert the patch.

- [ ] **Step 5: Commit**
  ```bash
  git add crates/app && git commit -m "app: dialog state machine, operation input/confirm dialogs, F7 mkdir"
  ```

---

### Task 6: app — jobs: worker thread, progress, conflict, error

**Files:**
- Create: `crates/app/src/jobs.rs`
- Modify: `crates/app/src/app.rs`, `crates/app/src/dialogs.rs`, `crates/app/src/main.rs` (`mod jobs;`), i18n

**Interfaces:**
- Consumes: `ops::{plan, transfer, delete, Handler, Method, Resolution, ErrorChoice, FileInfo, Report, PlanError}`.
- Produces:
  - `jobs::Job { Transfer { method, pairs }, Delete { paths, permanent } }`.
  - `jobs::Event { Progress { done, total, current: PathBuf }, Conflict { src: FileInfo, dst: FileInfo, reply: mpsc::Sender<Resolution> }, Error { path: PathBuf, error: String, reply: mpsc::Sender<ErrorChoice> }, Finished(Report) }`.
  - `jobs::spawn(job) -> (Arc<AtomicBool>, impl Stream<Item = Event>)`.
  - `Message::Op(jobs::Event)`, `Message::Resolve(Resolution)`, `Message::ErrorAnswer(ErrorChoice)`, `Message::CancelJob`.

- [ ] **Step 1: jobs.rs**
  ```rust
  //! Runs an `ops` job on a worker thread; events go to the UI over a channel, answers come back blocking.

  use cosmic::iced::futures::channel::mpsc as fmpsc;
  use shagoff_core::ops::{self, ErrorChoice, FileInfo, Handler, Method, Report, Resolution};
  use std::path::{Path, PathBuf};
  use std::sync::atomic::{AtomicBool, Ordering};
  use std::sync::{Arc, mpsc};
  use std::time::{Duration, Instant};

  pub enum Job {
      Transfer { method: Method, pairs: Vec<(PathBuf, PathBuf)> },
      Delete { paths: Vec<PathBuf>, permanent: bool },
  }

  #[derive(Debug, Clone)]
  pub enum Event {
      Progress { done: u64, total: u64, current: PathBuf },
      Conflict { src: FileInfo, dst: FileInfo, reply: mpsc::Sender<Resolution> },
      Error { path: PathBuf, error: String, reply: mpsc::Sender<ErrorChoice> },
      Finished(Arc<Report>),
  }

  struct ChannelHandler {
      tx: fmpsc::UnboundedSender<Event>,
      cancel: Arc<AtomicBool>,
      last: Option<Instant>,
  }

  const PROGRESS_EVERY: Duration = Duration::from_millis(100);

  impl Handler for ChannelHandler {
      fn progress(&mut self, done: u64, total: u64, current: &Path) {
          if self.last.is_some_and(|t| t.elapsed() < PROGRESS_EVERY) && done < total {
              return;
          }
          self.last = Some(Instant::now());
          let _ = self.tx.unbounded_send(Event::Progress { done, total, current: current.to_path_buf() });
      }
      fn conflict(&mut self, src: &FileInfo, dst: &FileInfo) -> Resolution {
          let (reply, rx) = mpsc::channel();
          let _ = self.tx.unbounded_send(Event::Conflict { src: src.clone(), dst: dst.clone(), reply });
          rx.recv().unwrap_or(Resolution::Cancel) // UI dropped the sender → cancel, never hang
      }
      fn error(&mut self, path: &Path, err: &std::io::Error) -> ErrorChoice {
          let (reply, rx) = mpsc::channel();
          let _ = self.tx.unbounded_send(Event::Error { path: path.to_path_buf(), error: err.to_string(), reply });
          rx.recv().unwrap_or(ErrorChoice::Cancel)
      }
      fn cancelled(&self) -> bool {
          self.cancel.load(Ordering::Relaxed)
      }
  }

  pub fn spawn(job: Job) -> (Arc<AtomicBool>, fmpsc::UnboundedReceiver<Event>) {
      let (tx, rx) = fmpsc::unbounded();
      let cancel = Arc::new(AtomicBool::new(false));
      let mut h = ChannelHandler { tx: tx.clone(), cancel: cancel.clone(), last: None };
      std::thread::spawn(move || {
          let report = match job {
              Job::Transfer { method, pairs } => ops::transfer(method, &pairs, &mut h),
              Job::Delete { paths, permanent } => ops::delete(&paths, permanent, &mut h),
          };
          let _ = tx.unbounded_send(Event::Finished(Arc::new(report)));
      });
      (cancel, rx)
  }
  ```
  `Report` is wrapped in an `Arc` because `Message` must be `Clone` and `Report` is not.

- [ ] **Step 2: app.rs**
  - Add `pub struct Running { side: usize, title: OpKind, done: u64, total: u64, current: String, cancel: Arc<AtomicBool>, focus: Option<(usize, String)> }` (`OpKind { Copy, Move, Delete }`) and `App.job: Option<Running>`.
  - `start_job(side, kind, job, focus) -> Task<Message>`: set `self.job`, call `jobs::spawn`, return `Task::run(rx, |e| cosmic::Action::App(Message::Op(e)))`.
  - `DialogSubmit` for Input Copy/Move: `dest = cwd.join(input.trim())`, then `ops::plan(&sources, &dest)`. On `Err`, put the localized text (`plan-into-itself`, `plan-same-file`) into `tab.error`. On `Ok`, call `start_job(Transfer { method, pairs })`.
  - For Rename, if the name is unchanged do nothing. Otherwise `pairs = vec![(src, cwd.join(new))]` (no `plan`, so typing an existing dir name doesn't move the file into it), `focus = Some((side, new))`.
  - For ConfirmDelete, call `start_job(Delete)`.
  - `Message::Op(Event::Progress)` → update the `job` fields.
  - `Message::Op(Event::Conflict | Event::Error)` → `self.dialog = Some(Dialog::Conflict{..} | Dialog::Error{..})`.
  - `Message::Resolve(r)` → take `Dialog::Conflict { reply, .. }` and send `r`. `Message::ErrorAnswer(c)` → the same for `Error`.
  - `Message::Op(Event::Finished(report))`: `job.take()`, then `unmark(&report.completed)` in the source pane's active tab, then reload both panes' active tabs. The source pane uses `job.focus` if it is set. Return `Task::batch`.
  - `DialogCancel`:
    - `Conflict` → send `Cancel`;
    - `Error` → send `Cancel`;
    - other dialogs → close;
    - no dialog but a `job` is running → `cancel.store(true)`.

    Any dropped `Sender` also makes the worker see Cancel.
  - `CancelJob` (the button) → `cancel.store(true)`.
  - Gate: panel keys are ignored while `self.job.is_some()` too.
  - `App::dialog()`: if `self.dialog` is set, show it. Otherwise, if a job is running, show the progress dialog. Otherwise show nothing.

- [ ] **Step 3: dialogs.rs** — add these variants and their views:
  - **Progress:** title from `OpKind` (`copying` / `moving` / `deleting`); body is the current name; control is `progress_bar::determinate_linear(done as f32 / total.max(1) as f32)` plus text — `format::size(done)` "из" `format::size(total)` for bytes, or counts for delete. Primary button: Cancel → `CancelJob`.
  - **Conflict:** title `file-exists`; body is two lines, «Новый: {size} {date}» and «Существующий: {size} {date}». Buttons: Replace, Skip, Replace all, Skip all, Replace older, Cancel → `Resolve(..)`. libcosmic's dialog takes a primary action, a secondary action and a tertiary action, so put the rest in a `row!` passed as the `control`.
  - **Error:** title `op-error`; body is path + error. Buttons: Retry, Skip, Cancel → `ErrorAnswer(..)`.
  - i18n keys: `copying`, `moving`, `deleting`, `file-exists`, `new-file`, `existing-file`, `replace`, `skip`, `replace-all`, `skip-all`, `replace-older`, `op-error`, `retry`, `plan-into-itself`, `plan-same-file`.

- [ ] **Step 4: Verify**

  Run: `just verify`

  Then check with a real run in a scratch dir. Create a test tree in the session scratchpad and launch the app with `-- <scratch>`. Take a screenshot after starting a copy through an uncommitted patch that auto-starts a job, and revert the patch.

  Real keyboard testing is the user's manual checklist.

- [ ] **Step 5: Commit**
  ```bash
  git add crates/app && git commit -m "app: file operation jobs with progress, conflict and error dialogs"
  ```

---

### Task 7: Docs and PR

- Add this checklist to `TESTING.md`:
  ```markdown
  ### Фаза 5: файловые операции
  - [ ] F5: диалог с путём другой панели; копирует файл/отмеченные/папку с вложенными; права и дата сохранены
  - [ ] F5 на `..` без отметок — ничего; F5 папки в её же подпапку — ошибка, ничего не тронуто
  - [ ] Конфликт: Заменить / Пропустить / Заменить все / Пропустить все / Заменить старые / Отмена работают
  - [ ] Большой файл (1+ ГБ): прогресс идёт, «Отмена» и Escape останавливают; заменяемый файл цел, `.shagoff-part` не осталось
  - [ ] F6 в каталог на том же диске — мгновенно; на другой диск (`/tmp` ↔ `~`, если разные ФС) — копирование и удаление
  - [ ] Shift+F6 / F2 — переименование, курсор на новом имени; занятое имя → диалог конфликта
  - [ ] F7: `new`, `a/b/c` — создано, курсор на созданном
  - [ ] F8 / Delete — в корзину (видно в корзине COSMIC Files), подтверждение, Enter подтверждает
  - [ ] Shift+F8 / Shift+Delete — безвозвратно, красная кнопка; симлинк на папку удаляется без цели
  - [ ] После операции отметки сняты с обработанных, обе панели перечитаны
  - [ ] Нет прав (копировать из `/root`) → диалог ошибки: Повторить/Пропустить/Отмена
  - [ ] Ctrl+T — новая вкладка без отметок
  - [ ] Во время операции клавиши панелей не действуют
  ```
- ROADMAP:
  - tick phase 5;
  - tick the tech-debt item about Ctrl+T marks;
  - add to the backlog: background queue, conflict «Переименовать» / «Сохранить оба», Shift+F5, dir mtime on copy.
- CLAUDE.md:
  - add `ops` to "Implemented so far";
  - add the files `dialogs.rs` and `jobs.rs`;
  - note that the engine is our own, not cosmic-files (spec deviation).
- Update `architecture-design.md`, section "Что копируем из cosmic-files", with a pointer to the phase 5 spec deviation.
- Commit, push, `gh pr create --title "File operations (phase 5)"` with the spec and plan links, wait for CI, and merge only after the user approves.
