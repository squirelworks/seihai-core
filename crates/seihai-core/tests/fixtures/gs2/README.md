# GS2 fixtures

Each `.gs2` is a short script written for these tests. The `.gs2bc` next to
it is that script compiled by the open-source
[gs2-parser](https://github.com/xtjoeytx/gs2-parser) (`gs2test file.gs2`),
built from the 2026-10-31 source tarball. That build reproduces the
repository's own baseline for `tests/scripts/expressions/01_arithmetic.gs2`
byte for byte. No byte here was written by hand.

- `shared.gs2`: commands the classic client also has (`set`, `showimg`,
  `hideimg`, `sleep`) and GS1-style arithmetic.
- `control.gs2`: `for`, `while`, `switch`, `continue`/`break`, `&&`/`||`,
  `?:`, `in |a, b|`, and numbers that need `F3`, `F4` and `F5`.
- `calls.gs2`: a function with parameters and a return value, `@`, `say2`,
  `setlevel2`, `unset`, and a `while (true)` for the loop limit.
- `timers.gs2`: `setTimer`, `this.timeout` read and written, `scheduleevent`
  with and without `on`, `cancelevents`, and a timer that runs out during a
  `sleep`.
- `strings.gs2`: `format`, `char`, `_()`, and the `trim`, `length`, `pos`,
  `charat`, `substring`, `starts` and `ends` methods.
- `lists.gs2`: a list literal, `a[i]` read and written, the list methods,
  `tokenize`, `positions`, `in`, `for`-each, and `new[n][m]` with `a[i, j]`.
- `rest.gs2`: `exp`, `log`, `getangle`, `getdir`, `vecx`, `vecy`, the
  shifts, `in |a, b|`, `type`, `indices`, `setarray`, and `with` on the
  player and on null.
- `objects.gs2`: a public function, `format`, an array literal, `for`-each,
  `@`/`SPC`, string methods, `new[3][2]`, `new TStaticVar()`, `with`, and
  some maths built-ins.

The layout and opcodes are in `documentation/gs2-opcodes.md`;
`tests/gs2_opcodes.rs` checks these files against it, and
`tests/gs2_exec.rs` runs them (`documentation/gs2-executor.md`).
