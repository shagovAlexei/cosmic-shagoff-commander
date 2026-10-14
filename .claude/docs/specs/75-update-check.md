# 75. Check for updates in About

**Goal:** from Help → About, see whether a newer release exists, without leaving the program.

- A «Check for updates» button right under the version in the About drawer (libcosmic's `about` widget drawn in two parts around it). Nothing is sent
  until it is pressed (no check at startup); opening About again forgets the last answer.
- Pressed: `curl -sSfL -m 10` on `api.github.com/repos/<repo>/releases/latest`, off the UI thread;
  `tag_name` is pulled out of the JSON by hand (no JSON crate for one field).
- Answer under the button: «Checking…» (button disabled) → «You have the latest version» /
  «Version vX.Y.Z is available» + «Open the release page» (`releases/tag/<tag>` via xdg-open) /
  «Could not check: <curl error, or GitHub's \"message\" (rate limit)>».
- Versions compare as numbers (`0.10.0 > 0.9.0`); a tag that is not `vN.N.N` is never «newer».
- `curl` added to the .deb `Depends`.

Core: `shagoff_core::update::{Status, field, newer, status, check}`. App: `App::update_check`, `Message::{CheckUpdate, UpdateChecked}`.
