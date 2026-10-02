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
2. **Spec.** Write it to `.claude/docs/specs/YYYY-MM-DD-<topic>.md`. This location overrides the brainstorming default `docs/superpowers/specs`. The user reviews the spec.
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
6. **Verify.** `just verify` must pass. Then run the app with `cargo run -p shagoff-commander` and walk through the feature by hand. Add its manual check to `TESTING.md`.
7. **Review.** Run `/code-review` and fix what it confirms.
8. **PR.**
   - Update `ROADMAP.md` by ticking the phase or feature.
   - If the architecture changed, update `CLAUDE.md`.
   - Then:
     ```sh
     git push -u origin feat/<topic>
     gh pr create --fill
     ```
     The body links the spec and the plan.
   - Wait for CI with `gh pr checks --watch`.
   - Merge only after the user says so: `gh pr merge --merge --delete-branch`.

Bug fix: use `superpowers:systematic-debugging`. Add a regression test named `regression_<what>` in core, add a row in `TESTING.md`, and work on branch `fix/<topic>`.
