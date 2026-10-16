# 99. Find: text inside archives — plan

Spec: `../specs/99-find-text-in-archives.md`.

1. Core: `archive::for_each_file`; `search::{text_in, regex_in}` over `Read`, `Query::in_archive` reads members when the query has a text (test: zip text and regex, tar.gz, off without «archives»).
2. i18n: the checkbox label.
