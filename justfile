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
    install -Dm0644 res/apparmor/usr.bin.{{name}} {{dest}}/share/doc/{{name}}/apparmor/usr.bin.{{name}}
    install -d {{dest}}/share/man/man1
    gzip -9nc res/{{name}}.1 > {{dest}}/share/man/man1/{{name}}.1.gz
    chmod 0644 {{dest}}/share/man/man1/{{name}}.1.gz

uninstall:
    rm -f {{dest}}/bin/{{name}} {{dest}}/share/applications/{{appid}}.desktop {{dest}}/share/metainfo/{{appid}}.metainfo.xml {{dest}}/share/icons/hicolor/scalable/apps/{{appid}}.svg {{dest}}/share/man/man1/{{name}}.1.gz

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
    Depends: $libs, libglib2.0-bin, xdg-utils, curl
    Recommends: cosmic-edit, gvfs-backends, gvfs-fuse
    Suggests: rclone
    Section: utils
    Priority: optional
    Homepage: https://github.com/shagovAlexei/cosmic-shagoff-commander
    Description: Total Commander-style dual-pane file manager for COSMIC
     Two panels, tabs, F-key operations, archives as folders, file search,
     directory sync and compare, network mounts; keys as in Total Commander.
    EOF
    sed -i 's/^    //' "$root/DEBIAN/control"
    dpkg-deb --root-owner-group --build "$root" "target/deb/{{name}}_${version}_${arch}.deb"

# .rpm for Fedora / openSUSE into target/generate-rpm/ (cargo-generate-rpm; library deps found by ldd)
rpm: (build-release '--locked')
    mkdir -p target/man && gzip -9nc res/{{name}}.1 > target/man/{{name}}.1.gz
    cargo generate-rpm -p crates/app --auto-req builtin

# Sign a draft release made by the `release` workflow and publish it (214a); the author runs it
# after each tag (the key asks for its password). The key never leaves this machine; the app
# installs nothing whose SHA256SUMS.minisig does not check (core::update::PUBLIC_KEY). Until the
# releases 0.130.0 checks are gone, also the old openssl signature (SHA256SUMS.sig) it wants.
# Sign release TAG and publish it
sign-release tag key='~/.config/minisign/shagoff.key':
    #!/usr/bin/env bash
    set -euo pipefail
    repo=shagovAlexei/cosmic-shagoff-commander
    test "$(git rev-parse '{{tag}}^{commit}')" = "$(gh api "repos/$repo/commits/{{tag}}" --jq .sha)"
    d=$(mktemp -d); trap 'rm -rf "$d"' EXIT
    gh release download '{{tag}}' -R "$repo" -p SHA256SUMS -p '*.deb' -p '*.rpm' -D "$d"
    (cd "$d" && sha256sum -c --strict SHA256SUMS)
    # Built by our workflow (gh ≥ 2.49 has `attestation`).
    if gh attestation --help >/dev/null 2>&1; then
        for f in "$d"/*.deb "$d"/*.rpm; do gh attestation verify "$f" -R "$repo" >/dev/null; done
    else
        echo "gh has no 'attestation': provenance not checked" >&2
    fi
    echo "Signing for {{tag}}:"; cat "$d/SHA256SUMS"
    minisign -S -s {{key}} -m "$d/SHA256SUMS" -t "shagoff-commander {{tag}}"
    minisign -Vq -p res/minisign.pub -m "$d/SHA256SUMS"
    files=("$d/SHA256SUMS.minisig")
    old=$HOME/.config/shagoff-release/signing.pem
    if [ -f "$old" ]; then
        openssl pkeyutl -sign -rawin -inkey "$old" -in "$d/SHA256SUMS" -out "$d/SHA256SUMS.sig"
        files+=("$d/SHA256SUMS.sig")
    fi
    gh release upload '{{tag}}' -R "$repo" "${files[@]}" --clobber
    gh release edit '{{tag}}' -R "$repo" --draft=false --latest
    echo "{{tag}} is out."
