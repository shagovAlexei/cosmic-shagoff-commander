# План 185: разрезать / собрать файл

1. core `split`: `parse_size`, `part_name`, `split`, `parts_of`, `combine` (`crc32fast`) + тесты `sizes_are_read_as_typed`, `a_file_is_cut_and_joined_back_checked_by_its_crc`.
2. app: `Action::SplitFile` / `CombineFiles` (меню «Файлы», справка, `cm_Split` / `cm_Combine`), `Dialog::Split`, `InputOp::Combine`, `Message::SplitSize` / `Pieces`, `pieces` в `util`; `read_only`, `on_files`; тест `split_offers_the_other_panel_and_combine_only_takes_parts`.
3. tc-reference, ROADMAP, TESTING, 0.99.0; verify, `/code-review`, PR.
