# `pokereval` in GS1

The same hand scorer as [the GS2 version](pokereval-gs2.md), for GS1 (the
NPC scripting of Graal 2.x). It needs Graal 2.02 or later for `tokenize`,
`#t` and `#e`. How it scores hands is in
[pokereval-explained.md](pokereval-explained.md).

## Calling it

GS1 functions take no arguments and return nothing, so `pokereval` reads the
**tokens** and leaves its answer in variables:

```
if (playerchats) {
  // The player says "As Ad Kh 7s 7c 2h 9d".
  tokenize #c;
  pokereval();
  if (this.pokerscore < 0) message I can't read that hand;
  else message #s(pokername): #v(this.pokerscore);
}
```

| In | Meaning |
|---|---|
| the tokens | 5 to 7 cards, from `tokenize <cards>;` (rank `2`–`9`, `T`, `J`, `Q`, `K`, `A`, suit `c`, `d`, `h`, `s`, either case) |

| Out | Meaning |
|---|---|
| `this.pokerscore` | higher is better, equal is a tie, -1 for a card it cannot read |
| `this.pokercat` | the category, 0 (high card) to 8 (straight flush) |
| string `pokername` | the category's name |

The scores are the same numbers the GS2 version gives, so the two can be
compared. To find a winner, score each player in turn and keep the highest
`this.pokerscore` in a variable of your own, as `pokereval` overwrites it.

The function keeps its working values in `this.pk…` variables (`this.pkrc`,
`this.pkheld`, `this.pki`, …), since GS1 has no local variables. Don't use
names starting with `this.pk` for anything else in the same script.

## The code

The file is
[`crates/seihai-core/tests/fixtures/poker/pokereval.gs1`](../../crates/seihai-core/tests/fixtures/poker/pokereval.gs1);
it is copied here unchanged.

```
// pokereval for GS1 (Graal 2.02 and later: tokenize, #t, #e, #v).
// GS1 functions take no arguments and return nothing, so pokereval
// reads the tokens and sets variables:
//   in:  the tokens: 5 to 7 cards, from  tokenize As Kd Tc 9h 2s;
//        or  tokenize #c;  for what the player said.
//        Rank 2 3 4 5 6 7 8 9 T J Q K A, suit c d h s (either case).
//   out: this.pokerscore  higher is better, equal is a tie, -1 a bad card
//        this.pokercat    0 high card, 1 pair, 2 two pair, 3 three of a
//                         kind, 4 straight, 5 flush, 6 full house,
//                         7 four of a kind, 8 straight flush
//        the string pokername, the category's name.
// It uses this.pk* variables as scratch.
function pokereval() {
  setarray this.pkrc,13;
  setarray this.pksc,4;
  setarray this.pksr,52;
  setarray this.pkheld,13;
  setarray this.pkk,5;
  setarray this.pkd,5;
  this.pokerscore = -1;
  this.pokercat = -1;
  setstring pokername,bad card;
  this.pkbad = 0;
  // setarray does not promise zeros, so clear the counts.
  for (this.pki = 0; this.pki < 52; this.pki++) {
    this.pksr[this.pki] = 0;
    if (this.pki < 13) this.pkrc[this.pki] = 0;
    if (this.pki < 4) this.pksc[this.pki] = 0;
    if (this.pki < 5) this.pkd[this.pki] = 0;
  }
  for (this.pki = 0; this.pki < tokenscount; this.pki++) {
    // The rank and suit letters, either case.
    this.pkr = -1;
    this.pks = -1;
    for (this.pkj = 0; this.pkj < 13; this.pkj++) {
      if (strequals(#e(0,1,#t(#v(this.pki))),#e(#v(this.pkj),1,23456789TJQKA))
          || strequals(#e(0,1,#t(#v(this.pki))),#e(#v(this.pkj),1,23456789tjqka)))
        this.pkr = this.pkj;
      if (this.pkj < 4) {
        if (strequals(#e(1,1,#t(#v(this.pki))),#e(#v(this.pkj),1,CDHS))
            || strequals(#e(1,1,#t(#v(this.pki))),#e(#v(this.pkj),1,cdhs)))
          this.pks = this.pkj;
      }
    }
    if (this.pkr < 0 || this.pks < 0) this.pkbad = 1;
    else {
      this.pkrc[this.pkr] = this.pkrc[this.pkr] + 1;
      this.pksc[this.pks] = this.pksc[this.pks] + 1;
      this.pksr[this.pks * 13 + this.pkr] = 1;
    }
  }
  if (this.pkbad == 0) pokerclassify();
}

function pokerclassify() {
  // A flush (five or more of one suit): straight flush, else the top five.
  this.pkflush = -1;
  for (this.pks = 0; this.pks < 4; this.pks++) {
    if (this.pksc[this.pks] >= 5) this.pkflush = this.pks;
  }
  if (this.pkflush >= 0) {
    for (this.pkr = 0; this.pkr < 13; this.pkr++)
      this.pkheld[this.pkr] = this.pksr[this.pkflush * 13 + this.pkr];
    pokerstraight();
    if (this.pkhi >= 0) {
      this.pkcat = 8;
      this.pkd[0] = this.pkhi;
    } else {
      this.pkskip1 = -1;
      this.pkskip2 = -1;
      this.pkn = 5;
      pokerkick();
      this.pkcat = 5;
      for (this.pki = 0; this.pki < 5; this.pki++) this.pkd[this.pki] = this.pkk[this.pki];
    }
    pokerscore();
  } else pokergroups();
}

// No flush: groups by size, the highest rank first.
function pokergroups() {
  this.pkquad = -1;
  this.pktrip = -1;
  this.pktrip2 = -1;
  this.pkpair = -1;
  this.pkpair2 = -1;
  for (this.pkr = 12; this.pkr >= 0; this.pkr--) {
    this.pkheld[this.pkr] = this.pkrc[this.pkr];
    this.pkc = this.pkrc[this.pkr];
    if (this.pkc == 4 && this.pkquad < 0) this.pkquad = this.pkr;
    else if (this.pkc == 3) {
      if (this.pktrip < 0) this.pktrip = this.pkr;
      else if (this.pktrip2 < 0) this.pktrip2 = this.pkr;
    } else if (this.pkc == 2) {
      if (this.pkpair < 0) this.pkpair = this.pkr;
      else if (this.pkpair2 < 0) this.pkpair2 = this.pkr;
    }
  }
  pokerstraight();
  this.pkskip1 = -1;
  this.pkskip2 = -1;
  if (this.pkquad >= 0) {
    this.pkcat = 7;
    this.pkskip1 = this.pkquad;
    this.pkn = 1;
    pokerkick();
    this.pkd[0] = this.pkquad;
    this.pkd[1] = this.pkk[0];
  } else if (this.pktrip >= 0 && (this.pktrip2 >= 0 || this.pkpair >= 0)) {
    // Full house: the second set of three can be the pair.
    this.pkcat = 6;
    this.pkd[0] = this.pktrip;
    this.pkd[1] = this.pkpair;
    if (this.pktrip2 > this.pkpair) this.pkd[1] = this.pktrip2;
  } else if (this.pkhi >= 0) {
    this.pkcat = 4;
    this.pkd[0] = this.pkhi;
  } else if (this.pktrip >= 0) {
    this.pkcat = 3;
    this.pkskip1 = this.pktrip;
    this.pkn = 2;
    pokerkick();
    this.pkd[0] = this.pktrip;
    this.pkd[1] = this.pkk[0];
    this.pkd[2] = this.pkk[1];
  } else if (this.pkpair2 >= 0) {
    // A third pair's rank can still be the kicker.
    this.pkcat = 2;
    this.pkskip1 = this.pkpair;
    this.pkskip2 = this.pkpair2;
    this.pkn = 1;
    pokerkick();
    this.pkd[0] = this.pkpair;
    this.pkd[1] = this.pkpair2;
    this.pkd[2] = this.pkk[0];
  } else if (this.pkpair >= 0) {
    this.pkcat = 1;
    this.pkskip1 = this.pkpair;
    this.pkn = 3;
    pokerkick();
    this.pkd[0] = this.pkpair;
    this.pkd[1] = this.pkk[0];
    this.pkd[2] = this.pkk[1];
    this.pkd[3] = this.pkk[2];
  } else {
    this.pkcat = 0;
    this.pkn = 5;
    pokerkick();
    for (this.pki = 0; this.pki < 5; this.pki++) this.pkd[this.pki] = this.pkk[this.pki];
  }
  pokerscore();
}

// this.pkhi: the top card of the best straight in this.pkheld, or -1.
// A-2-3-4-5 (the wheel) is five-high.
function pokerstraight() {
  this.pkhi = -1;
  this.pkrun = 0;
  for (this.pkj = 12; this.pkj >= 0 && this.pkhi < 0; this.pkj--) {
    if (this.pkheld[this.pkj] > 0) {
      this.pkrun++;
      if (this.pkrun == 5) this.pkhi = this.pkj + 4;
    } else this.pkrun = 0;
  }
  if (this.pkhi < 0 && this.pkrun == 4 && this.pkheld[12] > 0) this.pkhi = 3;
}

// this.pkk[0..]: the this.pkn highest ranks in this.pkheld, skipping
// this.pkskip1 and this.pkskip2, highest first; the rest are 0.
function pokerkick() {
  for (this.pkj = 0; this.pkj < 5; this.pkj++) this.pkk[this.pkj] = 0;
  this.pkm = 0;
  for (this.pkj = 12; this.pkj >= 0 && this.pkm < this.pkn; this.pkj--) {
    if (this.pkheld[this.pkj] > 0 && this.pkj != this.pkskip1 && this.pkj != this.pkskip2) {
      this.pkk[this.pkm] = this.pkj;
      this.pkm++;
    }
  }
}

// this.pokerscore = this.pkcat, then this.pkd[0..4], as base-13 digits.
function pokerscore() {
  this.pokercat = this.pkcat;
  this.pokerscore = this.pkcat;
  for (this.pkj = 0; this.pkj < 5; this.pkj++) {
    this.pokerscore = this.pokerscore * 13 + this.pkd[this.pkj];
    this.pkd[this.pkj] = 0;
  }
  if (this.pkcat == 0) setstring pokername,high card;
  if (this.pkcat == 1) setstring pokername,pair;
  if (this.pkcat == 2) setstring pokername,two pair;
  if (this.pkcat == 3) setstring pokername,three of a kind;
  if (this.pkcat == 4) setstring pokername,straight;
  if (this.pkcat == 5) setstring pokername,flush;
  if (this.pkcat == 6) setstring pokername,full house;
  if (this.pkcat == 7) setstring pokername,four of a kind;
  if (this.pkcat == 8) setstring pokername,straight flush;
}
```

## Notes on the GS1 form

- **Parsing a card.** GS1 has no `pos`, so each rank letter is tried in turn
  with `strequals(#e(0,1,card), #e(#v(this.pkj),1,23456789TJQKA))`. Both an
  upper and a lower case list are tried, so the test does not depend on
  whether `strequals` ignores case.
- **`#v` inside `#t` and `#e`.** `#t(this.pki)` is not a token number in GS1:
  the inside of a message code is text. `#t(#v(this.pki))` is.
- **Arrays.** `setarray` sizes an array; the values are cleared by hand,
  since keeping zeros after a second `setarray` is not something to rely on.
- **No `return`.** The flush and no-flush cases are two functions
  (`pokerclassify` and `pokergroups`) so nothing has to stop half way.

## Tested

`crates/seihai-core/tests/pokereval.rs` runs this file two ways: on Seihai's
classic GS1 engine, and compiled from GS1 to GS2 bytecode by Seihai's
compiler and run on its GS2 machine (the path a 5.x or 6.x client takes).
Both give the GS2 version's scores for every category, every tie-break case
and 1,200 random deals checked against the test's own evaluator.

Writing it found one gap in the GS1-to-GS2 compiler, fixed in the same
change: a `#v(..)` number inside `#t(..)` or `#e(..)` was compiled as the
text `#v(..)` (so 0), where the GS1 engine reads the expression. Two other
differences were found and are left as they are, as they need a real client
to settle: on the compiled path a plain string written by `setstring` is a
`client.gs1_` variable but `#s(name)` reads `player.name`, and GS1
`indexof(a,b)` compiles to `a.pos(b)` while the GS1 engine finds `a` in `b`.
`pokereval` avoids both.

It has not been run on an original Graal client or NPC-Server.
