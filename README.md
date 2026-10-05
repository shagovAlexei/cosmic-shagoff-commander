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
