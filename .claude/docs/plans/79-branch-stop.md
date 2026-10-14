# 79. Branch stop — plan

1. core `listing::branch` + `stop`, `Option` result; test (stop set → `None`).
2. `app.rs`: `Tab::branch_stop`, set on new read / leaving / Esc; `BranchListed.paths: Option`; tests `esc_stops_a_branch_read`, leaving stops.
3. i18n (shorter hint, «stopped»), tc-reference, ROADMAP, TESTING § 79; version 0.22.0; headless check (menu Вид → Ветвь on `/`, Esc).
