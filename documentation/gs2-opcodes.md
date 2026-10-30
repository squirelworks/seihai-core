# GS2 bytecode: file layout and opcodes

Date: 2026-10-30. Source: [gs2-parser](https://github.com/xtjoeytx/gs2-parser)
(GPLv3), the same tree as `gs2-grammar.md`. Read from `src/opcodes.h` (the
enum), `src/codegen/GS2Bytecode.cpp` (the file and operand writer),
`src/compiler/GS2CompilerVisitor.cpp` (what each construct emits) and
`src/compiler/GS2BuiltInFunctions.cpp` (the built-ins). Nothing here is copied
from them. The fixtures are in `crates/seihai-core/tests/fixtures/gs2/` and
`crates/seihai-core/tests/gs2_opcodes.rs` checks them against this file.

Seihai reads these files with `seihai_core::scripting::gs2::Script::parse`
(`crates/seihai-core/src/scripting/gs2.rs`). It returns the function table,
the string table and one instruction per op index, and `disassemble()` prints
them. It runs nothing. A value this table does not name, or a placeholder,
comes back as `Op::Unknown`.

This is what the open-source compiler **writes**. What the 3.x/4.x clients or
the official server do with each opcode at run time is not in this source.
Where the enum has a comment on meaning, it is the compiler author's own
reading, and it is marked *(enum comment)*. Run-time behaviour is for
Stages C and D, from GServer-v2 and the client decompiles.

Tags: **[src]** read from the compiler source · **[fx]** seen in the fixtures.

## File layout

All integers are big-endian. **[src][fx]**

The file is four segments, each a `u32 type`, a `u32 length` and `length`
bytes, in this order, then one `0x0A` byte that is outside every segment.

| Type | Segment | Body |
|---|---|---|
| 1 | GS1 event flags | one `u32` bit mask. This compiler always writes 0 |
| 2 | Function table | per function: `u32` op index of its entry, then the name, NUL-terminated |
| 3 | String table | NUL-terminated strings. Index 0 is the first |
| 4 | Bytecode | opcodes and their operands |

- **Op index** counts opcodes, not bytes: the first opcode is 0, the next
  one is 1, whatever operand bytes come between. Jump targets and the
  function table both use op indices.
- Function names: `public.` is put in front for a `public function`. A
  method is `obj.name`. A function on `universe` is written as
  `name,universe.name` (with `public.` in front of both parts if public).
- The compiler appends one `OP_RET` after the last statement of the
  script. The source says this works around the last function being
  uncallable on the client the author tested with.

## Operands

An operand follows its opcode directly. It starts with a marker byte. No
opcode value is 0xF0 or higher, so a marker byte can't be mistaken for an
opcode. **[src][fx]**

| Marker | Payload | Used for |
|---|---|---|
| `F0` | `u8` | string-table index |
| `F1` | `u16` | string-table index |
| `F2` | `u32` | string-table index |
| `F3` | `i8` | integer, or op index of a jump target |
| `F4` | `i16` | integer, or op index of a jump target |
| `F5` | `i32` | integer, or op index of a jump target |
| `F6` | NUL-terminated decimal text | a non-integer number (`0.5` is `F6 30 2E 35 00`) |

The writer picks the smallest marker that holds the value. Forward jumps
are an exception: they are always written as `F4` and a 16-bit placeholder,
which is patched once the target is known. Backward jumps (the end of a
loop) use the smallest `F3`/`F4`/`F5`. Integers are signed (`-1` is
`F3 FF`). String indices are unsigned. The `F1`, `F2` and `F5` widths are in
the writer but not in the fixtures, apart from `F5` for 100000.

Column **Operand**: `—` none · `str` a string index (`F0`–`F2`) · `num` an
integer (`F3`–`F5`) or a decimal (`F6`) · `jump` an op index (`F3`–`F5`).

## Opcodes

Column **Emitted**: `yes` the compiler writes it · `no` it is in the enum
but nothing in the compiler writes it.

| Dec | Hex | Name | Operand | What the compiler emits it for | Emitted |
|---|---|---|---|---|---|
| 0 | 0x00 | OP_NONE | — | Never written. Used inside the compiler for "no opcode" | no |
| 1 | 0x01 | OP_SET_INDEX | jump | Unconditional jump: over a function body, to the end of an `if`'s else, back to a loop's start, out of a `break`. *(enum comment: likely a jump)* | yes |
| 2 | 0x02 | OP_SET_INDEX_TRUE | jump | `switch`: jump to a case when the value matches | yes |
| 3 | 0x03 | OP_OR | jump | `\|\|`: jump past the right side | yes |
| 4 | 0x04 | OP_IF | jump | `if`, loop tests, `&&`: jump to the target when the test fails. *(enum comment: likely jump-if-not)* | yes |
| 5 | 0x05 | OP_AND | jump | `&&` outside an `if` or loop test: jump past the right side | yes |
| 6 | 0x06 | OP_CALL | — | Call by name. The name is pushed last, as `OP_TYPE_VAR`. Arguments are pushed in reverse after an `OP_TYPE_ARRAY` | yes |
| 7 | 0x07 | OP_RET | — | `return`, and the implicit `return 0` at the end of a function | yes |
| 8 | 0x08 | OP_SLEEP | — | `sleep(t)` | yes |
| 9 | 0x09 | OP_CMD_CALL | — | Start of a function body that contains a call, and the start of each loop body. *(enum comment: may count loop passes for the 10k limit)* | yes |
| 10 | 0x0A | OP_JMP | — | After a function's parameter list, before its body. No operand is written. *(enum comment: a jump by byte offset, which the writer does not do)* | yes |
| 11 | 0x0B | OP_WAITFOR | — | `waitfor(obj, event, timeout)` | yes |
| 20 | 0x14 | OP_TYPE_NUMBER | num | Number constant | yes |
| 21 | 0x15 | OP_TYPE_STRING | str | String constant | yes |
| 22 | 0x16 | OP_TYPE_VAR | str | Variable or member name, and the name before `OP_CALL` | yes |
| 23 | 0x17 | OP_TYPE_ARRAY | — | Start of an argument list, a parameter list or an array literal | yes |
| 24 | 0x18 | OP_TYPE_TRUE | — | `true` | yes |
| 25 | 0x19 | OP_TYPE_FALSE | — | `false` | yes |
| 26 | 0x1A | OP_TYPE_NULL | — | `null` | yes |
| 27 | 0x1B | OP_PI | — | `pi` | yes |
| 30 | 0x1E | OP_COPY_LAST_OP | — | Duplicates the top value: compound assignments, `++`/`--` used as a value, `switch` | yes |
| 31 | 0x1F | OP_SWAP_LAST_OPS | — | Swaps the top two values: postfix `++`/`--`, `addcontrol` | yes |
| 32 | 0x20 | OP_INDEX_DEC | — | Drops the top value (an unused call result, a loop's index) | yes |
| 33 | 0x21 | OP_CONV_TO_FLOAT | — | Converts to a number | yes |
| 34 | 0x22 | OP_CONV_TO_STRING | — | Converts to a string | yes |
| 35 | 0x23 | OP_MEMBER_ACCESS | — | `a.b` | yes |
| 36 | 0x24 | OP_CONV_TO_OBJECT | — | Converts to an object (method calls, `with`, `for`-each, `in`) | yes |
| 37 | 0x25 | OP_ARRAY_END | — | End of an array literal `{…}` | yes |
| 38 | 0x26 | OP_ARRAY_NEW | — | `new[n]` | yes |
| 39 | 0x27 | OP_SETARRAY | — | `setarray(var, n)` | yes |
| 40 | 0x28 | OP_INLINE_NEW | — | `new T(name)` and the `new T(name) { … }` statement, after the name | yes |
| 41 | 0x29 | OP_MAKEVAR | — | `makevar(s)` | yes |
| 42 | 0x2A | OP_NEW_OBJECT | — | `new T(…)`, after the type name | yes |
| 43 | 0x2B | OP_OBJ_FROM_STR | — | *(enum comment: from setstring)* | no |
| 44 | 0x2C | OP_INLINE_CONDITIONAL | — | End of an `&&`/`\|\|` chain outside an `if` or loop test | yes |
| 45–47 | 0x2D–0x2F | OP_UNKNOWN_45 … 47 | — | Placeholders in the enum | no |
| 50 | 0x32 | OP_ASSIGN | — | `=` and the store of a compound assignment | yes |
| 51 | 0x33 | OP_FUNC_PARAMS_END | — | End of a function's parameter list | yes |
| 52 | 0x34 | OP_INC | — | `++`, and the index step in `for`-each | yes |
| 53 | 0x35 | OP_DEC | — | `--` | yes |
| 54 | 0x36 | OP_UNKNOWN_54 | — | Placeholder | no |
| 60 | 0x3C | OP_ADD | — | `+`, `+=` | yes |
| 61 | 0x3D | OP_SUB | — | `-`, `-=` | yes |
| 62 | 0x3E | OP_MUL | — | `*`, `*=` | yes |
| 63 | 0x3F | OP_DIV | — | `/`, `/=` | yes |
| 64 | 0x40 | OP_MOD | — | `%`, `%=` | yes |
| 65 | 0x41 | OP_POW | — | `^`, `pow(a, b)`. `^=` has no opcode in the compiler's operator map | yes |
| 66–67 | 0x42–0x43 | OP_UNKNOWN_66, 67 | — | Placeholders | no |
| 68 | 0x44 | OP_NOT | — | `!` | yes |
| 69 | 0x45 | OP_UNARYSUB | — | Unary `-` | yes |
| 70 | 0x46 | OP_EQ | — | `==`, and the `switch` case test | yes |
| 71 | 0x47 | OP_NEQ | — | `!=` | yes |
| 72 | 0x48 | OP_LT | — | `<` | yes |
| 73 | 0x49 | OP_GT | — | `>` | yes |
| 74 | 0x4A | OP_LTE | — | `<=` | yes |
| 75 | 0x4B | OP_GTE | — | `>=` | yes |
| 76 | 0x4C | OP_BWO | — | `\|` | yes |
| 77 | 0x4D | OP_BWA | — | `&` | yes |
| 78 | 0x4E | OP_BWX | — | `xor` | yes |
| 79 | 0x4F | OP_BWI | — | `~` | yes |
| 80 | 0x50 | OP_IN_RANGE | — | `x in \|a, b\|` and `x in <a, b>` | yes |
| 81 | 0x51 | OP_IN_OBJ | — | `x in obj` | yes |
| 82 | 0x52 | OP_OBJ_INDEX | — | `obj.index(x)` | yes |
| 83 | 0x53 | OP_OBJ_TYPE | — | `obj.type()`. *(enum comment: 0 float, 1 string, 2 object, 3 array)* | yes |
| 84 | 0x54 | OP_FORMAT | — | `format(fmt, …)` | yes |
| 85 | 0x55 | OP_INT | — | The `int(x)` cast | yes |
| 86 | 0x56 | OP_ABS | — | `abs(x)` | yes |
| 87 | 0x57 | OP_RANDOM | — | `random(a, b)` | yes |
| 88 | 0x58 | OP_SIN | — | `sin(x)` | yes |
| 89 | 0x59 | OP_COS | — | `cos(x)` | yes |
| 90 | 0x5A | OP_ARCTAN | — | `arctan(x)` | yes |
| 91 | 0x5B | OP_EXP | — | `exp(x)` | yes |
| 92 | 0x5C | OP_LOG | — | `log(a, b)` | yes |
| 93 | 0x5D | OP_MIN | — | `min(a, b)` | yes |
| 94 | 0x5E | OP_MAX | — | `max(a, b)` | yes |
| 95 | 0x5F | OP_GETANGLE | — | `getangle(a, b)` | yes |
| 96 | 0x60 | OP_GETDIR | — | `getdir(a, b)` | yes |
| 97 | 0x61 | OP_VECX | — | `vecx(dir)` | yes |
| 98 | 0x62 | OP_VECY | — | `vecy(dir)` | yes |
| 99 | 0x63 | OP_OBJ_INDICES | — | `obj.indices(…)` | yes |
| 100 | 0x64 | OP_OBJ_LINK | — | `obj.link(…)` | yes |
| 101 | 0x65 | OP_BW_LEFTSHIFT | — | `<<`, `<<=` | yes |
| 102 | 0x66 | OP_BW_RIGHTSHIFT | — | `>>`, `>>=` | yes |
| 103 | 0x67 | OP_CHAR | — | `char(n)` | yes |
| 104 | 0x68 | OP_OBJ_COMPARE | — | *(enum comment: "something like" a compare)* | no |
| 110 | 0x6E | OP_OBJ_TRIM | — | `s.trim()` | yes |
| 111 | 0x6F | OP_OBJ_LENGTH | — | `s.length()` | yes |
| 112 | 0x70 | OP_OBJ_POS | — | `s.pos(sub)` | yes |
| 113 | 0x71 | OP_JOIN | — | `@`, `@=`, and `NL`/`SPC`/`TAB` | yes |
| 114 | 0x72 | OP_OBJ_CHARAT | — | `s.charat(i)` | yes |
| 115 | 0x73 | OP_OBJ_SUBSTR | — | `s.substring(…)` | yes |
| 116 | 0x74 | OP_OBJ_STARTS | — | `s.starts(x)` | yes |
| 117 | 0x75 | OP_OBJ_ENDS | — | `s.ends(x)` | yes |
| 118 | 0x76 | OP_OBJ_TOKENIZE | — | `s.tokenize([sep])`. With no argument the compiler pushes `" ,"` | yes |
| 119 | 0x77 | OP_TRANSLATE | — | The `_(s)` cast | yes |
| 120 | 0x78 | OP_OBJ_POSITIONS | — | `s.positions(sub)`. *(enum comment: array of the positions of a substring)* | yes |
| 130 | 0x82 | OP_OBJ_SIZE | — | `obj.size()`, `arraylen(a)`, `sarraylen(a)` | yes |
| 131 | 0x83 | OP_ARRAY | — | `a[i]` read | yes |
| 132 | 0x84 | OP_ARRAY_ASSIGN | — | `a[i] = …` and compound assignments to `a[i]` | yes |
| 133 | 0x85 | OP_ARRAY_MULTIDIM | — | `a[i, j]` read | yes |
| 134 | 0x86 | OP_ARRAY_MULTIDIM_ASSIGN | — | `a[i, j] = …` | yes |
| 135 | 0x87 | OP_OBJ_SUBARRAY | — | `a.subarray(…)` | yes |
| 136 | 0x88 | OP_OBJ_ADDSTRING | — | `a.add(x)` | yes |
| 137 | 0x89 | OP_OBJ_DELETESTRING | — | `a.delete(…)` | yes |
| 138 | 0x8A | OP_OBJ_REMOVESTRING | — | `a.remove(…)` | yes |
| 139 | 0x8B | OP_OBJ_REPLACESTRING | — | `a.replace(…)` | yes |
| 140 | 0x8C | OP_OBJ_INSERTSTRING | — | `a.insert(…)` | yes |
| 141 | 0x8D | OP_OBJ_CLEAR | — | `a.clear()` | yes |
| 142 | 0x8E | OP_ARRAY_NEW_MULTIDIM | — | Each size after the first in `new[n][m]` | yes |
| 150 | 0x96 | OP_WITH | jump | `with (obj)` and the `new T(…) { … }` statement: the target is just past `OP_WITHEND` | yes |
| 151 | 0x97 | OP_WITHEND | — | End of a `with` block | yes |
| 163 | 0xA3 | OP_FOREACH | jump | `for (x : list)`: the target is the loop's exit | yes |
| 180 | 0xB4 | OP_THIS | — | `this` | yes |
| 181 | 0xB5 | OP_THISO | — | `thiso` | yes |
| 182 | 0xB6 | OP_PLAYER | — | `player` | yes |
| 183 | 0xB7 | OP_PLAYERO | — | `playero` | yes |
| 184 | 0xB8 | OP_LEVEL | — | `level` | yes |
| 189 | 0xBD | OP_TEMP | — | `temp` | yes |
| 190 | 0xBE | OP_PARAMS | — | In the enum. The compiler does not map `params` to it | no |

Values that are not in the table (12–19, 28–29, 48–49, 55–59, 105–109,
121–129, 143–149, 152–162, 164–179, 185–188, 191–239) are not in the enum.

## What a few constructs compile to

From the fixtures. Op indices on the left. **[fx]**

`set("gs2seen");` as a statement (`shared.gs2`):

```
21  OP_TYPE_ARRAY
22  OP_TYPE_STRING F0 01        "gs2seen"
23  OP_TYPE_VAR    F0 02        set
24  OP_CALL
25  OP_INDEX_DEC                the result is unused
```

`if (this.count > 10) { … }`:

```
14  OP_THIS
15  OP_TYPE_VAR F0 00           count
16  OP_MEMBER_ACCESS
17  OP_CONV_TO_FLOAT
18  OP_TYPE_NUMBER F3 0A        10
19  OP_GT
20  OP_IF F4 00 22              to op 34 if false
```

A function: an `OP_SET_INDEX` that jumps over the body, then at the op index
in the function table `OP_TYPE_ARRAY`, the parameters as `OP_TYPE_VAR` in
reverse order, `OP_FUNC_PARAMS_END`, `OP_JMP`, `OP_CMD_CALL` if the body has
a call, the body, and `OP_TYPE_NUMBER 0`, `OP_RET` unless the body already
ends in a return.

## Built-in functions

The compiler maps these names to their own opcodes. Every other name is a
call by name (`OP_CALL`), and that covers every game command: `say2`, `set`,
`showimg`, `setTimer`, `echo`, `findplayer`, and the rest. So the opcode
table on its own does not say which commands a GS2 script uses. The string
table and the `OP_TYPE_VAR` before each `OP_CALL` do. **[src]**

Column **GS1**: does the classic GS1 engine (`seihai-core`
`scripting/engine/vocab.rs`) already have it? `yes` the same name ·
`var` GS1 has it as a variable, not a function · `no`.

Plain functions:

| Name | Opcode | GS1 |
|---|---|---|
| `sleep` | OP_SLEEP | yes (command) |
| `sin`, `cos`, `arctan` | OP_SIN, OP_COS, OP_ARCTAN | yes |
| `abs` | OP_ABS | yes |
| `random` | OP_RANDOM | yes |
| `arraylen`, `sarraylen` | OP_OBJ_SIZE | yes |
| `setarray` | OP_SETARRAY | yes (command) |
| `vecx`, `vecy` | OP_VECX, OP_VECY | var (from 2.03) |
| `exp`, `log`, `min`, `max`, `pow` | OP_EXP, OP_LOG, OP_MIN, OP_MAX, OP_POW | no |
| `char` | OP_CHAR | no |
| `getangle`, `getdir` | OP_GETANGLE, OP_GETDIR | no |
| `waitfor` | OP_WAITFOR | no |
| `format` | OP_FORMAT | no |
| `makevar` | OP_MAKEVAR | no |

`int(x)` is a cast keyword, not a built-in, and compiles to OP_INT. GS1 has
`int` as a function.

Methods (`obj.name(…)`):

| Name | Opcode | GS1 |
|---|---|---|
| `index`, `type`, `indices`, `link` | OP_OBJ_INDEX, OP_OBJ_TYPE, OP_OBJ_INDICES, OP_OBJ_LINK | no |
| `trim`, `length`, `pos`, `charat`, `substring` | OP_OBJ_TRIM, OP_OBJ_LENGTH, OP_OBJ_POS, OP_OBJ_CHARAT, OP_OBJ_SUBSTR | no (GS1 has `strlen`, `indexof`) |
| `starts`, `ends`, `positions` | OP_OBJ_STARTS, OP_OBJ_ENDS, OP_OBJ_POSITIONS | no (GS1 has `startswith`) |
| `tokenize` | OP_OBJ_TOKENIZE | yes, as the `tokenize` command |
| `size`, `subarray`, `clear` | OP_OBJ_SIZE, OP_OBJ_SUBARRAY, OP_OBJ_CLEAR | no |
| `add`, `delete`, `insert`, `remove`, `replace` | OP_OBJ_ADDSTRING, OP_OBJ_DELETESTRING, OP_OBJ_INSERTSTRING, OP_OBJ_REMOVESTRING, OP_OBJ_REPLACESTRING | yes, as `addstring`, `deletestring`, `insertstring`, `removestring` and `replacestring` (the last three from 2.1) |

GS1's own names are from `vocab.rs` as it is on main. Its era rules are not
repeated here.
