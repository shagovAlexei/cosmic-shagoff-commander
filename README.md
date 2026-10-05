# Shagoff Commander

A dual-pane file manager in the spirit of Total Commander, built with libcosmic for the COSMIC desktop (Pop!_OS 24.04).

**Status:** early development. See [.claude/docs/ROADMAP.md](.claude/docs/ROADMAP.md).

## Features

- Two panels with tabs (lock, rename, move between panels), Total Commander keys: F3–F8, Shift+F5/F6, Insert / Space / Num+ / Num− marking, Alt+letter quick search, history and hotlist
- Copy / move / delete with conflict answers (replace, skip, replace older, rename, keep both), trash or permanent delete, clipboard compatible with COSMIC Files
- Archives as folders: zip, tar.gz / bz2 / xz / zst, 7z — browse, copy in and out, pack / unpack
- Built-in viewer (F3): text with wrap and Cyrillic code pages (1251, KOI8-R, 866, guessed), hex, images
- Find files (masks or regex, text inside, size / date, inside archives), compare and synchronize folders, compare files side by side
- Properties with permission bits, multi-rename, command line `path>`, network mounts through gvfs
- Right-click menus, full / brief view, file type icons, two looks: Classic (Total Commander) and Modern (COSMIC)

## Toolbar buttons

Right click on the toolbar (Classic look) or *Configuration → Toolbar…* opens the button editor: add, remove, move, pick an icon. A button runs an internal command (`cm_*`, picked from the list) or a program with parameters:

| | |
|---|---|
| `%P` / `%T` | the active / other panel's folder |
| `%N` | the name under the cursor; `%O` without its extension, `%E` the extension |
| `%S` | the selected names |

Each value is quoted as one shell word. The command runs through `sh -c` in the panel's folder, without a terminal. Example, a terminal that lists the file under the cursor and waits for Enter:

- Command: `cosmic-term -e`
- Parameters: `sh -c "ls -l %N; read x"`

Use double quotes around the script (the inserted names carry their own single quotes), and `read x`, not a bare `read`: `sh` is dash, where `read` without a variable fails at once and the terminal closes. A failing command shows its error in the status line.

## Install

A `.deb` for Pop!_OS 24.04 / Ubuntu 24.04 is attached to each [release](https://github.com/shagovAlexei/cosmic-shagoff-commander/releases):

```sh
sudo apt install ./shagoff-commander_*_amd64.deb
```

## Build

```sh
just build-release
sudo just install   # or: just deb → target/deb/shagoff-commander_<version>_<arch>.deb
```

## License

GPL-3.0-only. Some parts are adapted from [cosmic-files](https://github.com/pop-os/cosmic-files) (GPL-3.0-only).
