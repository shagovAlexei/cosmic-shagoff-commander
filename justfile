name := 'shagoff-commander'
appid := 'io.github.shagovAlexei.cosmic-shagoff-commander'
prefix := '/usr'
# Staging root for packaging: `just rootdir=pkg install` puts files under pkg/usr/…
rootdir := ''
dest := rootdir + prefix

[private]
default:
    @just --list

# fmt check + clippy (deny warnings) + tests — run before every PR
verify:
    cargo fmt --all --check
    cargo clippy --all-targets -- -D warnings
    cargo test

build-debug *args:
    cargo build -p {{name}} {{args}}

build-release *args: (build-debug '--release' args)

# Run with debug logs
run-logs *args:
    env RUST_LOG=debug RUST_BACKTRACE=full cargo run -p {{name}} {{args}}

install:
    install -Dm0755 target/release/{{name}} {{dest}}/bin/{{name}}
    install -Dm0644 res/{{appid}}.desktop {{dest}}/share/applications/{{appid}}.desktop
    install -Dm0644 res/{{appid}}.metainfo.xml {{dest}}/share/metainfo/{{appid}}.metainfo.xml
    install -Dm0644 res/icons/{{appid}}.svg {{dest}}/share/icons/hicolor/scalable/apps/{{appid}}.svg

uninstall:
    rm -f {{dest}}/bin/{{name}} {{dest}}/share/applications/{{appid}}.desktop {{dest}}/share/metainfo/{{appid}}.metainfo.xml {{dest}}/share/icons/hicolor/scalable/apps/{{appid}}.svg

# .deb for Debian / Ubuntu / Pop!_OS into target/deb/ (dpkg-deb; library deps from dpkg-shlibdeps)
deb: (build-release '--locked')
    #!/usr/bin/env bash
    set -euo pipefail
    umask 022
    version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
    arch=$(dpkg --print-architecture)
    root=target/deb/root
    rm -rf target/deb && mkdir -p "$root/DEBIAN"
    just rootdir="$root" install
    strip "$root/usr/bin/{{name}}"
    chmod 0755 "$root"
    doc="$root/usr/share/doc/{{name}}"
    mkdir -p "$doc"
    # Debian policy: the license by reference to common-licenses, and a changelog.
    printf '%s\n' \
        'Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/' \
        'Upstream-Name: {{name}}' \
        'Source: https://github.com/shagovAlexei/cosmic-shagoff-commander' \
        '' 'Files: *' 'Copyright: 2026 Shagov Alexei <shagov.alexei@gmail.com>' 'License: GPL-3.0-only' \
        ' On Debian systems the full text is in /usr/share/common-licenses/GPL-3.' > "$doc/copyright"
    printf '%s (%s) unstable; urgency=medium\n\n  * Release %s, see the GitHub releases.\n\n -- Shagov Alexei <shagov.alexei@gmail.com>  %s\n' \
        '{{name}}' "$version" "$version" "$(date -R)" | gzip -9n > "$doc/changelog.gz"
    # dpkg-shlibdeps wants a debian/control next to it; a throwaway one is enough.
    work=$(mktemp -d) && trap 'rm -rf "$work"' EXIT
    mkdir "$work/debian" && touch "$work/debian/control"
    libs=$(cd "$work" && dpkg-shlibdeps -O "$OLDPWD/$root/usr/bin/{{name}}" | sed 's/^shlibs:Depends=//')
    cat > "$root/DEBIAN/control" <<EOF
    Package: {{name}}
    Version: $version
    Architecture: $arch
    Maintainer: Shagov Alexei <shagov.alexei@gmail.com>
    Installed-Size: $(du -sk "$root/usr" | cut -f1)
    Depends: $libs, libglib2.0-bin, xdg-utils
    Recommends: cosmic-edit, gvfs-backends, gvfs-fuse
    Section: utils
    Priority: optional
    Homepage: https://github.com/shagovAlexei/cosmic-shagoff-commander
    Description: Total Commander-style dual-pane file manager for COSMIC
     Two panels, tabs, F-key operations, archives as folders, file search,
     directory sync and compare, network mounts; keys as in Total Commander.
    EOF
    sed -i 's/^    //' "$root/DEBIAN/control"
    dpkg-deb --root-owner-group --build "$root" "target/deb/{{name}}_${version}_${arch}.deb"
