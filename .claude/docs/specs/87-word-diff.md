# 87. Word differences in the file comparison

ROADMAP «Сравнение файлов, дальше: пословное отличие в строке».

TC's «Compare by content» marks the differing words inside a changed line; we marked one span from
the first difference to the last.

- `diff::words(a, b, opts)`: the lines cut into tokens (a run of letters / digits / `_`, a run of
  spaces, one other char), compared as the lines are (`Opts::key`: ignore case / spaces); the common
  head and tail dropped, the longest common sequence of the middle (DP, O(n·m); over 250 000 cells
  the middle is one mark), the tokens outside it marked. Marks with no letter or digit between are
  one (a rewritten `x.foo(a)` is one mark); spaces at a mark's edges are not marked, unless spaces
  are all that differs. Byte ranges for each side, on char boundaries.
- The comparison dialog tints every mark of a changed row (`half` takes the list). Replaces
  `diff::inline`.
