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
- `objects.gs2`: a public function, `format`, an array literal, `for`-each,
  `@`/`SPC`, string methods, `new[3][2]`, `new TStaticVar()`, `with`, and
  some maths built-ins.

The layout and opcodes are in `documentation/gs2-opcodes.md`;
`tests/gs2_opcodes.rs` checks these files against it.
