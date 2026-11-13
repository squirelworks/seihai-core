# GS2 VMs by client version

The GS2 executor (`scripting::gs2::exec`) was first written from the 3.153
client's VM. Later clients run the same bytecode with a different VM: they
handle more opcodes, rewrite some sequences when a script loads, and change a
few limits and maths rules. This page records what was read in each client
and how `scripting::gs2::vm::VmProfile` follows it.

[`Machine::new`] keeps 3.153's rules. [`Machine::for_version`] picks the
profile for a `ClientVersion`.

## What was read

| Build | Binary | How the VM was found |
|---|---|---|
| 3.153 | Mac (PPC Mach-O) | Dispatcher decompiled as one `switch` |
| 4.20r6 | Mac (PPC Mach-O) | Dispatcher decompiled as one `switch` |
| 4.20r8 | Windows (x86 PE) | Dispatcher `switch`, plus the load-time optimiser |
| 5.00r7 | Windows (x86 PE) | Dispatcher `switch` |
| 6.01r1 | Linux (ELF) | Computed-goto handler table, one function per handler |
| 6.03r7 | Linux (ELF) | Computed-goto handler table, one function per handler |

Builds that were not read take the closest earlier VM that was:

| Versions | Profile |
|---|---|
| everything before 4.04r2 | 3.153 |
| 4.04r2 (Windows and Mac), 4.20r6 | 4.20r6 |
| 4.20r8 | 4.20r8 |
| 5.00r7, 5.12, 5.24r4, 5.30r3, 5.31r4 | 5.00r7 |
| 5.12 (Mac) | 5.00r7 with the PowerPC shift rule (assumed) |
| 6.00r2, 6.00r4, 6.015, 6.034, 6.03r7 (both) | 6.03r7 (6.01r1 has the same handler set) |

The 7.x client (Unity) was not read.

## Opcodes each VM handles

| Build | Handled | Change from the build before |
|---|---|---|
| 3.153 | 1–10, 20–27, 30–47, 50–53, 60–103, 110–120, 130–142, 150, 151, 163, 180–184, 189, 190 | — |
| 4.20r6 | as 3.153, plus 104 | adds `OP_OBJ_COMPARE` |
| 4.20r8 | as 4.20r6, plus 11 and 200–233 | adds `OP_WAITFOR`; the optimiser's own opcodes |
| 5.00r7 | as 4.20r8, plus 48, 49, 54, 121 | adds object and number-list literals, a member store, add-or-join |
| 6.01r1 / 6.03r7 | as 5.00r7 without 48 and 49, plus 0, 164, 234–242 | an end-of-script sentinel (164) and more optimiser opcodes |

An opcode a VM has no handler for is skipped by the profile and logged
(`not in the <label> VM`).

### Opcodes gs2-parser does not name

| Op | Builds | What it does |
|---|---|---|
| 45 | all | Copies the top into local slot *n* (operand, *n* < 1024) of the running function, without popping it |
| 46 | all | Pushes a copy of local slot *n* |
| 47 | all | Turns the top into a resolved variable reference (creating the variable). With 45/46 this caches a variable for repeated use. Seihai reads variables when they are used, so it leaves the slot as it is |
| 48 | 5.00r7 only | Object literal from *n* name/value pairs. Seihai has no object values: the pairs are dropped and the result is `null` |
| 49 | 5.00r7 only | List of *n* numbers (clamped to 0–10000) |
| 54 | 5.00r7 on | A member store with three operands. Not understood; skipped |
| 66 | all | `a && b` as a value: 1 or 0, both sides evaluated |
| 67 | all | `a \|\| b` as a value: 1 or 0, both sides evaluated |
| 121 | 5.00r7 on | `+` that joins as text when either operand is a string on the stack, else adds |
| 164 | 6.x | Appended at load: ends the script and goes on to the next one. Never in a file |

### Named opcodes with version notes

- **11 `OP_WAITFOR`** (4.20r8 on) pops the timeout, the event and the object.
  When the event is not empty and the timeout is above 0, the VM registers
  the event, saves the run and returns; the run resumes with 1 when the event
  comes. Otherwise it pushes 0. Seihai returns `Outcome::WaitFor`; the owner
  calls `Machine::resume_waitfor(caught, host)`, which pushes 1 or 0. That
  the timeout gives 0 is assumed (the timeout path was not followed).
- **104 `OP_OBJ_COMPARE`** (4.20r6 on) compares the two values and leaves
  -1, 0 or 1.

### The load-time optimiser (4.20r8 on)

When a script loads, 4.20r8 and later rewrite common sequences into single
internal opcodes. Compilers never write these, and Seihai does not need them
to run bytecode, but they explain the handlers above 200:

| Internal | Fuses |
|---|---|
| 200–207 | `OP_TYPE_NUMBER` + arithmetic 60–67 |
| 208–215 | arithmetic 60–67 + `OP_ASSIGN` |
| 216–223 | `OP_TYPE_NUMBER` + arithmetic + `OP_ASSIGN` |
| 224–227 | `OP_TYPE_NUMBER` + compare 72–75 |
| 228–233 | slot load 46 + 30, 33, 34, 36, or 52/53 with a pop |
| 234–242 (6.x) | variable + member access, variable + call (241), 47 + 45 (242), number + array index (240) |

## Limits

| Limit | 3.153 | 4.20r6 | 4.20r8 | 5.00r7 | 6.x |
|---|---|---|---|---|---|
| Value stack | 256 | 10000 | 10000 | 10000 | none |
| Loop passes (per loop) | 10000 | 10000 | 10000 | 10000, `maxlooplimit` | 10000, settable |
| Call depth | 10000 | 10000 | 10000 | 10000 | 10000 |

5.00r7 adds `this.maxlooplimit` (`TGraalVar::setMaxLoopLimit`), clamped to
100–1000000. 6.x keeps the limit in a field of the script object (default
10000); its property name is scrambled in the binary and was not recovered,
so Seihai gives 6.x the 5.00r7 name.

## Maths and text

- **Integer conversion** (`int()`, bitwise operators, indices): every VM
  computes `floor(x + 0.0001)`. Seihai used `floor(x)`.
- **`a % b`**: 0 when `b` is 0. 4.20r6 and 4.20r8 compute
  `a - floor(a/b + 0.0001) * b`; 3.153, 5.00r7 and 6.x drop the tolerance
  (3.153 adds and removes it).
- **`a ^ b`**: a positive base goes to `pow`; a zero base gives 0; a negative
  base gives 1 for `b == 0`, `±pow(-a, b)` by parity for an integer `b`,
  `1/pow(a, -b)` for a negative `b`, else 0. 4.x and 5.x add `b == 0.5` →
  `sqrt`. 6.x checks `b == 2` first (`a * a`) and computes a positive base's
  square root in single precision.
- **Shifts**: the PowerPC builds (3.153, 4.20r6) take the count modulo 64 and
  give 0 or -1 past 31. The x86 builds (4.20r8 on) take the count modulo 32.
- **Comparisons**: two numbers are equal within 0.0001 in every VM; a string
  against a number compares as the number it reads as; two strings by bytes.
- **Number to text** (the same in 3.153 and 6.x): `|x| < 0.0001` is `"0"`;
  otherwise the sign, the integer part (truncated, at most 2147483647), and up
  to 9 decimals, truncated with trailing zeros dropped, where a fraction
  ending in 999 is carried up. Seihai's `+` join, `format("%f")` and
  `format("%s")` now write numbers this way.

## Not known

- What 54 stores, and in which order 48 and 49 take their operands.
- The value `waitfor` gives when its timeout runs out.
- The 6.x name for the loop-limit property.
- 4.04r2 (both builds), 5.12–5.31r4 (both 5.12 builds), 6.00r2 and 6.00r4 were not read.
