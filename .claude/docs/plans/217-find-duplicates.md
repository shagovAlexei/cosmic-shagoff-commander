# 217. Find duplicates — plan

Spec: [../specs/217-find-duplicates.md](../specs/217-find-duplicates.md). Branch `feat/find-duplicates`, 0.136.0.

1. core `search`: `Dupes { name, size, content }` and `duplicates(paths, by, stop, each) -> Vec<Vec<PathBuf>>`, tests first (name only, size only, content splits a same-size group, symlinks / dirs out, stop → empty).
2. app `find::spawn(id, root, q, dupes: Option<Dupes>)`: with dupes collect, then `FindEvent::Group` per group; hashing progress through `FindEvent::Dir`.
3. `dialogs::Find` + `FindPrefs`: `dupes`, `dup_name`, `dup_size`, `dup_content`; `groups: Vec<usize>` (start of each group in `results`); checkboxes, divider in the list; error when none ticked.
4. i18n en / ru, tc-reference Alt+F7 row, TESTING.md section, ROADMAP 217, README(s) if they list search features, CHANGELOG, version.
