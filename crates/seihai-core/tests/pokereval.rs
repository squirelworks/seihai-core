//! `pokereval`, the poker hand scorer in `tests/fixtures/poker/`, written in
//! GS2 and in GS1, compiled and run on Seihai's GS2 machine and checked
//! against an independent best-of-five evaluator written here. See
//! `documentation/poker/`.

use seihai_core::scripting::engine::world::{ObjId, ObjSpec, ScriptWorld};
use seihai_core::scripting::engine::{Era, Val};
use seihai_core::scripting::gs2::compile;
use seihai_core::scripting::gs2::exec::{EffectHost, Machine, Outcome, Root};

const GS2: &str = include_str!("fixtures/poker/pokereval.gs2");
const GS1: &str = include_str!("fixtures/poker/pokereval.gs1");

/// The GS1 caller: the player says the hand.
const GS1_HARNESS: &str = "if (playerchats) { tokenize #c; pokereval(); }\n";

const GS2_HARNESS: &str = r#"
function onScore(hand) {
  this.score = pokereval(hand.tokenize(" "));
  this.name = pokername(this.score);
}
"#;

fn gs2() -> (Machine, EffectHost) {
    let src = format!("{GS2}\n{GS2_HARNESS}");
    let script = compile::compile_gs2(&src).unwrap_or_else(|e| panic!("{e}"));
    let mut m = Machine::new(script);
    let mut h = EffectHost::new(Era::V217, 1);
    assert_eq!(m.run_top_level(&mut h), Outcome::Done);
    (m, h)
}

fn gs2_score(m: &mut Machine, h: &mut EffectHost, hand: &str) -> (f64, String) {
    assert_eq!(
        m.call_event("onScore", &[Val::from(hand)], h),
        Outcome::Done
    );
    assert!(h.log.is_empty(), "{hand}: {:?}", h.log);
    let score = match m.var(Root::This, "score") {
        Some(Val::Num(n)) => *n,
        other => panic!("{hand}: this.score = {other:?}"),
    };
    let name = match m.var(Root::This, "name") {
        Some(Val::Str(s)) => s.clone(),
        other => panic!("{hand}: this.name = {other:?}"),
    };
    (score, name)
}

/// GS1, compiled to GS2 bytecode and run on Seihai's GS2 machine.
fn gs1_machine() -> (Machine, EffectHost) {
    let src = format!("{GS1}\n{GS1_HARNESS}");
    let script = compile::compile_gs1(&src, Era::V217).unwrap_or_else(|e| panic!("{e}"));
    let mut m = Machine::new(script);
    let mut h = EffectHost::new(Era::V217, 1);
    assert_eq!(m.run_top_level(&mut h), Outcome::Done);
    (m, h)
}

fn gs1_machine_score(m: &mut Machine, h: &mut EffectHost, hand: &str) -> (f64, String) {
    h.player.chat = hand.into();
    assert_eq!(
        m.call_event("onPlayerChats", &[], h),
        Outcome::Done,
        "{hand}"
    );
    assert!(h.log.is_empty(), "{hand}: {:?}", h.log);
    let score = match m.var(Root::This, "pokerscore") {
        Some(Val::Num(n)) => *n,
        other => panic!("{hand}: this.pokerscore = {other:?}"),
    };
    // A plain GS1 string is a `client.gs1_` variable on the GS2 machine.
    let name = h
        .flags
        .get("client.gs1_pokername")
        .cloned()
        .unwrap_or_default();
    (score, name)
}

/// GS1 on the classic GS1 engine.
fn gs1_classic() -> (ScriptWorld, ObjId) {
    let src = format!("{GS1}\n{GS1_HARNESS}");
    let mut w = ScriptWorld::new(Era::V217);
    let id = w.add(ObjSpec::npc("blank.png", 10.0, 20.0, &src));
    assert!(
        w.obj(id).unwrap().diagnostics().is_empty(),
        "{:?}",
        w.obj(id).unwrap().diagnostics()
    );
    (w, id)
}

fn gs1_classic_score(w: &mut ScriptWorld, id: ObjId, hand: &str) -> (f64, String) {
    w.chat(hand);
    let log = w.take_log();
    assert!(log.is_empty(), "{hand}: {log:?}");
    let score = match w.obj(id).unwrap().vars.get("this.pokerscore") {
        Some(Val::Num(n)) => *n,
        other => panic!("{hand}: this.pokerscore = {other:?}"),
    };
    // The classic engine keeps GS1 strings with the flags.
    (score, w.flag("pokername").unwrap_or_default().to_string())
}

// ---- The reference: every five-card subset, each scored directly. ----

fn card(c: &str) -> (usize, usize) {
    let b = c.as_bytes();
    let r = b"23456789TJQKA"
        .iter()
        .position(|&x| x == b[0].to_ascii_uppercase())
        .unwrap();
    let s = b"CDHS"
        .iter()
        .position(|&x| x == b[1].to_ascii_uppercase())
        .unwrap();
    (r, s)
}

fn pack(cat: usize, ranks: &[usize]) -> f64 {
    let mut v = cat;
    for i in 0..5 {
        v = v * 13 + ranks.get(i).copied().unwrap_or(0);
    }
    v as f64
}

fn five(cards: &[(usize, usize)]) -> f64 {
    let mut counts = [0usize; 13];
    for &(r, _) in cards {
        counts[r] += 1;
    }
    // (count, rank): the biggest group first, then the higher rank.
    let mut groups: Vec<(usize, usize)> = (0..13)
        .filter(|&r| counts[r] > 0)
        .map(|r| (counts[r], r))
        .collect();
    groups.sort_by(|a, b| b.cmp(a));
    let ranks: Vec<usize> = groups.iter().map(|g| g.1).collect();
    let flush = cards.iter().all(|c| c.1 == cards[0].1);
    let straight = if groups.len() != 5 {
        None
    } else if ranks[0] - ranks[4] == 4 {
        Some(ranks[0])
    } else if ranks == [12, 3, 2, 1, 0] {
        Some(3)
    } else {
        None
    };
    let shape: Vec<usize> = groups.iter().map(|g| g.0).collect();
    match (straight, flush, shape.as_slice()) {
        (Some(h), true, _) => pack(8, &[h]),
        (_, _, [4, 1]) => pack(7, &ranks),
        (_, _, [3, 2]) => pack(6, &ranks),
        (None, true, _) => pack(5, &ranks),
        (Some(h), false, _) => pack(4, &[h]),
        (_, _, [3, 1, 1]) => pack(3, &ranks),
        (_, _, [2, 2, 1]) => pack(2, &ranks),
        (_, _, [2, 1, 1, 1]) => pack(1, &ranks),
        _ => pack(0, &ranks),
    }
}

fn reference(hand: &str) -> f64 {
    let cards: Vec<_> = hand.split(' ').map(card).collect();
    let n = cards.len();
    let mut best = -1.0f64;
    for mask in 0u32..(1 << n) {
        if mask.count_ones() == 5 {
            let pick: Vec<_> = (0..n)
                .filter(|i| mask & (1 << i) != 0)
                .map(|i| cards[i])
                .collect();
            best = best.max(five(&pick));
        }
    }
    best
}

/// Deterministic deals: `n` distinct cards from a shuffled deck.
fn deals(count: usize, n: usize) -> Vec<String> {
    let mut seed = 0x2545_f491_4f6c_dd1du64 ^ n as u64;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    (0..count)
        .map(|_| {
            let mut deck: Vec<String> = "cdhs"
                .chars()
                .flat_map(|s| "23456789TJQKA".chars().map(move |r| format!("{r}{s}")))
                .collect();
            for i in (1..52).rev() {
                deck.swap(i, (next() % (i as u64 + 1)) as usize);
            }
            deck[..n].join(" ")
        })
        .collect()
}

/// Hands that reach every category, and the awkward cases.
const NAMED: &[(&str, &str)] = &[
    ("As Ks Qs Js Ts 2d 3c", "straight flush"),
    ("Ah 2h 3h 4h 5h Kd Kc", "straight flush"),
    ("9c 9d 9h 9s 2c 3d Ah", "four of a kind"),
    ("Kc Kd Kh 7s 7c 2d 3h", "full house"),
    ("Kc Kd Kh 7s 7c 7d 3h", "full house"),
    ("2h 7h 9h Jh Ah Kd Qd", "flush"),
    ("Th Jc Qd Ks Ah 2c 3c", "straight"),
    ("Ah 2c 3d 4s 5h Kd Qd", "straight"),
    ("5c 5d 5h Ks 2c 8d 9h", "three of a kind"),
    ("Jc Jd 4h 4s 9c 9d 2h", "two pair"),
    ("Qc Qd 2h 7s 9c Td 3h", "pair"),
    ("Ac Jd 9h 7s 4c 3d 2h", "high card"),
    ("as ks qs js ts", "straight flush"),
    ("2c 3c 4c 5c 7d", "high card"),
];

/// The order poker puts hands in: each pair is (better, worse) or a tie.
const ORDER: &[(&str, &str, bool)] = &[
    // A higher kicker wins.
    ("Ac Ad Kh 7s 4c", "Ac Ad Qh 7s 4c", false),
    // The wheel is the lowest straight.
    ("2c 3d 4h 5s 6c", "Ac 2d 3h 4s 5c", false),
    // A flush's fifth card counts.
    ("Ah Kh 9h 7h 3h", "Ah Kh 9h 7h 2h", false),
    // Full houses compare the three first.
    ("Kc Kd Kh 2s 2c", "Qc Qd Qh As Ac", false),
    // Two pair from three: the lowest pair's rank is only a kicker.
    ("Jc Jd 9h 9s 4c 4d 2h", "Jc Jd 9h 9s 3c 2d 2h", false),
    ("Jc Jd 9h 9s 8c 4d 4h", "Jc Jd 9h 9s 4c 4d 2h", false),
    // A royal flush on the board: both players split.
    ("Ts Js Qs Ks As 2c 3d", "Ts Js Qs Ks As 4h 5d", true),
];

#[test]
fn gs2_names_every_category() {
    let (mut m, mut h) = gs2();
    for (hand, name) in NAMED {
        let (score, got) = gs2_score(&mut m, &mut h, hand);
        assert_eq!(got, *name, "{hand}");
        assert_eq!(score, reference(hand), "{hand}");
    }
}

#[test]
fn gs2_orders_hands_like_poker() {
    let (mut m, mut h) = gs2();
    for (a, b, tie) in ORDER {
        let sa = gs2_score(&mut m, &mut h, a).0;
        let sb = gs2_score(&mut m, &mut h, b).0;
        if *tie {
            assert_eq!(sa, sb, "{a} vs {b}");
        } else {
            assert!(sa > sb, "{a} ({sa}) should beat {b} ({sb})");
        }
    }
    assert_eq!(gs2_score(&mut m, &mut h, "Xs Kd Qh Jc Tc").0, -1.0);
}

#[test]
fn gs2_matches_the_reference_on_random_deals() {
    let (mut m, mut h) = gs2();
    for n in [5, 6, 7] {
        for hand in deals(400, n) {
            assert_eq!(
                gs2_score(&mut m, &mut h, &hand).0,
                reference(&hand),
                "{hand}"
            );
        }
    }
}

#[test]
fn gs1_on_the_gs2_machine_names_every_category_and_matches_the_reference() {
    let (mut m, mut h) = gs1_machine();
    for (hand, name) in NAMED {
        let (score, got) = gs1_machine_score(&mut m, &mut h, hand);
        assert_eq!(got, *name, "{hand}");
        assert_eq!(score, reference(hand), "{hand}");
    }
    assert_eq!(gs1_machine_score(&mut m, &mut h, "Xs Kd Qh Jc Tc").0, -1.0);
    for n in [5, 6, 7] {
        for hand in deals(400, n) {
            assert_eq!(
                gs1_machine_score(&mut m, &mut h, &hand).0,
                reference(&hand),
                "{hand}"
            );
        }
    }
}

#[test]
fn gs1_on_the_classic_engine_names_every_category_and_matches_the_reference() {
    let (mut w, id) = gs1_classic();
    for (hand, name) in NAMED {
        let (score, got) = gs1_classic_score(&mut w, id, hand);
        assert_eq!(got, *name, "{hand}");
        assert_eq!(score, reference(hand), "{hand}");
    }
    assert_eq!(gs1_classic_score(&mut w, id, "Xs Kd Qh Jc Tc").0, -1.0);
    for n in [5, 6, 7] {
        for hand in deals(400, n) {
            assert_eq!(
                gs1_classic_score(&mut w, id, &hand).0,
                reference(&hand),
                "{hand}"
            );
        }
    }
}

#[test]
fn gs1_and_gs2_agree_on_the_order_of_hands() {
    let (mut m, mut h) = gs1_machine();
    for (a, b, tie) in ORDER {
        let sa = gs1_machine_score(&mut m, &mut h, a).0;
        let sb = gs1_machine_score(&mut m, &mut h, b).0;
        assert_eq!(sa == sb, *tie, "{a} vs {b}");
        assert!(*tie || sa > sb, "{a} ({sa}) should beat {b} ({sb})");
    }
}

/// The worked scores in `documentation/poker/pokereval-gs2.md`.
#[test]
fn the_documented_scores() {
    let (mut m, mut h) = gs2();
    for (hand, score) in [
        ("As Ks Qs Js Ts", 3_313_076.0),
        ("Ac Ad Kh 7s 4c", 739_063.0),
        ("Ac 2d 3h 4s 5c", 1_570_855.0),
        ("7c 5d 4h 3s 2c", 149_747.0),
    ] {
        assert_eq!(gs2_score(&mut m, &mut h, hand).0, score, "{hand}");
    }
}
