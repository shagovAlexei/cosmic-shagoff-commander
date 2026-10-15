# 87. Word diff — plan

Spec: `../specs/87-word-diff.md`.

1. Test first: `words` — one word, two places, Cyrillic bytes, equal lines, neighbours merged,
   an insertion only on the right.
2. `tokens`, `words` (LCS table, backtrack, merge, trim); drop `inline`.
3. Dialog: `half(.., &[Range])` pushes plain / tinted segments.
4. headless: F11 on two versions of a file.
