# 148 — план

Спек: `specs/148-terminal.md`. Две фазы, два PR.

## Фаза A — внешний терминал (148)

1. core `terminals`: `exec_argv` (Exec= с кавычками, без `%U`), `installed` (`TerminalEmulator`, без `NoDisplay` и `--server`), `default_of` (`gio mime x-scheme-handler/terminal` → `x-terminal-emulator` → первый), `run_prefix` (флаг «выполни» по программе), `scan`. Тесты.
2. config `terminal_app`; `Setting::TerminalApp`.
3. app: `terminals` / `term_default` (`scan_terminals` при старте, в фоне), `terminal()`, `terminal_prefix()` (ручной `terminal` главнее), `open_terminal()`; `Action::OpenTerminal` / `PickTerminal`; `ListKind::Terminals` (галочка, курсор на выбранном). Подменю libcosmic не взяли: во всплывающих меню Wayland не проверить, вложенных у нас нет.
4. Меню «Команды», кнопка `cm_ExecuteDOS`; строки en / ru; tc-reference, README, ROADMAP, TESTING § 148.

## Фаза B — встроенный терминал (149)

1. `alacritty_terminal` в зависимостях; `crates/app/src/term/` из cosmic-term (terminal, terminal_box, terminal_theme, mouse_reporter), один терминал, без pane_grid / вкладок / профилей / поиска.
2. config `term_builtin`, `term_shell`, `term_height`; core: оболочки из `/etc/shells` без повторов, «простаивает ли» (`tcgetpgrp`), строка `cd` с кавычками — тесты.
3. view: вместо командной строки; верхняя кромка тянется; Ctrl+O — на всю высоту / фокус.
4. Следование за каталогом панели, перезапуск после `exit`; меню «Вид → Встроенный терминал», «Команды → Оболочка…».
