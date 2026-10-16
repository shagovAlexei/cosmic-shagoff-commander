# 98. Changing the group — plan

Spec: `../specs/98-chgrp.md`.

1. Core: `owners::my_groups`, `Owners::groups`, `props::chgrp` (test: own group through a tree, links not followed; another group refused).
2. App: `Props.{gids, group_names, group, group_changed}`, `Message::PropsGroup`, chgrp before chmod on OK (test).
