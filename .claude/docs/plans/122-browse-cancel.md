# 122 — план

1. `mount::browse(&cancel)` через `talk_until`.
2. `browsing: Option<Arc<AtomicBool>>`; `ConnectBrowse` запускает / останавливает; `DialogCancel`, `Connected` останавливают; `Browsed` без обзора или `Cancelled` — молча.
3. Тест `browse_stops_by_its_button_or_by_closing_the_dialog`.
