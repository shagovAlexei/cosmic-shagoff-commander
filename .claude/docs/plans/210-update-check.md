# 210 plan

1. `core::update`: `due`, `deb_name`, `can_install`, `expected_sum`, `download`, `install` (tests: `due_once_a_day`, `the_deb_and_its_sum`).
2. `Config.check_updates`, `State.update_checked` / `update_skip`; Settings toggle.
3. `Dialog::Update` + `UpdateStage`; messages `StartupUpdate`, `UpdateNow`, `UpdateSkip`, `UpdateDone`; the task in `init`.
4. Test `startup_update_check_offers_a_newer_release_once`; headless check with a fake `curl` in `PATH` answering `v9.0.0`.
