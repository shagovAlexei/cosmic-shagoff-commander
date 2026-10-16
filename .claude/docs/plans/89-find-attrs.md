# 89. Find attributes — plan

Spec: `../specs/89-find-attrs.md`.

1. Tests first: `attrs("x")`, a colour rule `x` (file yes, dir no); search with `x`, `r`, `h`, none.
2. `Attrs.exec`, `fits_mode`; `Query.attrs`, `has_attrs`, no archive members with attributes.
3. Dialog field, `FindField::Attrs`, `FindPrefs.attrs`, `find_query` parse error, `h` → hidden.
4. App test: a bad letter is refused. Colour help and hint mention `x`.
5. headless: Alt+F7 at 1036x530, `x` finds the program.
