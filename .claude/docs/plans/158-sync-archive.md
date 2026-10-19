# 158 — план

Спек: `specs/158-sync-archive.md`.

1. core sync: `list` из оглавления для папки в архиве; `ArchivePlan` / `archive_plan` — тест на zip.
2. app: `read_only` — Sync только при обеих сторонах в архивах; `start_sync` — задачи по `archive_plan`. Тест.
3. repack: `Shown` — пути через архив в вопросах (регрессионный тест).
4. tc-reference, ROADMAP, TESTING § 158; 0.72.0.
