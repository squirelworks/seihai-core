# How `pokereval` works

`pokereval` takes 5 to 7 playing cards and gives back one number that says
how good the best five-card poker hand among them is. Two hands are compared
by comparing their numbers: the higher number wins and equal numbers tie.
That is all a poker table script needs, whether it is Texas hold'em (two
hole cards plus five on the board), seven-card stud or five-card draw.

The code is in [pokereval-gs2.md](pokereval-gs2.md) (GS2) and
[pokereval-gs1.md](pokereval-gs1.md) (GS1). Both follow the steps below and
give the same numbers.

## Is `pokereval` a Graal built-in?

Not in any build we hold. The name was searched for (and `evalpoker`,
`pokerhand`, `handeval`, `handrank`) in:

- the Graal_analysis, seihai-core and seihai-servertools repos, including
  the GS1 command tables, the GS2 opcode and built-in registry notes and the
  6.01 registration tables;
- the strings (ASCII and UTF-16) of 169 client, server and tool binaries in
  the project files, among them the 6.0.3.7 client, whose built-in help text
  lists the GS2 functions by name (`findplayerbycommunityname`, `tokenize`,
  `makescreenshot2`, …);
- the scripts and NPC-Server builds on the archive drives.

None of them has it. The only poker in the collection is the iOS game
*Adrenaline Poker Online* (versions 1.2 and 1.60), a separate native app.

"The golden function" fits that: a function a scripter wrote in GS and
passed around, not something the engine provides. Seihai's compiler accepts
a call to `pokereval` either way (GS2 compiles every call by name); without a
script that defines it, Seihai's machine logs "function pokereval not found"
and gives 0, as a real client does for any unknown function.

## Step 1: read the cards

Each card is two characters, rank then suit:

| Rank | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 | T | J | Q | K | A |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| Number | 0 | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 | 10 | 11 | 12 |

Suits are `c` `d` `h` `s` (clubs, diamonds, hearts, spades), numbered 0 to 3.
The rank number is the card's position in the text `23456789TJQKA`, so
higher numbers are higher cards and the ace is 12.

While reading, the function fills three tables:

- **rank counts**: how many cards of each rank (13 entries);
- **suit counts**: how many cards of each suit (4 entries);
- **cards held**: one entry per card in the deck (52), set to 1 when held,
  at `suit × 13 + rank`.

A card it can't read stops it with -1.

## Step 2: find the category

Poker hands fall into nine categories. The function checks from the best
down and stops at the first that fits, so a hand is always given its best
reading.

| Category | Number | What it checks |
|---|---|---|
| straight flush | 8 | a suit with 5 or more cards, and those cards hold a straight |
| four of a kind | 7 | a rank count of 4 |
| full house | 6 | a rank count of 3, and another of 3 or 2 |
| flush | 5 | a suit with 5 or more cards |
| straight | 4 | five ranks in a row are held |
| three of a kind | 3 | a rank count of 3 |
| two pair | 2 | two rank counts of 2 |
| pair | 1 | one rank count of 2 |
| high card | 0 | anything else |

The code checks for a flush first, before four of a kind and full house,
which looks out of order but is safe with seven cards or fewer. A flush takes
five cards of one suit, leaving at most two others. Four of a kind has four
different suits, so at most one of its cards is in the flush suit and three
must be among the others. A full house likewise needs at least three cards
outside the flush suit. Seven cards can't do either, so when there is a
flush, the flush (or a straight flush) is the answer.

**Straights.** The function walks the ranks from ace down to deuce,
counting how many in a row are held. When the count reaches five, the
straight's top card is the rank it is on plus four. Walking from the top
means the first straight found is the highest one. The ace also counts as
the lowest card: if 2, 3, 4 and 5 are held and so is an ace, the hand is a
five-high straight (the "wheel"), the lowest straight there is.

For a straight flush the same walk is run on the cards of the flush suit
only, so a straight and a flush made of different cards don't count as a
straight flush.

## Step 3: pick the tie-break cards

Two hands of the same category are compared card by card, in a fixed order:

| Category | Tie-break cards, in order |
|---|---|
| straight flush, straight | the top card |
| four of a kind | the four's rank, then the best other card |
| full house | the three's rank, then the pair's |
| flush, high card | the five highest cards |
| three of a kind | the three's rank, then the two best other cards |
| two pair | the higher pair, the lower pair, then the best other card |
| pair | the pair's rank, then the three best other cards |

"Other cards" (kickers) come from all the cards held, not just five, so
with seven cards the best kickers are always used. Some cases that trip up
simpler evaluators:

- **Three pairs** (possible with seven cards): only the two highest pairs
  count, and the third pair's rank can be the kicker if it beats the single
  card.
- **Two threes** (also possible with seven): the higher three is the three,
  and the lower three is used as the pair of the full house.
- **A flush with six or seven cards of a suit**: only the five highest count.
- **The board plays**: when the best five are all on the board, every player
  gets the same number and the pot is split.

## Step 4: pack it into one number

The category and up to five tie-break ranks are packed as digits of a
base-13 number, category first:

```
score = category × 13⁵ + r1 × 13⁴ + r2 × 13³ + r3 × 13² + r4 × 13 + r5
      = category × 371293 + r1 × 28561 + r2 × 2197 + r3 × 169 + r4 × 13 + r5
```

Missing ranks are 0. Since every rank is below 13, a higher category always
gives a higher number, whatever the ranks; within a category the first rank
decides, then the second, and so on. That is exactly the order poker uses,
so comparing scores with `>` and `==` is the whole comparison.

Example, a pair of aces with king, seven and four:

```
category 1, ranks A K 7 4  =  1, 12, 11, 5, 2, 0
score = 1×371293 + 12×28561 + 11×2197 + 5×169 + 2×13 + 0 = 739063
```

The largest score is a royal flush, `8×371293 + 12×28561 = 3313076`, well
within the range GS1 and GS2 numbers hold exactly. `int(score / 371293)`
gives the category back, which is how `pokername` names it.

## Why this way

It is the counting method, not the "try all 21 five-card hands" method. A
seven-card hand has 21 five-card subsets; scoring each and keeping the best
is simple but does 21 times the work, and in a script that runs on a
client or an NPC server, work is time. Counting ranks and suits once and
reading the answer off the counts touches each card once and runs in a
fixed number of steps.

The tests do the opposite on purpose: they score every five-card subset with
a separate, plain evaluator and check that `pokereval` agrees on 1,200
random deals of 5, 6 and 7 cards, so a mistake in the counting shortcuts
would show up.

The script uses only what both GS1 (2.02 on) and GS2 have: arrays, loops,
`if`, numbers and short strings. No bit operations (GS2's `^` is power, not
xor, and GS1 has none), no sorting, and nothing a particular client version
might lack.
