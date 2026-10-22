# 186. Переместить в архив

TC: `cm_MoveToArchive` (Alt+Shift+F5) — упаковать и удалить исходные. У нас флажок «Переместить в архив (удалить исходные)» в Alt+F5 уже был (фаза 11; исходные удаляются после успешной записи архива, `jobs::Pack.move_after`).

- `Action::MoveToArchive`: Alt+Shift+F5, Файлы → «Переместить в архив…», справка, кнопка `cm_MoveToArchive`. Открывает то же окно, что Alt+F5 (`dialog_for`: одна ветка с `Pack`), с `move_after: true`; флажок можно снять.
- В архиве и на дереве Ctrl+F8 — как Alt+F5 (`read_only`, `on_files`).
