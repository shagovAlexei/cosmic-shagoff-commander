# 62. План: быстрый просмотр Ctrl+Q

Спек: `.claude/docs/specs/62-quick-view.md`.

1. keymap Ctrl+Q → `Action::QuickView`; меню «Вид», справка, `cm_SrcQuickview`.
2. `Lister.quick`; `App::viewer()` вместо `lister.is_some()` там, где решается, кому клавиши;
   Esc закрывает только полноэкранный.
3. `quick_follow` в `update`; читает файл в фоне (id отбрасывает старые ответы).
4. view: просмотр на месте неактивной панели; кнопки F7 / N / P / Esc — только у F3.
5. Версия 0.6.0; tc-reference, TESTING, ROADMAP.
