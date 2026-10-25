# Changelog

Restored from the merged pull requests and tags (the GitHub releases older than the newest five are deleted by CI; see `.github/workflows/release.yml`). Newest first. Add an entry here in every PR that bumps the version.

## 0.123.2 — 2026-10-25

- fix: «Change date and time» runs in the background: «and everything inside» on a big tree, or a dead network mount, no longer freezes the window ([#293](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/293))

## 0.123.1 — 2026-10-25

- fix: About: «Latest release» (the releases page) in place of «Source code» ([#292](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/292))

## 0.123.0 — 2026-10-25

- feat: Alt+F5 7z with a password: «Encrypt file names too»; a change inside keeps them encrypted; the password is kept for the new archive ([#291](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/291))

## 0.122.0 — 2026-10-25

- feat: built-in terminal: a mouse selection is copied (clipboard and primary), the middle button pastes ([#290](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/290))
- fix: toolbar / Start menu / F12 parameters: a name starting with `-` goes in as `./-name`, never as an option ([#290](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/290))

## 0.121.4 — 2026-10-25

- fix: Alt+Enter as root (pkexec): chmod / chown walk by descriptors, never through a symlink swapped in meanwhile ([#289](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/289))

## 0.121.3 — 2026-10-25

- fix: built-in terminal: no panic when the cursor's line is not laid out yet ([#288](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/288))

## 0.121.2 — 2026-10-25

- fix: Cancel while the update downloads stops it before pkexec; its dir is swept at the next start if the run dies; a relative XDG_CACHE_HOME is ignored; Start menu: a submenu's start and end never swap, a submenu of empty ones is hidden, an unnamed one is «Submenu» everywhere, a bound in the toolbar / F12 runs nothing ([#287](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/287))

## 0.121.1 — 2026-10-25

- fix: update review — the .deb must be the tag's version for amd64, sha256sum read in the C locale, a dir per instance, a stricter tag check; Start menu: deleting a submenu bound takes its partner, empty submenus hidden, an unnamed one shown as «Submenu» ([#286](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/286))

## 0.121.0 — 2026-10-25

- feat: submenus in the Start menu ([#285](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/285))

## 0.120.3 — 2026-10-25

- fix: the update is checked by `sha256sum -c --strict` in ~/.cache, a mismatch is deleted; tag and package name checked before the download ([#284](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/284))

## 0.120.2 — 2026-10-25

- fix: donate text in About (0.120.2) ([#277](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/277))
- ci: keep the five newest releases ([#276](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/276))

## 0.120.1 — 2026-10-25

- fix: description under the name in About (0.120.1) ([#275](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/275))

## 0.120.0 — 2026-10-25

- feat: the update banner names the program and says what it is (0.120.0) ([#274](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/274))

## 0.119.4 — 2026-10-25

- fix: findings of the full code review (0.119.4) ([#273](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/273))

## 0.119.3 — 2026-10-25

- fix: no submenu inside a submenu in the main menu (0.119.3) ([#272](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/272))

## 0.119.2 — 2026-10-25

- fix(security): update download dir is created fresh with mode 0700 (0.119.2) ([#271](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/271))

## 0.119.1 — 2026-10-25

- fix: check for updates at every start, not once a day (0.119.1) ([#270](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/270))

## 0.119.0 — 2026-10-25

- feat: opt-in AppArmor profile in complain mode (0.119.0) ([#269](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/269))

## 0.118.2 — 2026-10-24

- test: fuzz smoke for parsers, zeroize archive passwords (207c) ([#268](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/268))

## 0.118.1 — 2026-10-24

- chore(deps): zip 9 (0.118.1) ([#267](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/267))
- ci(release): SHA-256 sums in the release notes ([#266](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/266))
- docs(readme): Support the project section ([#265](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/265))

## 0.118.0 — 2026-10-24

- feat(update): update check at start with a donate ask (0.118.0) ([#264](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/264))
- docs(roadmap): 210, update check at start ([#263](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/263))

## 0.117.1 — 2026-10-24

- ci: supply-chain protection (207a, 207b, 207d) ([#256](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/256))
- docs(roadmap): 208, paid Pro version ([#255](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/255))
- docs(roadmap): 207, protection of the app ([#254](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/254))

## 0.117.0 — 2026-10-24

- feat(menu): shorter menus through submenus (0.117.0) ([#253](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/253))

## 0.116.0 — 2026-10-24

- feat(view): delete a saved view mode (0.116.0) ([#252](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/252))

## 0.115.0 — 2026-10-24

- feat(sort): sort by Owner / Group columns (0.115.0) ([#251](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/251))
- docs(roadmap): 202 menus into submenus ([#250](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/250))

## 0.114.1 — 2026-10-24

- feat(view): drag the divider of stacked panes (0.114.1) ([#249](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/249))

## 0.114.0 — 2026-10-24

- feat(view): Alt+F3 views with the other viewer (0.114.0) ([#248](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/248))

## 0.113.0 — 2026-10-23

- feat(marks): shrink and exchange selection (0.113.0) ([#247](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/247))

## 0.112.0 — 2026-10-23

- feat(panels): copy to a side, focus actions (0.112.0) ([#246](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/246))

## 0.111.0 — 2026-10-23

- feat(ops): verify copies by reading them back (0.111.0) ([#245](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/245))

## 0.110.1 — 2026-10-23

- feat(files): date dialog with modified / accessed (0.110.1) ([#244](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/244))

## 0.110.0 — 2026-10-23

- feat(files): change the date and time of files (0.110.0) ([#243](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/243))

## 0.109.0 — 2026-10-23

- feat(view): view modes (0.109.0) ([#242](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/242))

## 0.108.1 — 2026-10-23

- feat(thumbs): setting for the disk cache (0.108.1) ([#241](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/241))

## 0.108.0 — 2026-10-23

- feat(thumbs): shared freedesktop thumbnail cache (0.108.0) ([#240](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/240))

## 0.107.0 — 2026-10-23

- feat(view): pick the columns of the full view (0.107.0) ([#239](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/239))

## 0.106.0 — 2026-10-23

- feat(view): separate tree, Ctrl+Shift+F8 (0.106.0) ([#238](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/238))

## 0.105.1 — 2026-10-23

- feat(view): Panels submenu (0.105.1) ([#237](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/237))

## 0.105.0 — 2026-10-23

- feat(view): panels one above the other (0.105.0) ([#236](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/236))
- feat(thumbs): Thumbnails submenu and hotkeys (0.104.1) ([#235](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/235))

## 0.104.1 — 2026-10-23

- feat(thumbs): Thumbnails submenu and hotkeys (0.104.1)

## 0.104.0 — 2026-10-23

- feat(thumbs): thumbnail zoom and reload (0.104.0) ([#234](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/234))

## 0.103.0 — 2026-10-23

- feat(view): window parts, panel at 50 % / 100 % (0.103.0) ([#233](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/233))

## 0.102.1 — 2026-10-23

- fix(state): keep each tab's sort across runs (0.102.1) ([#232](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/232))

## 0.102.0 — 2026-10-23

- feat(view): ignore list and switches, unsorted Ctrl+F7 (0.102.0) ([#231](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/231))

## 0.101.0 — 2026-10-22

- feat(mark): compare dirs with subdirs, show only what differs (0.101.0) ([#230](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/230))

## 0.100.0 — 2026-10-22

- feat(files): move to archive, Alt+Shift+F5 (0.100.0) ([#229](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/229))

## 0.99.0 — 2026-10-22

- feat(files): split a file into parts and combine them back (0.99.0) ([#228](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/228))

## 0.98.0 — 2026-10-22

- feat(files): new file from a template (0.98.0) ([#227](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/227))

## 0.97.0 — 2026-10-22

- feat(tabs): list, background, other panel, all tabs (0.97.0) ([#226](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/226))
- refactor(app): split app.rs into app/ by topic ([#225](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/225))

## 0.96.0 — 2026-10-22

- feat(clipboard): paths and file details (0.96.0) ([#224](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/224))

## 0.95.0 — 2026-10-22

- feat(marks): selection save / load, same name, only selected (0.95.0) ([#223](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/223))

## 0.94.0 — 2026-10-22

- feat(menu): the Start menu (0.94.0) ([#222](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/222))

## 0.93.0 — 2026-10-22

- feat(panels): synchronous dir change, other panel to this dir (0.93.0) ([#221](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/221))

## 0.92.0 — 2026-10-22

- feat(archive): encoding of zip names (0.92.0) ([#220](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/220))

## 0.91.0 — 2026-10-22

- feat(trash): the trash as a dir (0.91.0) ([#219](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/219))

## 0.90.1 — 2026-10-22

- fix(thumbs): the name under the cursor is readable (0.90.1) ([#218](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/218))

## 0.90.0 — 2026-10-22

- feat(panel): edit the path line (176, TC cm_EditPath) ([#217](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/217))
- docs(roadmap): TC 11.57 command audit — what's missing, ranked as phases 176–193 ([#216](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/216))

## 0.89.0 — 2026-10-22

- feat(thumbs): thumbnailers in a bwrap sandbox (175) ([#215](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/215))

## 0.88.1 — 2026-10-22

- fix(lister): a picture in the viewer's own window shows without a key press (0.88.1) ([#214](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/214))

## 0.88.0 — 2026-10-22

- feat(thumbs): PDF, video, SVG thumbnails through the desktop's thumbnailers (174) ([#213](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/213))

## 0.87.0 — 2026-10-22

- feat(toolbar): Shift + drag moves a button, a working dir per button (173) ([#212](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/212))

## 0.86.0 — 2026-10-21

- feat: Ctrl+F10 / F11 / F12 — which files a panel shows (172) ([#211](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/211))

## 0.85.2 — 2026-10-21

- fix(dialogs): arrows walk the buttons ([#210](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/210))
- docs(testing): how to make a cut tar and a 7z with encrypted names; a dir without r, not without x ([#209](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/209))

## 0.85.1 — 2026-10-21

- fix(menu): every key has a menu item ([#208](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/208))

## 0.85.0 — 2026-10-21

- feat: checksum files — create / verify (171) ([#207](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/207))

## 0.84.1 — 2026-10-20

- fix(i18n): errors in the UI's language (0.84.1) ([#206](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/206))

## 0.84.0 — 2026-10-20

- feat(tree): directory tree in the panel, Ctrl+F8 (170) ([#205](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/205))

## 0.83.0 — 2026-10-20

- feat(tree): folder tree Alt+F10 (169) ([#204](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/204))

## 0.82.0 — 2026-10-20

- feat(archive): test archives with Alt+Shift+F9 (168) ([#203](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/203))

## 0.81.0 — 2026-10-20

- feat(keys): TC keys Num /, Alt+Num ±, Shift+F10, Ctrl+← / →, Ctrl+L (167) ([#202](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/202))

## 0.80.2 — 2026-10-20

- fix: whole-code review (0.80.2) ([#201](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/201))

## 0.80.1 — 2026-10-20

- fix: what a headless pass over TESTING found (0.80.1) ([#200](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/200))

## 0.80.0 — 2026-10-20

- feat(net): remember a WebDAV password in the keyring (166) ([#199](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/199))

## 0.79.0 — 2026-10-20

- feat(cmdline): Ctrl+X next command, names not in UTF-8 (165) ([#198](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/198))

## 0.78.1 — 2026-10-20

- fix(net): «Browse network» found nothing (gio attribute separator) ([#197](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/197))

## 0.78.0 — 2026-10-20

- feat(net): browse inside the Windows network (164) ([#196](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/196))

## 0.77.0 — 2026-10-20

- feat(props): change the owner in Alt+Enter (163) ([#195](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/195))

## 0.76.0 — 2026-10-20

- feat(fonts): font search; columns scale with the text (162) ([#194](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/194))

## 0.75.0 — 2026-10-20

- feat(colors): ready palettes; path line, tabs, F-keys in the palette (161) ([#193](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/193))

## 0.74.1 — 2026-10-20

- fix: compare editor at the block's line; hotkeys buttons fit (0.74.1) ([#192](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/192))

## 0.74.0 — 2026-10-20

- feat(tabs): locked with dir changes allowed; Ctrl+Up keeps the sort (160) ([#191](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/191))

## 0.73.0 — 2026-10-20

- feat(diff): edit text in the compare window (159) ([#190](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/190))

## 0.72.0 — 2026-10-20

- feat(sync): sync with an archive (158) ([#189](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/189))

## 0.71.0 — 2026-10-20

- feat(find): found inside archives in «To panel» (157) ([#188](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/188))

## 0.70.0 — 2026-10-20

- perf(archive): fast zip writes (156) ([#187](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/187))

## 0.69.0 — 2026-10-19

- feat(archive): archives inside archives (155) ([#186](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/186))

## 0.68.0 — 2026-10-19

- feat(keys): your own hotkeys (154) ([#185](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/185))

## 0.67.1 — 2026-10-19

- build: man page and .rpm (153) ([#184](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/184))
- ci(release): .deb for Ubuntu 22.04 and newer (glibc 2.35) ([#183](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/183))

## 0.67.0 — 2026-10-19

- feat(cmdline): Shift+Enter from the panel (151) ([#182](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/182))

## 0.66.0 — 2026-10-19

- feat(net): remember the network password in the keyring (150) ([#181](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/181))

## 0.65.1 — 2026-10-19

- fix(terminal): the panel's cd goes only to known shells — tmux / screen / a hand-set program as the shell are never typed into ([#180](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/180))

## 0.65.0 — 2026-10-19

- feat(terminal): built-in terminal under the panels (0.65.0) ([#179](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/179))

## 0.64.0 — 2026-10-19

- feat(terminal): choose the external terminal, Open terminal (0.64.0) ([#178](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/178))

## 0.63.0 — 2026-10-19

- feat(marks): right-button drag marks rows, as TC (0.63.0) ([#177](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/177))

## 0.62.13 — 2026-10-19

- perf(sort): big lists sort in the background read (0.62.13) ([#176](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/176))

## 0.62.12 — 2026-10-19

- fix(keys): modifiers are let go when the window loses focus (0.62.12) ([#175](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/175))

## 0.62.11 — 2026-10-19

- fix: mask dialog opens with *.*, btrfs subvolumes get drive buttons (0.62.11) ([#174](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/174))

## 0.62.10 — 2026-10-19

- fix: a dead network mount no longer hangs Enter, F7 or the window (0.62.10) ([#173](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/173))

## 0.62.9 — 2026-10-19

- fix(search): ↑ in an empty list; roadmap: line index stays, clicks under progress (0.62.9) ([#172](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/172))

## 0.62.8 — 2026-10-19

- fix(lister): a picture over 32 MB opens as hex and says why (0.62.8) ([#171](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/171))
- test(sync): cover the sync job's delete step ([#170](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/170))

## 0.62.7 — 2026-10-19

- fix(diff): closing the window with unsaved block copies warns first (0.62.7) ([#169](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/169))

## 0.62.6 — 2026-10-19

- fix(queue): every job dropped from the queue says why (0.62.6) ([#168](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/168))

## 0.62.5 — 2026-10-19

- fix: a dead network mount no longer hangs the window; a gone dir goes up (0.62.5) ([#167](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/167))

## 0.62.4 — 2026-10-19

- fix(view): F3 / F4 give the real reason a file can't be opened (0.62.4) ([#166](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/166))

## 0.62.3 — 2026-10-19

- fix(config): a setting that could not be saved says so (0.62.3) ([#165](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/165))

## 0.62.2 — 2026-10-19

- fix(status): a pane's read error is a card too (0.62.2) ([#164](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/164))

## 0.62.1 — 2026-10-19

- fix(history): a failed Alt+← / Alt+→ step keeps the dir shown (0.62.1) ([#163](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/163))

## 0.62.0 — 2026-10-19

- feat(status): messages as a card of their own over the panels (0.62.0) ([#162](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/162))

## 0.61.13 — 2026-10-19

- fix(hotlist): a non-UTF-8 dir is refused with a reason (0.61.13) ([#161](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/161))

## 0.61.12 — 2026-10-19

- fix(hotlist): an outside change rebuilds the open Ctrl+D (0.61.12) ([#160](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/160))

## 0.61.11 — 2026-10-19

- fix(search): Alt+letter adds to an open quick search (0.61.11) ([#159](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/159))

## 0.61.10 — 2026-10-18

- fix(panel): a click in the other pane clears the error, as Tab (0.61.10) ([#158](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/158))

## 0.61.9 — 2026-10-18

- fix(panel): r-- dir entries show empty cells, not 0 / 1970 (0.61.9) ([#157](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/157))

## 0.61.8 — 2026-10-18

- fix(tabs): Ctrl+T opens the new tab without the filter (0.61.8) ([#156](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/156))

## 0.61.7 — 2026-10-18

- fix(viewer): N / P inside an archive, cancelled — cursor back on the file shown (0.61.7) ([#155](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/155))

## 0.61.6 — 2026-10-18

- fix(panel): Ctrl+R re-reads where the tab is going (0.61.6) ([#154](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/154))

## 0.61.5 — 2026-10-18

- fix(search): watcher events don't re-read search results (0.61.5) ([#153](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/153))

## 0.61.4 — 2026-10-18

- fix(archive): F4 — a failed put-back is asked about again (0.61.4) ([#152](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/152))

## 0.61.3 — 2026-10-18

- fix(archive): moving into an archive deletes only what went in (0.61.3) ([#151](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/151))
- docs(readme): screenshots of both looks, keys, a closer look ([#150](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/150))

## 0.61.2 — 2026-10-18

- fix(connect): «Browse network» can be stopped (0.61.2) ([#149](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/149))

## 0.61.1 — 2026-10-18

- fix(diff): count only the differences within reach (0.61.1) ([#148](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/148))

## 0.61.0 — 2026-10-18

- feat(diff): the current block is highlighted and follows the view (0.61.0) ([#147](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/147))

## 0.60.2 — 2026-10-18

- fix(archive): F5 of one file to a new name inside an archive (0.60.2) ([#146](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/146))

## 0.60.1 — 2026-10-18

- feat(dialogs): name on its own line with its icon in F5/F6/link/pack/unpack (0.60.1) ([#145](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/145))

## 0.60.0 — 2026-10-18

- feat(branch): Esc stops the second step too (117) ([#144](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/144))

## 0.59.1 — 2026-10-18

- fix(tabs): Ctrl+↑ on .. in search results ([#142](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/142))

## 0.59.0 — 2026-10-18

- feat(props): Alt+Enter inside an archive (116) ([#141](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/141))

## 0.58.0 — 2026-10-18

- feat(branch): Ctrl+B inside an archive (115) ([#140](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/140))

## 0.57.0 — 2026-10-18

- feat(find): Alt+L — results to panel (114) ([#139](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/139))

## 0.56.0 — 2026-10-18

- feat(diff): undo block copies (113) ([#138](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/138))
- chore(headless): vkey for Ctrl/Alt+letters ([#143](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/143))

## 0.55.0 — 2026-10-17

- feat(view): sizes in bytes or in KB / MB / GB ([#136](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/136))

## 0.54.1 — 2026-10-17

- perf(lister): ASCII search without an allocation per line ([#137](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/137))

## 0.54.0 — 2026-10-17

- feat(lister): files over 32 MB read window by window ([#134](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/134))

## 0.53.0 — 2026-10-16

- feat: viewer wraps at word boundaries ([#133](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/133))

## 0.52.0 — 2026-10-16

- feat: viewer code pages beyond Cyrillic (chardetng + encoding_rs) ([#132](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/132))

## 0.51.0 — 2026-10-16

- feat: a line between the panes and a frame around the active one ([#131](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/131))

## 0.50.0 — 2026-10-16

- feat: F3 in a window of its own (TC Lister) ([#130](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/130))

## 0.49.0 — 2026-10-16

- feat: F12 runs a command set in the settings ([#129](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/129))

## 0.48.0 — 2026-10-16

- docs(roadmap): the stale scroll frame after a tab switch cannot happen ([#127](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/127))
- feat: the right button marks files, held it opens the menu (setting) ([#128](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/128))

## 0.47.1 — 2026-10-16

- fix: status-line totals and Brief width cached, not counted every frame ([#126](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/126))

## 0.47.0 — 2026-10-16

- feat: Thumbnails view (Ctrl+Shift+F1) ([#125](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/125))

## 0.46.0 — 2026-10-16

- feat: drag the divider between the panes ([#124](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/124))

## 0.45.0 — 2026-10-16

- feat: list dialogs in the panels' font ([#123](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/123))

## 0.44.0 — 2026-10-16

- feat: exclude dirs in masks (sync, find) ([#122](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/122))

## 0.43.0 — 2026-10-16

- feat: sync dialog remembers its options ([#121](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/121))

## 0.42.0 — 2026-10-16

- feat: find text inside archives ([#120](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/120))

## 0.41.0 — 2026-10-16

- feat: change the group in Alt+Enter ([#119](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/119))

## 0.40.0 — 2026-10-16

- feat: hotlist — reorder by dragging ([#118](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/118))

## 0.39.0 — 2026-10-16

- feat: open password-protected zip / 7z ([#117](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/117))

## 0.38.0 — 2026-10-16

- feat: pack zip / 7z with a password ([#116](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/116))

## 0.37.0 — 2026-10-16

- feat: multi-rename token buttons and mask help ([#115](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/115))

## 0.36.0 — 2026-10-16

- feat: Ctrl+M (multi-rename) inside archives ([#114](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/114))

## 0.35.0 — 2026-10-16

- feat: F5 / F6 / Ctrl+V / drop from archive to archive ([#113](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/113))

## 0.34.0 — 2026-10-16

- feat: clipboard in archives (Ctrl+C / Ctrl+X / Ctrl+V) ([#112](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/112))

## 0.33.0 — 2026-10-16

- feat(dnd): drag and drop in archives (90) ([#111](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/111))

## 0.32.0 — 2026-10-16

- feat(find): attributes in Alt+F7 (89) ([#109](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/109))

## 0.31.0 — 2026-10-16

- feat(tabs): drag a tab to the other pane (88) ([#108](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/108))

## 0.30.0 — 2026-10-16

- feat(diff): word differences in a changed line (87) ([#107](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/107))

## 0.29.0 — 2026-10-16

- feat(lister): + / - zoom a picture (86) ([#106](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/106))

## 0.28.0 — 2026-10-16

- feat(lister): + / - change the text size (85) ([#105](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/105))

## 0.27.0 — 2026-10-16

- feat(props): setuid / setgid / sticky in Properties (84) ([#104](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/104))
- docs(roadmap): drag and drop in archives ([#110](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/110))

## 0.26.1 — 2026-10-15

- fix(lister): the list keeps its scroll after F3 → Esc ([#103](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/103))
- feat(colors): Background 2 — striped rows (83) ([#102](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/102))

## 0.26.0 — 2026-10-15

- docs(testing): § 82 checked by hand ([#101](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/101))

## 0.25.0 — 2026-10-15

- feat(dnd): arrows, Enter and hot letters in the drop menu (82) ([#100](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/100))
- docs(roadmap): archive encryption to do ([#99](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/99))

## 0.24.0 — 2026-10-15

- feat(dnd): the right-button drop menu at the pointer (81) ([#98](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/98))

## 0.23.0 — 2026-10-15

- feat(dnd): a card under the pointer while dragging (80) ([#97](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/97))
- docs(testing): § 78 checked by hand ([#96](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/96))

## 0.22.0 — 2026-10-15

- feat(branch): Esc stops a long Ctrl+B read (79) ([#95](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/95))

## 0.21.0 — 2026-10-15

- feat(fonts): file list font and size, viewer text size (78) ([#94](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/94))

## 0.20.0 — 2026-10-15

- feat(colors): panel text and background, «Blue» palette (77) ([#93](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/93))

## 0.19.0 — 2026-10-15

- feat(colors): attributes, marked and cursor colours (76) ([#92](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/92))
- docs(testing): § 74 checked by hand ([#90](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/90))

## 0.18.0 — 2026-10-14

- feat(about): check for updates (75) ([#91](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/91))

## 0.17.0 — 2026-10-14

- feat(colors): row background, size and age conditions ([#89](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/89))
- docs: Russian README, update by plain .deb download ([#88](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/88))

## 0.16.0 — 2026-10-14

- feat(view): colors by file type (v0.16.0) ([#87](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/87))

## 0.15.0 — 2026-10-14

- feat: symbolic links (Ctrl+Shift+F5), right-button drag with Copy / Move / Link (v0.15.0) ([#85](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/85))

## 0.14.0 — 2026-10-13

- feat(dnd): drop on tabs and drive buttons (v0.14.0) ([#83](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/83))

## 0.13.1 — 2026-10-13

- fix(startup): software rendering when the GPU lacks SHADER_F16_IN_F32 (v0.13.1) ([#84](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/84))

## 0.13.0 — 2026-10-12

- feat(ops): rename mask in F5 / F6, Shift+F5 for marked files, «Rename old», queue order (v0.13.0) ([#82](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/82))
- docs(testing): § 69 queue fixes checked by hand ([#81](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/81))

## 0.12.1 — 2026-10-12

- fix(ops): queue fixes — replan at start, ask before quitting, questions wait for the open dialog (v0.12.1) ([#80](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/80))

## 0.12.0 — 2026-10-07

- feat(ops): job queue, F2 «В очередь»; v0.12.0 ([#79](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/79))

## 0.11.0 — 2026-10-07

- feat(panel): drag & drop; v0.11.0 ([#78](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/78))

## 0.10.1 — 2026-10-06

- feat(about): donate button opens the Stripe page; v0.10.1 ([#77](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/77))

## 0.10.0 — 2026-10-06

- feat(panel): Ctrl+B branch view; v0.10.0 ([#76](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/76))
- Merge pull request #75 from shagovAlexei/docs/branch-view ([#75](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/75))

## 0.9.0 — 2026-10-06

- feat(lister): Ctrl+Q on a dir inside an archive; v0.9.0 ([#74](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/74))

## 0.8.0 — 2026-10-06

- feat(lister): Ctrl+Q inside an archive; v0.8.0 ([#73](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/73))

## 0.7.0 — 2026-10-06

- feat(lister): Ctrl+Q on a dir — files, folders, size; v0.7.0 ([#72](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/72))

## 0.6.0 — 2026-10-06

- feat(lister): Ctrl+Q quick view; v0.6.0 ([#71](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/71))

## 0.5.0 — 2026-10-06

- feat(hotlist): & hot letters; v0.5.0 ([#70](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/70))

## 0.4.0 — 2026-10-06

- feat(history): Clear, Remember history after restart; v0.4.0 ([#69](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/69))
- feat(dialogs): letter keys in list dialogs; v0.3.0 ([#68](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/68))
- docs: version bump rule ([#67](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/67))

## 0.2.0 — 2026-10-06

- chore: v0.2.0 ([#66](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/66))
- feat(open-with): All programs…, command history ([#65](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/65))
- feat(open-with): icons, Other program…, Make default ([#64](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/64))
- X11 menu: scrolls, no code change; wheel in vpointer ([#63](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/63))
- docs: X11 check ([#62](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/62))
- feat(context-menu): Open with… ([#61](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/61))
- docs(roadmap): Open with in the context menu ([#60](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/60))
- feat(hotlist): submenus (55) ([#59](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/59))

## 0.1.0 — 2026-10-06

- feat(toolbar): icon picker; toolbar docs (53) ([#58](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/58))
- feat(panel): sort by counted dir size, stop counts on leaving (54) ([#57](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/57))
- feat(toolbar): configurable button bar (53) ([#56](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/56))
- feat(tabs): move tabs left / right (52) ([#55](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/55))
- Phase 51: double click selects a word in the viewer ([#54](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/54))
- Phase 50: Shift+F4 — new file ([#53](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/53))
- Phase 49: select and copy text in the viewer ([#52](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/52))
- Phase 48: TC-like column widths ([#51](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/51))
- Phase 47: clickable path in the classic skin ([#50](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/50))
- Phase 46: TC-like "file exists" dialog ([#49](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/49))
- Phase 45: TC-like compare-files window ([#48](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/48))
- Phase 44: TC-like synchronize window ([#47](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/47))
- fix: findings of manual check 2 (phase 43) ([#46](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/46))
- Phase 42: background operations ([#45](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/45))
- fix: findings of the manual check (phase 41) ([#44](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/44))
- feat: TC-style directory hotlist (phase 40) ([#42](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/42))
- feat: Alt+Shift+Enter counts all folder sizes (phase 39) ([#41](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/41))
- feat: Space on a folder counts its size (phase 38) ([#40](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/40))
- Phase 37: right-click context menus (rows, dir, tabs) ([#39](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/39))
- Phase 36: two UI skins — classic (TC) and modern (COSMIC) ([#38](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/38))
- Phase 35: tech debt — quoted masks, results by file name, lister encoding guess, status gap ([#37](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/37))
- Phase 34: properties (Alt+Enter) with permission bits ([#36](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/36))
- Phase 33: find — name regex, inside archives, remembered filters ([#35](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/35))
- Phase 32: Lister — wrap (W), encodings 1251 / KOI8-R / 866 ([#34](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/34))
- Phase 31: .deb package and release on tags ([#33](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/33))
- Phase 30: Shift+F5, rename on conflict, dir dates, ~ in targets ([#32](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/32))
- Phase 29: Brief view (Ctrl+F1/F2) and type icons ([#31](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/31))
- Phase 28: TC tabs (Ctrl+Up, copy/move, lock, rename) ([#30](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/30))
- Phase 27: command line (path>) ([#29](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/29))
- Phase 26: network — saved connections, browse, cancel ([#28](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/28))
- Phase 25: compare files — ignore spaces/case, inline diff, copy blocks ([#27](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/27))
- Phase 24: one status line above the F-keys ([#26](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/26))
- Phase 23: sync — mirror, delete rows, file mask ([#25](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/25))
- Phase 22: find — to panel, size/date filters, regex ([#24](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/24))
- Phase 21: writing into archives ([#23](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/23))
- Phase 20: built-in F3 viewer (Lister) ([#22](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/22))
- Phase 19: window status bar ([#21](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/21))
- Phase 18: help (F1), about, settings (Ctrl+,) ([#20](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/20))
- Phase 17: main menu in the header bar ([#19](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/19))
- Phase 16: mount sticks and network shares (Alt+F1, Ctrl+F, Ctrl+Shift+F) ([#18](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/18))
- Phase 15: compare files by content (Ctrl+Shift+D) ([#17](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/17))
- Phase 14: compare and synchronize dirs (Shift+F2, Ctrl+Shift+S) ([#16](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/16))
- Phase 13: find files (Alt+F7) ([#15](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/15))
- Phase 12: browse archives as folders (read-only) ([#14](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/14))
- Phase 11: pack and unpack archives (Alt+F5 / Alt+F9) ([#13](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/13))
- Multi-rename Ctrl+M (phase 10) ([#12](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/12))
- Clipboard Ctrl+C/X/V, compatible with cosmic-files ([#11](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/11))
- History (Alt+←/→/↓), hotlist (Ctrl+D), swap panes (Ctrl+U) ([#10](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/10))
- Quick search (Alt+letters) and panel filter (Ctrl+S) ([#9](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/9))
- Tech debt: 13 fixes after MVP ([#8](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/8))
- Phase 6: F3/F4, drives, watcher, config — MVP done ([#7](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/7))
- File operations (phase 5) ([#6](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/6))
- TC selection (phase 4) ([#5](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/5))
- Tabs (phase 3) ([#4](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/4))
- Two TC panels (phase 2) ([#3](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/3))
- core: listing, sort, Panel (milestone 1) ([#2](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/2))
- Bootstrap: workspace, empty window, CI, architecture docs ([#1](https://github.com/shagovAlexei/cosmic-shagoff-commander/pull/1))
- Initial commit: README, GPL-3.0 license

