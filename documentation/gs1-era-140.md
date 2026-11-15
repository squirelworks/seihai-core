# Explicit GS1 1.40 vocabulary

Call `scripting::engine::parse(source, Era::V140)` to select the 125-word
registration table of 1.40 / 1.40 rev 1. The existing GS1 compiler example
also selects it with `--era 1.40`, through `Era::ALL` and `Era::label`.

The published [per-build matrix](https://github.com/squirelworks/Graal_analysis/blob/0dc661a8f0b1fb7bd613ce8045358cfb14345553/npc-library/gs1-commands-by-version.tsv)
has 111 words for 1.39, 125 for 1.40 and 126 for 1.41 rev 1. Relative to
1.39, 1.40 adds `function`, `return`, `break`, `continue`, `callnpc`,
`callweapon`, `say2`, `canwarp`, `changeimgvis`, `drawovertrees`,
`putexplosion2`, `setimg`, `setimgpart` and `spyfire`, with no removals.
Only `showstats` is added in 1.41 rev 1. The
[build study](https://github.com/squirelworks/Graal_analysis/blob/0dc661a8f0b1fb7bd613ce8045358cfb14345553/client-internals/graal2002-and-140-builds.md#what-the-140-builds-settle)
explains the static evidence.

These are registration dates. Argument signatures and executors are the
existing Seihai implementations; this does not establish native 1.40
argument or effect behavior. No original client was run. In particular,
the existing measured timeout gate still starts at 1.41; selecting 1.40
keeps Seihai's earlier fallback rather than assuming 1.41 timing.

`Era::for_version` and all online profiles/tokens keep their existing
mapping. No `ClientVersion::V140` or network profile is introduced. Source
vocabulary selection does not establish a target client's bytecode or
packet support.

Adding `Era::V140` to the public exhaustive enum requires downstream
exhaustive matches to handle that variant. It does not change the era
selected by an existing `ClientVersion`.
