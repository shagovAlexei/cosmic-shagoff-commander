# 198. Копирование по стороне и фокус

TC: `cm_TransferLeft`, `cm_TransferRight`, `cm_FocusLeft`, `cm_FocusRight`, `cm_FocusCmdLine`.

- `Action::TransferLeft` / `TransferRight`: источник — противоположная панель, она становится активной, дальше `Action::Copy`. Фокус — только смена активной панели / курсор в командной строке (`cmd_focus`).
- Не делаем: перенос (`cm_TransferLeft` у TC только копирует), клавиши по умолчанию.
