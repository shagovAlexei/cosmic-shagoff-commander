# Shagoff Commander

A dual-pane file manager in the spirit of Total Commander, built with libcosmic for the COSMIC desktop (Pop!_OS 24.04).

**Status:** early development. See [.claude/docs/ROADMAP.md](.claude/docs/ROADMAP.md).

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
