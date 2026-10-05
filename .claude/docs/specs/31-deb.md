# 31. Пакет .deb

Статус: решения приняты сами (пользователь: «давай следующее»). Следующий пункт бэклога «Упаковка: deb, flatpak»; flatpak отложен.

## Решения

| Вопрос | Решение |
|---|---|
| Чем собирать | `dpkg-deb` из рецепта `just deb`, без `cargo-deb` (не нужен новый инструмент). Файлы ставит тот же `just install` с `rootdir=target/deb/root` |
| Зависимости | Библиотеки — `dpkg-shlibdeps` по бинарнику (как у `cosmic-files`: libc6, libgcc-s1, libxkbcommon0); плюс `libglib2.0-bin` (`gio` для монтирования), `xdg-utils` (открытие файлов). Recommends: `cosmic-edit` (F4 по умолчанию), `gvfs-backends`, `gvfs-fuse` (сеть) |
| Политика Debian | Бинарник без символов; `copyright` в машинном формате со ссылкой на `/usr/share/common-licenses/GPL-3`; `changelog.gz`; права 0755/0644. lintian без ошибок (остаются предупреждения: категория `COSMIC` в .desktop — как у приложений COSMIC; нет man-страницы) |
| Релиз | Workflow `release.yml`: тег `v*` → сборка на ubuntu-24.04 (glibc как в Pop!_OS 24.04) → `gh release create` с .deb |
| Flatpak | Позже: нужен `cargo-sources.json`, который надо перегенерировать на каждое изменение `Cargo.lock` |

## Проверка

`just deb` локально, `lintian` по пакету, `dpkg-deb -c` — состав. Установка (`sudo apt install`) — вручную пользователем.
