# 173. Toolbar drag + working dir — plan

Spec: `.claude/docs/specs/173-toolbar-drag.md`.

1. core `reorder.rs`: `Reorder { hover, grab, vacated }` + `hover(at, &mut Vec<T>) -> Option<usize>`,
   `grab()`, `release()`, `left(i)`, `scrolled()`; tests moved from hotlist's drag test.
2. hotlist.rs uses `Reorder` (its test still passes).
3. core `cmdline::start_dir` + test.
4. config `ToolButton.dir`; toolbar dialog field `ToolMsg::Dir`; `run_button` runs in `start_dir`.
5. App `tool_drag: Reorder`; `toolbar::bar` wraps each button in a `mouse_area` (enter / exit);
   subscription for Left press / release while Shift or held; `Message::ToolDrag(..)`; moves write
   the config; `Tool(i)` ignored while Shift / held.
6. i18n en/ru, tc-reference row 64, TESTING.md, ROADMAP, version 0.x → minor.
