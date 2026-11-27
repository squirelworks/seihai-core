# `pokereval` in GS2

`pokereval(cards)` scores the best five-card poker hand found in 5, 6 or 7
cards. It is ordinary GS2: paste it into a weapon, class or NPC script and
call it like any other function. Graal never shipped a built-in of this
name (see [the explainer](pokereval-explained.md#is-pokereval-a-graal-built-in)),
so this is Seihai's own version, written for scripts that need one.

How the score is built, and why the code is laid out as it is, is in
[pokereval-explained.md](pokereval-explained.md). The GS1 version is
[pokereval-gs1.md](pokereval-gs1.md).

## Calling it

```javascript
function onPlayerChats() {
  // "As Ad Kh 7s 7c 2h 9d": two hole cards and five on the board.
  temp.cards = player.chat.tokenize(" ");
  temp.score = pokereval(temp.cards);
  if (temp.score < 0) player.chat = "I can't read that hand";
  else player.chat = pokername(temp.score) @ " (" @ temp.score @ ")";
}
```

To find the winner at a table, score every player's hole cards plus the
board and take the highest score. Equal scores split the pot.

```javascript
function pokerwinners(hands) {        // hands: one card array per seat
  temp.best = -1;
  temp.win = {};
  for (temp.i = 0; temp.i < hands.size(); temp.i++) {
    temp.s = pokereval(hands[temp.i]);
    if (temp.s > temp.best) {
      temp.best = temp.s;
      temp.win = {temp.i};
    } else if (temp.s == temp.best) temp.win.add(temp.i);
  }
  return temp.win;                      // seat numbers; more than one is a split
}
```

| Input | Meaning |
|---|---|
| `cards` | an array of 5 to 7 card strings: rank `2`–`9`, `T`, `J`, `Q`, `K`, `A`, then suit `c`, `d`, `h`, `s`; either case (`"Td"`, `"td"`, `"TD"`) |

| Result | Meaning |
|---|---|
| a number from 0 to 3,313,076 | higher is a better hand; equal is a tie |
| `-1` | a card it cannot read (`"10h"`, `"Xs"`, an empty string) |

`int(score / 371293)` is the category: 0 high card, 1 pair, 2 two pair,
3 three of a kind, 4 straight, 5 flush, 6 full house, 7 four of a kind,
8 straight flush. `pokername(score)` gives its name.

Some scores to check against:

| Hand | Score | `pokername` |
|---|---|---|
| `As Ks Qs Js Ts` | 3,313,076 | straight flush |
| `Ac Ad Kh 7s 4c` | 739,063 | pair |
| `Ac 2d 3h 4s 5c` (the wheel) | 1,570,855 | straight |
| `7c 5d 4h 3s 2c` | 149,747 | high card |

It does not check for duplicate cards: dealing is the caller's job.

## The code

The file is
[`crates/seihai-core/tests/fixtures/poker/pokereval.gs2`](../../crates/seihai-core/tests/fixtures/poker/pokereval.gs2);
it is copied here unchanged.

```javascript
// pokereval(cards): scores the best five-card poker hand in 5 to 7 cards.
// cards: an array of card strings, such as {"As", "Kd", "Tc", "9h", "2s"}.
//   Rank letter: 2 3 4 5 6 7 8 9 T J Q K A (lower case is fine too).
//   Suit letter: c d h s.
// Returns one number: higher is a better hand, equal is a tie (split pot),
// and -1 for a card it cannot read.
//   score = category * 13^5 + five tie-break ranks, as base-13 digits.
//   category = int(score / 371293):
//   0 high card, 1 pair, 2 two pair, 3 three of a kind, 4 straight,
//   5 flush, 6 full house, 7 four of a kind, 8 straight flush.
function pokereval(cards) {
  temp.rc = new[13];        // cards held of each rank (0 = deuce, 12 = ace)
  temp.sc = new[4];         // cards held of each suit
  temp.sr = new[52];        // suit * 13 + rank: 1 when held
  for (temp.i = 0; temp.i < 13; temp.i++) temp.rc[temp.i] = 0;
  for (temp.i = 0; temp.i < 4; temp.i++) temp.sc[temp.i] = 0;
  for (temp.i = 0; temp.i < 52; temp.i++) temp.sr[temp.i] = 0;
  for (temp.c : cards) {
    temp.r = "23456789TJQKA".pos(temp.c.substring(0, 1).upper());
    temp.s = "CDHS".pos(temp.c.substring(1, 1).upper());
    if (temp.r < 0 || temp.s < 0) return -1;
    temp.rc[temp.r]++;
    temp.sc[temp.s]++;
    temp.sr[temp.s * 13 + temp.r] = 1;
  }

  // A flush (five or more of one suit): straight flush, else the top five.
  for (temp.s = 0; temp.s < 4; temp.s++) {
    if (temp.sc[temp.s] >= 5) {
      temp.fl = new[13];
      for (temp.r = 0; temp.r < 13; temp.r++) temp.fl[temp.r] = temp.sr[temp.s * 13 + temp.r];
      temp.hi = pokerstraight(temp.fl);
      if (temp.hi >= 0) return pokerscore(8, {temp.hi});
      return pokerscore(5, pokerkick(temp.fl, -1, -1, 5));
    }
  }

  // Groups by size, the highest rank first.
  temp.quad = -1;
  temp.trip = -1;
  temp.trip2 = -1;
  temp.pair = -1;
  temp.pair2 = -1;
  for (temp.r = 12; temp.r >= 0; temp.r--) {
    temp.n = temp.rc[temp.r];
    if (temp.n == 4 && temp.quad < 0) temp.quad = temp.r;
    else if (temp.n == 3) {
      if (temp.trip < 0) temp.trip = temp.r;
      else if (temp.trip2 < 0) temp.trip2 = temp.r;
    } else if (temp.n == 2) {
      if (temp.pair < 0) temp.pair = temp.r;
      else if (temp.pair2 < 0) temp.pair2 = temp.r;
    }
  }

  if (temp.quad >= 0)
    return pokerscore(7, {temp.quad, pokerkick(temp.rc, temp.quad, -1, 1)[0]});
  if (temp.trip >= 0 && (temp.trip2 >= 0 || temp.pair >= 0)) {
    // Full house: the second set of three can be the pair.
    temp.p = temp.pair;
    if (temp.trip2 > temp.p) temp.p = temp.trip2;
    return pokerscore(6, {temp.trip, temp.p});
  }
  temp.hi = pokerstraight(temp.rc);
  if (temp.hi >= 0) return pokerscore(4, {temp.hi});
  if (temp.trip >= 0) {
    temp.k = pokerkick(temp.rc, temp.trip, -1, 2);
    return pokerscore(3, {temp.trip, temp.k[0], temp.k[1]});
  }
  if (temp.pair2 >= 0) {
    // A third pair's rank can still be the kicker.
    temp.k = pokerkick(temp.rc, temp.pair, temp.pair2, 1);
    return pokerscore(2, {temp.pair, temp.pair2, temp.k[0]});
  }
  if (temp.pair >= 0) {
    temp.k = pokerkick(temp.rc, temp.pair, -1, 3);
    return pokerscore(1, {temp.pair, temp.k[0], temp.k[1], temp.k[2]});
  }
  return pokerscore(0, pokerkick(temp.rc, -1, -1, 5));
}

// The top card of the best straight among the ranks held, or -1.
// held[r] > 0 means rank r is held. A-2-3-4-5 (the wheel) is five-high.
function pokerstraight(held) {
  temp.run = 0;
  for (temp.r = 12; temp.r >= 0; temp.r--) {
    if (held[temp.r] > 0) {
      temp.run++;
      if (temp.run == 5) return temp.r + 4;
    } else temp.run = 0;
  }
  if (temp.run == 4 && held[12] > 0) return 3;
  return -1;
}

// The n highest ranks held, skipping ranks skip1 and skip2, highest first.
function pokerkick(held, skip1, skip2, n) {
  temp.out = {};
  for (temp.r = 12; temp.r >= 0 && temp.out.size() < n; temp.r--) {
    if (held[temp.r] > 0 && temp.r != skip1 && temp.r != skip2) temp.out.add(temp.r);
  }
  return temp.out;
}

// Packs a category and up to five ranks into one comparable number.
function pokerscore(cat, ranks) {
  temp.v = cat;
  for (temp.i = 0; temp.i < 5; temp.i++) {
    temp.d = 0;
    if (temp.i < ranks.size()) temp.d = ranks[temp.i];
    temp.v = temp.v * 13 + temp.d;
  }
  return temp.v;
}

// Names the category of a score.
function pokername(score) {
  if (score < 0) return "bad card";
  temp.names = {"high card", "pair", "two pair", "three of a kind", "straight",
                "flush", "full house", "four of a kind", "straight flush"};
  return temp.names[int(score / 371293)];
}
```

## Tested

`crates/seihai-core/tests/pokereval.rs` compiles this file with Seihai's GS2
compiler and runs it on Seihai's GS2 machine. Every category is checked by
name, the tie-break rules are checked one by one (kickers, the wheel, a
flush's fifth card, full house order, three pairs, a board that plays), and
1,200 random deals of 5, 6 and 7 cards are checked against an independent
evaluator in the test that tries every five-card subset.

Writing it found two gaps in Seihai's GS2 machine, fixed in the same change:
`a[i]++` now changes the element (it changed a copy), and a list returned by a
function can be indexed or sized in place (`f()[1]`, `f().size()` gave 0).

It has not been run on an original Graal client or NPC-Server.
