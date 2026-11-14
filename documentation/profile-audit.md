# Profile audit

Roadmap #7, "Next 2". For every profile row and every field the chain ever
changes, where the row's value comes from: the step that set it, how that
step is backed, and what the row's own step says it does not know. The full
table is [profile-audit.tsv](profile-audit.tsv), one line per row and field
(52 fields × 49 rows). Code: `crates/seihai-core/src/protocol/audit.rs`;
regenerate with

```sh
cargo run -p seihai-core --example profile_audit -- tsv > documentation/profile-audit.tsv
cargo run -p seihai-core --example profile_audit -- summary
```

A unit test fails when either copy is stale.

## How to read it

The chain stores each build as a delta against the build before it, so most
of a row's values were set by an older build. That is a Seihai assumption
unless the row's own build was checked, so each value gets one of five
labels:

| Evidence | Meaning |
|---|---|
| read here | Set by this row's own step, which has a binary. Native evidence. |
| inferred here | Set by this row's own step, which has no binary (release notes, server source, neighbours). |
| carried | Set by an earlier step. This row has a binary and its step notes nothing about the field: either it was read and matched, or it was not looked at. The step's `source` line says which reads were made. |
| carried, flagged unknown | Set by an earlier step, and this row's own step says the field was not read or is not known. A Seihai fallback, kept explicit. |
| carried, no binary | Set by an earlier step, and this row has no binary at all. |

A shared login token or archive label is never evidence on its own: 5.30r3,
5.31r4, 6.00r2 and 6.00r4 share `G3D26090`, yet each row's values trace to
its own reads.

## Summary

| Row | Basis | Read here | Inferred here | Carried | Carried, flagged unknown | Carried, no binary |
|---|---|---|---|---|---|---|
| 1.21 | binary | 52 | 0 | 0 | 0 | 0 |
| 1.28 beta 2 | binary | 8 | 0 | 44 | 0 | 0 |
| 1.3 test | binary | 5 | 0 | 47 | 0 | 0 |
| 1.31 | binary | 2 | 0 | 50 | 0 | 0 |
| 1.32 | binary | 2 | 0 | 50 | 0 | 0 |
| 1.33 | binary | 2 | 0 | 50 | 0 | 0 |
| 1.34 | binary | 3 | 0 | 48 | 1 | 0 |
| 1.35 | binary | 2 | 0 | 50 | 0 | 0 |
| 1.36 | binary | 1 | 0 | 51 | 0 | 0 |
| 1.37 | binary | 1 | 0 | 51 | 0 | 0 |
| 1.38 | binary | 7 | 0 | 45 | 0 | 0 |
| 1.39 | binary | 8 | 0 | 43 | 1 | 0 |
| 1.41 | binary | 12 | 0 | 36 | 4 | 0 |
| 1.42 | binary | 11 | 0 | 41 | 0 | 0 |
| 2.00 | binary | 4 | 0 | 44 | 4 | 0 |
| 2.01 | binary | 7 | 0 | 45 | 0 | 0 |
| 2.02 | binary | 5 | 0 | 47 | 0 | 0 |
| 2.02 rev2 | binary | 2 | 0 | 50 | 0 | 0 |
| 2.03 | binary | 4 | 0 | 48 | 0 | 0 |
| 2.04 | binary | 4 | 0 | 48 | 0 | 0 |
| 2.05 | binary | 8 | 0 | 44 | 0 | 0 |
| 2.1 | inferred | 0 | 15 | 0 | 0 | 37 |
| 2.12 | inferred | 0 | 0 | 0 | 0 | 52 |
| 2.13 | inferred | 0 | 0 | 0 | 0 | 52 |
| 2.14 | inferred | 0 | 4 | 0 | 0 | 48 |
| 2.15 | inferred | 0 | 0 | 0 | 0 | 52 |
| 2.16 | inferred | 0 | 2 | 0 | 0 | 50 |
| 2.17 | binary | 13 | 0 | 34 | 5 | 0 |
| 2.22 | binary | 11 | 0 | 38 | 3 | 0 |
| 2.3 | inferred | 0 | 14 | 0 | 0 | 38 |
| 2.31 | binary | 6 | 0 | 45 | 1 | 0 |
| 3.04 | binary | 13 | 0 | 36 | 3 | 0 |
| 3.153 | binary | 9 | 0 | 42 | 1 | 0 |
| 4.04r2 | binary | 9 | 0 | 40 | 3 | 0 |
| 4.04r2 Mac | binary | 2 | 0 | 48 | 2 | 0 |
| 4.20r6 | binary | 3 | 0 | 48 | 1 | 0 |
| 4.20r8 | binary | 2 | 0 | 0 | 50 | 0 |
| 5.00r7 | binary | 5 | 0 | 40 | 7 | 0 |
| 5.12 | binary | 3 | 0 | 47 | 2 | 0 |
| 5.12 Mac | binary | 3 | 0 | 47 | 2 | 0 |
| 5.24r4 | binary | 3 | 0 | 43 | 6 | 0 |
| 5.30r3 | binary | 1 | 0 | 50 | 1 | 0 |
| 5.31r4 | binary | 1 | 0 | 48 | 3 | 0 |
| 6.00r2 | binary | 5 | 0 | 44 | 3 | 0 |
| 6.00r4 | binary | 1 | 0 | 47 | 4 | 0 |
| 6.015 | inferred | 0 | 3 | 0 | 0 | 49 |
| 6.034 | inferred | 0 | 2 | 0 | 0 | 50 |
| 6.03r7 | binary | 2 | 0 | 50 | 0 | 0 |
| 6.03r7 Linux | binary | 1 | 0 | 45 | 6 | 0 |

## What the audit shows

- **Rows with no binary:** 2.1, 2.12 to 2.16, 2.3, 6.015 and 6.034. Every
  value is either inferred in that step or carried; nothing is native.
- **4.20r8:** only the modified engine DLL could be read (the stock build is
  in a password-protected archive), so its step flags the whole row.
- **Send ids (`pli`)** are the most common fallback: 5.12, 5.24r4, 5.30r3,
  5.31r4, 6.00r2 and 6.00r4 keep an older build's set and say so.
- **Fields never changed in the chain** (one value from `ORIGIN` in every
  row) are not listed; they are the same for every build by construction.
