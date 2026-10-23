# План 194: дисковый кэш эскизов

1. core `thumbcache` (uri, cache_file, read, write) + тесты; `png` в core.
2. `thumbs::make_in` читает / пишет кэш, `forget` для «Перечитать»; тест в `thumbs`.
3. tc-reference не меняется (не TC); ROADMAP, TESTING, 0.108.0; verify, `/code-review`, PR.
