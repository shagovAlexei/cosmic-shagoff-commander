name := 'shagoff-commander'
appid := 'io.github.shagovAlexei.cosmic-shagoff-commander'
prefix := '/usr'

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
    install -Dm0755 target/release/{{name}} {{prefix}}/bin/{{name}}
    install -Dm0644 res/{{appid}}.desktop {{prefix}}/share/applications/{{appid}}.desktop
    install -Dm0644 res/{{appid}}.metainfo.xml {{prefix}}/share/metainfo/{{appid}}.metainfo.xml
    install -Dm0644 res/icons/{{appid}}.svg {{prefix}}/share/icons/hicolor/scalable/apps/{{appid}}.svg

uninstall:
    rm -f {{prefix}}/bin/{{name}} {{prefix}}/share/applications/{{appid}}.desktop {{prefix}}/share/metainfo/{{appid}}.metainfo.xml {{prefix}}/share/icons/hicolor/scalable/apps/{{appid}}.svg
