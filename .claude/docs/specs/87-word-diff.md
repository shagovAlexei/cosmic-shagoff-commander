# 87. Word differences in the file comparison

ROADMAP «Сравнение файлов, дальше: пословное отличие в строке».

TC's «Compare by content» marks the differing words inside a changed line; we marked one span from
the first difference to the last.

- `diff::words(a, b)`: the lines cut into tokens (a run of letters / digits / `_`, a run of spaces,
  one other char), their longest common sequence (DP, O(n·m); over 250 000 cells the whole line
  is one mark), the tokens outside it marked; marks with only spaces between are one; spaces at a
  mark's edges are not marked. Byte ranges for each side, on char boundaries.
- The comparison dialog tints every mark of a changed row (`half` takes the list). Replaces
  `diff::inline`.
