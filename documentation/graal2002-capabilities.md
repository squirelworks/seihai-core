# Graal 2002 build capabilities

`protocol::graal2002::Graal2002Build` identifies the six Windows world
builds explicitly. `descriptor()` provides their labels, FileVersions,
statically read tokens and GS1 registration-table counts.

Use `gs1_word_registered("replacestring")` on the known build to query
word membership. It includes script keywords and aliases. For example,
ver7 and ver9 share `GNW03081` and FileVersion `2.1.0.0`, but only ver9
registers `replacestring`. Ver13 and v22 share `GNW31101` and FileVersion
`2.1.0.4`, but only v22 registers `setshootparams` and the cursor/wrap
commands. Neither token nor FileVersion alone can select these capabilities.

The names come from the static study
[Graal 2002 world builds](https://github.com/squirelworks/Graal_analysis/blob/main/client-internals/graal2002-and-140-builds.md).
An independent rerun of that repository's committed GS1 table scanner on
all six builds and the actual 2.05 baseline confirmed the counts
161 → 167/167/168/169/169/174. After normalizing trailing-space markers,
none removes a baseline name. The module reuses the existing `V205` name
table and adds only the thirteen names established by this comparison.
Source fixtures and original executables are not included in this repo.

This API records registrations, not native argument semantics or execution
effects. It does not select the GS1 parser's `Era`, add an online
`ClientVersion` profile, or infer packet handlers, layouts or send support.
The generic inferred `V21`/`V212`/`V215`/`V216` eras stay unchanged. A caller
must retain the exact build identity separately instead of deriving it from
the shared token. No native runtime or wire acceptance is claimed.
