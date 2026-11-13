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
| 4.04r2 | Windows (`GraalEngine.dll`) | Dispatcher `switch` and its jump table |
| 4.20r6 | Mac (PPC Mach-O) | Dispatcher decompiled as one `switch` |
| 4.20r8 | Windows (x86 PE) | Dispatcher `switch`, plus the load-time optimiser |
| 5.00r7 | Windows (x86 PE) | Dispatcher `switch` |
| 5.12 | Windows (`Graal3DEngine.dll`) | Dispatcher compared with 5.00r7's instruction by instruction |
| 5.24r4 | Windows (unpacked from MoleBox) | Dispatcher `switch` over pre-decoded instructions |
| 5.30r3, 5.31r4 | Windows (unpacked from Themida) | As 5.24r4; the two dispatchers are identical |
| 6.00r2, 6.00r4 | Windows (unpacked from Themida) | As 5.30r3; the two dispatchers are identical |
| 6.01r1 | Linux (ELF) | Computed-goto handler table, one function per handler |
| 6.03r7 | Linux (ELF) | Computed-goto handler table, one function per handler |
| 6.03r7 | Windows (unpacked from Themida) | Dispatcher `switch`, the stack and `^` helpers |

Each build and the profile it runs:

| Versions | Profile | Why |
|---|---|---|
| everything before 4.04r2 | 3.153 | Earlier clients have no GS2 VM |
| 4.04r2 (Windows) | 4.04r2 | Read: 4.20r6's handler set and rules on x86 |
| 4.04r2 (Mac), 4.20r6 | 4.20r6 | The Mac 4.04r2 was not read; the Windows 4.04r2 has 4.20r6's rules, and the Mac build is PowerPC as 4.20r6 is |
| 4.20r8 | 4.20r8 | Read |
| 5.00r7, 5.12 | 5.00r7 | Read; 5.12's dispatcher is 5.00r7's |
| 5.12 (Mac) | 5.00r7 with the PowerPC shift rule | Not read; assumed from the Windows 5.12 and the CPU |
| 5.24r4, 5.30r3, 5.31r4, 6.00r2, 6.00r4 | 5.24r4 | Read; the same VM in all five |
| 6.015, 6.034 | 6.03r7 | No binary; 6.01r1 and 6.03r7 on either side share a VM |
| 6.03r7 (Windows and Linux) | 6.03r7 | Read |

The 7.x client (Unity) was not read. The project treats 7.x as low
priority, and no 7.x `ClientVersion` exists yet.

## Opcodes each VM handles

| Build | Handled | Change from the build before |
|---|---|---|
| 3.153 | 1–10, 20–27, 30–47, 50–53, 60–103, 110–120, 130–142, 150, 151, 163, 180–184, 189, 190 | — |
| 4.20r6 | as 3.153, plus 104 | adds `OP_OBJ_COMPARE` |
| 4.20r8 | as 4.20r6, plus 11 and 200–233 | adds `OP_WAITFOR`; the optimiser's own opcodes |
| 4.04r2 | as 4.20r6 | — (the same set; no `waitfor`, no optimiser) |
| 5.00r7 / 5.12 | as 4.20r8, plus 48, 49, 54, 121 | adds object and number-list literals, a member store, add-or-join |
| 5.24r4 to 6.00r4, 6.03r7 (Windows) | as 5.00r7 without 48 and 49, plus 234–242 | the rewritten VM: instructions decoded once at load, a linked value stack, more optimiser opcodes |
| 6.01r1 / 6.03r7 (Linux) | as 5.24r4, plus 0 and 164 | an end-of-script sentinel (164) |

An opcode a VM has no handler for is skipped by the profile and logged
(`not in the <label> VM`).

### Opcodes gs2-parser does not name

| Op | Builds | What it does |
|---|---|---|
| 45 | all | Copies the top into local slot *n* (operand, *n* < 1024) of the running function, without popping it |
| 46 | all | Pushes a copy of local slot *n* |
| 47 | all | Turns the top into a resolved variable reference (creating the variable). With 45/46 this caches a variable for repeated use. Seihai reads variables when they are used, so it leaves the slot as it is |
| 48 | 5.00r7, 5.12 | Object literal from *n* name/value pairs. Seihai has no object values: the pairs are dropped and the result is `null` |
| 49 | 5.00r7, 5.12 | List of *n* numbers (clamped to 0–10000) |
| 54 | 5.00r7 on | `obj.(name) = value` in one step. Pops the value, the member name and the object; converts the object as 36 does, names the member as 35 does, and stores as 50 does. Nothing is left on the stack |
| 66 | all | `a && b` as a value: 1 or 0, both sides evaluated |
| 67 | all | `a \|\| b` as a value: 1 or 0, both sides evaluated |
| 121 | 5.00r7 on | `+` that joins as text when either operand is a string on the stack, else adds |
| 164 | 6.x | Appended at load: ends the script and goes on to the next one. Never in a file |

### Named opcodes with version notes

- **11 `OP_WAITFOR`** (4.20r8 on) pops the timeout, the event and the object.
  When the event is not empty and the timeout is above 0, the VM registers
  the event, saves the run and returns; the run resumes with 1 when the event
  comes. Otherwise it pushes 0. Seihai returns `Outcome::WaitFor`; the owner
  calls `Machine::resume_waitfor(caught, host)`, which pushes 1 or 0. The
  timeout path was followed in 5.12 and 6.00r2: the wait is saved with 1 as
  its result and an end time (the clock plus the timeout); the timer
  resumes a wait once the clock plus 0.0001 is past that time, after it
  sets the end time and the result to 0. So a timed-out `waitfor` gives 0.
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

| Limit | 3.153 | 4.04r2, 4.20r6 | 4.20r8 | 5.00r7, 5.12 | 5.24r4 to 6.00r4 | 6.03r7 |
|---|---|---|---|---|---|---|
| Value stack | 256 | 10000 | 10000 | 10000 | 10000 | none |
| Loop passes (per loop) | 10000 | 10000 | 10000 | 10000, `maxlooplimit` | 10000, `maxlooplimit` | 10000, settable |
| Call depth | 10000 | 10000 | 10000 | 10000 | 10000 | 10000 |

5.00r7 adds `this.maxlooplimit` (`TGraalVar::setMaxLoopLimit`), clamped to
100–1000000. 6.x keeps the limit in a field of the script object (default
10000); its property name is scrambled in the binary and was not recovered,
so Seihai gives 6.x the 5.00r7 name. 5.24r4 to 5.31r4 still name it
`maxlooplimit`; 6.00r2 and 6.00r4 have the field without the plain name.

## Maths and text

- **Integer conversion** (`int()`, bitwise operators, indices): every VM
  computes `floor(x + 0.0001)`. Seihai used `floor(x)`.
- **`a % b`**: 0 when `b` is 0. 4.20r6 and 4.20r8 compute
  `a - floor(a/b + 0.0001) * b`, and so does 4.04r2; 3.153, 5.00r7 and
  everything after drop the tolerance (3.153 adds and removes it).
- **`a ^ b`**: a positive base goes to `pow`; a zero base gives 0; a negative
  base gives 1 for `b == 0`, `±pow(-a, b)` by parity for an integer `b`,
  `1/pow(a, -b)` for a negative `b`, else 0. 4.x and 5.x add `b == 0.5` →
  `sqrt`. From 5.24r4 the VM checks `b == 2` first (`a * a`, the same value
  as `pow`). 6.03r7 (Windows and Linux) computes a positive base's square
  root in single precision; 5.24r4 to 6.00r4 keep double precision.
- **Shifts**: the PowerPC builds (3.153, 4.20r6) take the count modulo 64 and
  give 0 or -1 past 31. The x86 builds (4.04r2, 4.20r8 on) take the count
  modulo 32.
- **Comparisons**: two numbers are equal within 0.0001 in every VM; a string
  against a number compares as the number it reads as; two strings by bytes.
- **Number to text** (the same in 3.153 and 6.x): `|x| < 0.0001` is `"0"`;
  otherwise the sign, the integer part (truncated, at most 2147483647), and up
  to 9 decimals, truncated with trailing zeros dropped, where a fraction
  ending in 999 is carried up. Seihai's `+` join, `format("%f")` and
  `format("%s")` now write numbers this way.

## Not known

- In which order 48 and 49 take their operands.
- The 6.x name for the loop-limit property.
- The Mac 4.04r2 and Mac 5.12 VMs, and 7.x, were not read.
