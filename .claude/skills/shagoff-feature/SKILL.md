---
name: shagoff-feature
description: Use when starting any new feature, MVP phase, or non-trivial change in Shagoff Commander — runs the project cycle brainstorm → spec → plan → branch → TDD → verify → review → PR, with docs under .claude/docs.
---

# Feature cycle for Shagoff Commander

Every feature and every MVP phase goes through these steps in order. Do not skip a step. Each step's gate is the user's approval.

1. **Brainstorm.** Invoke `superpowers:brainstorming`. Before you ask any questions, read:
   - `CLAUDE.md`
   - `.claude/docs/ROADMAP.md`
   - the matching rows in `.claude/docs/tc-reference.md`

   If something should behave "like TC", tc-reference.md is the source of truth. If the feature adds keys or behaviour, update tc-reference.md.
2. **Spec.** Write it to `.claude/docs/specs/NN-<topic>.md`, where NN is the same number its plan gets (phase number for MVP phases); no dates in file names. This location overrides the brainstorming default `docs/superpowers/specs`. The user reviews the spec.
3. **Plan.** Invoke `superpowers:writing-plans`. Save the plan to `.claude/docs/plans/NN-<topic>.md`, where NN is the next free number.
4. **Branch.**
   ```sh
   git switch main && git pull
   git switch -c feat/<topic>
   ```
   Never commit to `main` directly.
5. **Implement.**
   - Logic goes into `crates/core`, written test-first with `superpowers:test-driven-development`.
   - `crates/app` stays thin: map the message, call the core, render.
   - Every new UI string goes in `fl!` with both `en` and `ru` entries.
6. **Verify.** `just verify` must pass. Then look at the real UI yourself with `.claude/skills/shagoff-feature/headless.sh` (invisible sway + `wtype` + `grim`; never the user's desktop or config): `start`, `panes L R`, `key -k F7`, `shot name`, then Read the png; check every new dialog at the default 1036x530 size for clipped buttons. Letter shortcuts (Ctrl+…) can't be sent this way; the user checks those. Mouse: `click X Y [right]` (virtual pointer through `vpointer/`, headless sway only) — clicks, right-click menus, popups. Add the manual check to `TESTING.md`; the user walks through it by hand.
7. **Review.** Run `/code-review` and fix what it confirms.
8. **PR.**
   - Update `ROADMAP.md` by ticking the phase or feature.
   - If the architecture changed, update `CLAUDE.md`.
   - Bump `version` in the root `Cargo.toml` (`cargo build` updates `Cargo.lock`): feature → minor, fix → patch (see CLAUDE.md "Versions").
   - Then:
     ```sh
     git push -u origin feat/<topic>
     gh pr create --fill
     ```
     The body links the spec and the plan.
   - Wait for CI with `gh pr checks --watch`.
   - Merge only after the user says so: `gh pr merge --merge --delete-branch`.
   - Then tag the bumped version on `main` and push the tag (`vX.Y.Z`): the `release` workflow attaches the .deb. Check it with `gh run watch` / `gh release view`.

Bug fix: use `superpowers:systematic-debugging`. Add a regression test named `regression_<what>` in core, add a row in `TESTING.md`, and work on branch `fix/<topic>`.

X11 check: after `headless.sh start`, ask sway for its Xwayland display (`SWAYSOCK=$XDG_RUNTIME_DIR/sway-ipc.*.<sway pid>.sock swaymsg exec 'echo $DISPLAY > /tmp/shagoff-headless/display'`), kill the app and start `target/debug/shagoff-commander` with `env -u WAYLAND_DISPLAY DISPLAY=:N` and the same `XDG_*_HOME`; `click` / `shot` work as usual. Under X11 libcosmic draws menus inside the window, not as popups.
