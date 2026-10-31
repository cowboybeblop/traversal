//! Harnais de test d'Obsidienne sous WASI : duels contre les niveaux de référence,
//! mirror A/B (cap en nœuds ou parité fuel), sanity, vitesse, chemin de production.

use std::time::{Duration, Instant};

use traversal_bot::core::bot::{choose_move, exit_costs, Level, Tuning};
use traversal_bot::core::config::{Kind, SQUARES};
use traversal_bot::core::movegen::{legal_moves, Move, MoveKind};
use traversal_bot::core::rules::{apply_move, new_game};
use traversal_bot::core::state::State;
use traversal_bot::core::wire::{encode, BUFFER_SIZE};
use traversal_bot::core::Rng;

use obsidienne::engine::{Engine, Flags, MAX_TURNS};
use obsidienne::eval::{eval, EvalCfg};

/// Fuel alloué par coup en simulation (l'arène donne 2 Md, on en garde 3/16 en réserve).
const FUEL_BUDGET: u64 = 2_000_000_000 / 16 * 13;
/// Fuel consommé par nœud du moteur de référence (étalonnage d'origine).
const REF_FUEL_PER_NODE: u64 = 23_400;
const BASE_SEED: u64 = 0xDEAD_BEEF;

static mut FUEL_LEFT: u64 = 0;
static mut FUEL_STEP: u64 = REF_FUEL_PER_NODE;
static mut CAP_LEFT: u64 = 0;
static mut FUEL_ON: bool = false;
static mut MAXFAN: usize = 0;

fn set_fuel(step: u64) {
    unsafe {
        *std::ptr::addr_of_mut!(FUEL_STEP) = step;
        *std::ptr::addr_of_mut!(FUEL_LEFT) = FUEL_BUDGET;
        *std::ptr::addr_of_mut!(FUEL_ON) = true;
    }
}

fn set_cap(cap: u64) {
    unsafe {
        *std::ptr::addr_of_mut!(CAP_LEFT) = cap;
        *std::ptr::addr_of_mut!(FUEL_ON) = false;
    }
}

/// Arrêt de recherche du harnais : appelé tous les 64 nœuds, charge 64 unités.
fn harness_stop() -> bool {
    unsafe {
        if *std::ptr::addr_of!(FUEL_ON) {
            let step = 64 * *std::ptr::addr_of!(FUEL_STEP);
            let left = std::ptr::addr_of_mut!(FUEL_LEFT);
            if *left > step {
                *left -= step;
                false
            } else {
                *left = 0;
                true
            }
        } else {
            let cap = std::ptr::addr_of_mut!(CAP_LEFT);
            if *cap > 64 {
                *cap -= 64;
                false
            } else {
                *cap = 0;
                true
            }
        }
    }
}

fn never_stop() -> bool {
    false
}

fn move_seed(seed: u64, ply: u16) -> u64 {
    Rng::new(seed ^ (ply as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)).next_u64()
}

// ---------------------------------------------------------------- specs ----

#[derive(Clone, Copy)]
struct Spec {
    cfg: EvalCfg,
    flags: Flags,
    /// fuel simulé par nœud (étalonné sur ce moteur : hang/blocked/ec).
    weight: u64,
}

/// `default` = évaluation de base, aucun terme optionnel ; sinon clés `k=v` séparées
/// par des virgules. Les poids fuel sont mesurés sur ce build (n=8, prodpath).
fn parse_spec(s: &str) -> Spec {
    let mut cfg = EvalCfg::default();
    let mut flags = Flags::default();
    if s != "default" {
        for kv in s.split(',') {
            let mut it = kv.splitn(2, '=');
            let k = it.next().unwrap_or("").trim();
            let val: i32 = it.next().and_then(|v| v.trim().parse().ok()).unwrap_or(1);
            match k {
                "score" => cfg.score = val,
                "base" => cfg.base = val,
                "prog_span" => cfg.prog_span = val,
                "prog_a" => cfg.prog_a = val,
                "prog_b" => cfg.prog_b = val,
                "sent_span" => cfg.sent_span = val,
                "sent_w" => cfg.sent_w = val,
                "race" => cfg.race = val,
                "race_h" => cfg.race_h = val,
                "tempo" => cfg.tempo = val,
                "delta" => cfg.delta = val,
                "futility" => cfg.futility = val,
                "support" => flags.support_w = val,
                "hang" => flags.hang_w = val,
                "blocked" => flags.blocked = val,
                "asp" => flags.aspiration = val != 0,
                "nmp" => flags.nmp = val != 0,
                "lmr" => flags.lmr = val != 0,
                "fut" => flags.fut = val != 0,
                "lmp" => flags.lmp = val != 0,
                "tt" => flags.ttx = val != 0,
                "ec" => flags.ecache = val != 0,
                "rs" => flags.rs = val != 0,
                "rn" => flags.rn = val != 0,
                "hb" => flags.hb = val != 0,
                "cm" => flags.cm = val != 0,
                "rdif" => flags.rdif = val != 0,
                "ssafe" => flags.ssafe = val != 0,
                "ch" => flags.ch = val != 0,
                "seet" => flags.seet = val != 0,
                "zc" => flags.zc = val != 0,
                "mat" => flags.mat = val != 0,
                "k3" => flags.k3 = val != 0,
                "iir" => flags.iir = val != 0,
                "ptab" => { flags.ptab = val != 0; },
                "hangtab" => flags.hangtab = val != 0,
                "hangd" => flags.hangd = val != 0,
                "ptabt" => {
                    // table apprise par distillation (tools/ptab_tune.py, corpus 6 114 pos.)
                    flags.ptab = val != 0;
                    if val != 0 {
                        cfg.ptab[0] = [91, 84, 73, 72, 65, 56, 49, 42, 35, 28, 21, 14, 7, 0, 0, 0];
                        cfg.ptab[1] = [130, 140, 106, 96, 90, 90, 71, 60, 50, 40, 30, 20, 10, 0, 0, 0];
                        cfg.ptab[2] = [169, 238, 157, 128, 109, 90, 91, 78, 65, 52, 39, 26, 13, 0, 0, 0];
                    }
                },
                _ => {}
            }
        }
    }
    let mut w: i64 = REF_FUEL_PER_NODE as i64;
    if flags.hang_w > 0 {
        w += 651;
    }
    if flags.blocked > 0 {
        w += 61_660;
    }
    if flags.support_w > 0 {
        w += 500;
    }
    if flags.ecache {
        w += if flags.blocked > 0 { -21_317 } else if flags.hang_w > 0 { -1_112 } else { 915 };
    }
    Spec { cfg, flags, weight: w.max(1000) as u64 }
}

// -------------------------------------------------------------- players ----

struct ObsPlayer {
    engine: Engine,
    spec: Spec,
    fuel_on: bool,
    cap: u64,
    depth: u8,
    nodes: u64,
    moves: u64,
    last_score: i32,
}

impl ObsPlayer {
    fn new(h: [u8; SQUARES], spec: Spec, fuel_on: bool, cap: u64, depth: u8) -> ObsPlayer {
        ObsPlayer { engine: Engine::new(h, spec.cfg, spec.flags), spec, fuel_on, cap, depth, nodes: 0, moves: 0, last_score: 0 }
    }

    fn choose(&mut self, s: &State) -> Move {
        let moves = legal_moves(s);
        unsafe {
            let mf = std::ptr::addr_of_mut!(MAXFAN);
            if moves.len() > *mf {
                *mf = moves.len();
            }
        }
        if moves.is_empty() {
            return Move::PASS;
        }
        // Raccourci production : première sortie = Sentinelle = victoire immédiate (R8.2).
        if s.finished == 0 {
            if let Some(m) = moves
                .iter()
                .find(|m| m.kind == MoveKind::Exit && s.piece(m.piece).kind == Kind::Sentinel)
            {
                return *m;
            }
        }
        if self.fuel_on {
            set_fuel(self.spec.weight);
        } else {
            set_cap(self.cap);
        }
        let (mv, score, _d) = self.engine.search(s, self.depth, harness_stop);
        self.last_score = score;
        self.nodes += self.engine.nodes;
        self.moves += 1;
        moves
            .iter()
            .find(|m| m.piece == mv.piece && m.kind == mv.kind && m.to == mv.to)
            .copied()
            .unwrap_or(moves[0])
    }
}

/// Bot de référence (niveaux du site : Galet=1 … Mont Blanc=5).
fn ref_level(lvl: u8) -> Level {
    let (depth, beam, noise, insight) = match lvl {
        1 => (1u8, 999usize, 90.0f64, 0u8),
        2 => (1, 999, 25.0, 0),
        3 => (2, 12, 8.0, 1),
        4 => (3, 9, 2.0, 2),
        _ => (4, 7, 0.0, 3),
    };
    Level { depth, beam, noise, budget: Duration::MAX, insight, tuning: Tuning::default(), stop: Some(never_stop) }
}

struct RefPlayer {
    level: Level,
    seed: u64,
}

impl RefPlayer {
    fn new(lvl: u8, seed: u64) -> RefPlayer {
        RefPlayer { level: ref_level(lvl), seed }
    }

    fn choose(&self, s: &State) -> Move {
        let mut rng = Rng::new(move_seed(self.seed, s.ply));
        choose_move(s, self.level, &mut rng).mv
    }
}

// ----------------------------------------------------------------- jeu ----

fn play(h: [u8; SQUARES], mut red: impl FnMut(&State) -> Move, mut blue: impl FnMut(&State) -> Move) -> (Option<u8>, u16) {
    let mut s = new_game(h);
    while !s.over {
        let m = if s.to_move == 1 { red(&s) } else { blue(&s) };
        s = apply_move(&s, &m);
    }
    (s.winner, s.ply)
}

/// Elo d'un score V-D-N (nulles = 0,5), échelle 400·log10.
fn elo(v: usize, d: usize, n: usize) -> i32 {
    let t = (v + d + n) as f64;
    if t == 0.0 {
        return 0;
    }
    let p = (v as f64 + 0.5 * n as f64) / t;
    if p >= 1.0 {
        10_000
    } else if p <= 0.0 {
        -10_000
    } else {
        (-400.0 * (1.0 / p - 1.0).log10()).round() as i32
    }
}

/// Parse `sdk/boards.json` à la main : 200 plateaux de 100 hauteurs.
fn load_boards() -> Vec<[u8; SQUARES]> {
    let txt = std::fs::read_to_string("sdk/boards.json").expect("boards.json illisible");
    let mut boards = Vec::new();
    let mut cur: Option<Vec<u8>> = None;
    let mut acc: u32 = 0;
    let mut has = false;
    for &c in txt.as_bytes() {
        match c {
            b'[' => {
                if cur.is_none() {
                    cur = Some(Vec::with_capacity(SQUARES));
                }
            }
            b'0'..=b'9' => {
                acc = acc.saturating_mul(10) + (c - b'0') as u32;
                has = true;
            }
            _ => {
                if has {
                    if let Some(v) = cur.as_mut() {
                        v.push(acc as u8);
                    }
                    acc = 0;
                    has = false;
                }
                if c == b']' {
                    if let Some(v) = cur.take() {
                        let arr: [u8; SQUARES] =
                            v.try_into().expect("plateau de taille inattendue");
                        boards.push(arr);
                    }
                }
            }
        }
    }
    assert!(!boards.is_empty(), "aucun plateau chargé");
    boards
}

// ------------------------------------------------------------ scénarios ----

/// Mirror A/B : deux Obsidienne de specs différentes. Chaque carte est jouée deux
/// fois à couleurs inversées (i/2 = carte, i%2 = couleur de A) : insensible au biais
/// de couleur propre à certaines cartes du plateau officiel.
fn mirror_run(n: usize, cap: u64, an: &str, sa: Spec, bn: &str, sb: Spec, off: usize, fuel: bool) {
    let boards = load_boards();
    let t0 = Instant::now();
    let (mut v, mut d, mut dr) = (0usize, 0usize, 0usize);
    // Pentanomial sur paires inversées (point de vue de A) : LL, LW/TT, WW/DD, WT, WW.
    let mut penta = [0usize; 5];
    let mut penta_nd = 0usize;
    let mut half: Vec<Option<bool>> = Vec::with_capacity(n / 2);
    let depth = if fuel { MAX_TURNS } else { 40 };
    for i in 0..n {
        let h = boards[(off + i / 2) % boards.len()];
        let mut pa = ObsPlayer::new(h, sa, fuel, cap, depth);
        let mut pb = ObsPlayer::new(h, sb, fuel, cap, depth);
        let (w, _) = if i % 2 == 0 {
            play(h, |s| pa.choose(s), |s| pb.choose(s))
        } else {
            play(h, |s| pb.choose(s), |s| pa.choose(s))
        };
        let a_won = w.map(|x| (x == 1) == (i % 2 == 0));
        match a_won {
            Some(true) => v += 1,
            Some(false) => d += 1,
            None => dr += 1,
        }
        if i % 2 == 0 {
            half.push(a_won);
        } else {
            match (half.pop().flatten(), a_won) {
                (Some(first), Some(second)) => {
                    penta[match (first, second) {
                        (false, false) => 0,
                        (false, true) | (true, false) => 2,
                        (true, true) => 4,
                    }] += 1;
                }
                _ => penta_nd += 1,
            }
        }
    }
    let tag = if fuel { "fmirror" } else { "mirror" };
    let cap_s = if fuel { format!("fuel{}", FUEL_BUDGET) } else { format!("cap {}", cap) };
    // Elo pentanomial : score attendu = (0.25*p0 + 0.5*p2 + p4) / paires.
    let pairs = penta[0] + penta[1] + penta[2] + penta[3] + penta[4];
    let penta_elo = if pairs > 0 {
        let sc = (penta[0] as f64 * 0.0 + penta[2] as f64 * 0.5 + penta[4] as f64 * 1.0
            + (penta[1] + penta[3]) as f64 * 0.25) / pairs as f64;
        400.0 * (sc / (1.0 - sc)).log10()
    } else { 0.0 };
    println!(
        "{} {} [{}+{}] {} vs {}: {}V-{}D-{}N ({} Elo pour {}) | penta LL/LT/TT/WT/WW nd: {}/{}/{}/{}/{} +{} [{:.0} s] pentaElo {:+.0}",
        tag, cap_s, off, off + n, an, bn, v, d, dr, elo(v, d, dr), an,
        penta[0], penta[1], penta[2], penta[3], penta[4], penta_nd, t0.elapsed().as_secs_f64(), penta_elo
    );
}

/// Distillation : dump (position → score de recherche à budget fixe côté trait).
/// Contrairement au selfdump (résultat de partie), le label est l'avis de la
/// recherche profonde — cible stable pour la régression des constantes d'eval.
fn distill(n: usize, off: usize, spec: Spec, cap: u64, depth: u8) {
    let boards = load_boards();
    for i in 0..n {
        let h = boards[(off + i) % boards.len()];
        let mut p = ObsPlayer::new(h, spec, false, cap, depth);
        let mut s = new_game(h);
        while !s.over {
            let m = p.choose(&s);
            // score après recherche, côté trait courant (= joueur qui vient de chercher)
            let z = p.last_score;
            let mut line = String::new();
            line.push_str(&format!("{} {} {} |", s.to_move, s.scores[0] as u32, s.scores[1] as u32));
            for j in 0..SQUARES {
                line.push(' ');
                line.push_str(&h[j].to_string());
            }
            line.push_str(" |");
            for pc in s.pieces() {
                line.push_str(&format!(" {}:{}:{}", pc.owner, pc.kind.strength(), pc.sq));
            }
            println!("{} | {}", line, z);
            s = apply_move(&s, &m);
        }
    }
}

/// SPRT (test séquentiel de probabilité ratio, style Fishtest) sur miroir A/B.
/// Hypothèses elo0 < vrai < elo1 ; s'arrête dès acceptation/rejet. Score attendu
/// via logistic(E/400). Likelihood-ratio sur loi trinomiale (V/D/N, nulle=0.5).
fn sprt_mirror(
    n_max: usize,
    cap: u64,
    an: &str,
    sa: Spec,
    bn: &str,
    sb: Spec,
    off: usize,
    fuel: bool,
    elo0: f64,
    elo1: f64,
) {
    let boards = load_boards();
    let t0 = Instant::now();
    let depth = if fuel { MAX_TURNS } else { 40 };
    let (mut v, mut d, mut dr) = (0usize, 0usize, 0usize);
    // proba attendue de victoire sous H (nulle comptée 0.5)
    let pwin = |elo: f64| 1.0 / (1.0 + (-elo / 400.0).exp());
    let p0 = pwin(elo0);
    let p1 = pwin(elo1);
    let mut llr = 0.0f64;
    let mut accepted: Option<bool> = None;
    let mut i = 0usize;
    while i < n_max {
        let h = boards[(off + i / 2) % boards.len()];
        let mut pa = ObsPlayer::new(h, sa, fuel, cap, depth);
        let mut pb = ObsPlayer::new(h, sb, fuel, cap, depth);
        let (w, _) = if i % 2 == 0 {
            play(h, |s| pa.choose(s), |s| pb.choose(s))
        } else {
            play(h, |s| pb.choose(s), |s| pa.choose(s))
        };
        match w {
            Some(x) if (x == 1) == (i % 2 == 0) => {
                v += 1;
                // V : P(V) = p, P(D) = 1-p
                llr += (p1 / p0).ln() + ((1.0 - p1) / (1.0 - p0)).ln();
            }
            Some(_) => {
                d += 1;
                llr += ((1.0 - p1) / (1.0 - p0)).ln() * 2.0;
            }
            None => {
                dr += 1;
            }
        }
        i += 1;
        // seuils : alpha=beta=0.05 → L > 2.944 accepte H1, < -2.944 accepte H0
        if llr >= 2.944 {
            accepted = Some(true);
            break;
        }
        if llr <= -2.944 {
            accepted = Some(false);
            break;
        }
    }
    let tag = if fuel { "fsprt" } else { "sprt" };
    let cap_s = if fuel { format!("fuel{}", FUEL_BUDGET) } else { format!("cap {}", cap) };
    match accepted {
        Some(true) => println!(
            "{} {} [{}+{}] {} vs {}: H1 ACCEPTÉE (gain) — {}V-{}D-{}N llr={:.2} en {} p. [{:.0} s]",
            tag, cap_s, off, off + i, an, bn, v, d, dr, llr, i, t0.elapsed().as_secs_f64()
        ),
        Some(false) => println!(
            "{} {} [{}+{}] {} vs {}: H0 ACCEPTÉE (pas de gain) — {}V-{}D-{}N llr={:.2} en {} p. [{:.0} s]",
            tag, cap_s, off, off + i, an, bn, v, d, dr, llr, i, t0.elapsed().as_secs_f64()
        ),
        None => println!(
            "{} {} [{}+{}] {} vs {}: INCONCLUSIF — {}V-{}D-{}N llr={:.2} en {} p. [{:.0} s]",
            tag, cap_s, off, off + i, an, bn, v, d, dr, llr, i, t0.elapsed().as_secs_f64()
        ),
    }
}

/// Self-play avec dump de positions labellisées pour le tuning Texel.
/// Format par ligne : to_move ply scores finished | hauteurs | pièces | résultat W/D/L côté trait.
/// Le résultat est écrit après coup : W=1, D=0.5, L=0 du point de vue du joueur au trait.
fn selfdump(n: usize, off: usize, spec: Spec) {
    let boards = load_boards();
    let depth = MAX_TURNS;
    for i in 0..n {
        let h = boards[(off + i) % boards.len()];
        // Une partie sur deux contre un bot de référence (diversité du corpus).
        let vs_ref = i % 2 == 1;
        let lvl = 3u8 + (i % 2) as u8;
        let seed = 0x5A17u64 + i as u64;
        let mut p = ObsPlayer::new(h, spec, true, 0, depth);
        let mut r = RefPlayer::new(if vs_ref { lvl } else { 3 }, seed);
        let mut s = new_game(h);
        let mut hist: Vec<State> = Vec::new();
        while !s.over {
            hist.push(s.clone());
            let m = if vs_ref && s.to_move == 2 { r.choose(&s) } else { p.choose(&s) };
            s = apply_move(&s, &m);
        }
        // résultat du point de vue du trait de chaque position
        for st in &hist {
            let z = match s.winner {
                Some(w) if w == st.to_move => 1.0f64,
                Some(_) => 0.0,
                None => 0.5,
            };
            let mut line = String::new();
            line.push_str(&format!("{} {} {} {} |", st.to_move, st.ply, st.scores[0] as u32, st.scores[1] as u32));
            for j in 0..SQUARES {
                line.push(' ');
                line.push_str(&h[j].to_string());
            }
            line.push_str(" |");
            for pc in st.pieces() {
                line.push_str(&format!(" {}:{}:{}", pc.owner, pc.kind.strength(), pc.sq));
            }
            println!("{} | {}", line, z);
        }
    }
}

/// Duel Obsidienne (spec) contre bot de référence niveau `lvl`.
fn duel_run(lvl: u8, n: usize, cap: u64, depth: u8, off: usize, spec: Spec, fuel: bool) {
    let boards = load_boards();
    let t0 = Instant::now();
    let (mut v, mut d, mut dr) = (0usize, 0usize, 0usize);
    let (mut nodes, mut moves) = (0u64, 0u64);
    for i in 0..n {
        let h = boards[(off + i / 2) % boards.len()];
        let seed = BASE_SEED + (off + i) as u64;
        let mut po = ObsPlayer::new(h, spec, fuel, cap, if fuel { MAX_TURNS } else { depth });
        let mut pr = RefPlayer::new(lvl, seed);
        let (w, _) = if i % 2 == 0 {
            play(h, |s| po.choose(s), |s| pr.choose(s))
        } else {
            play(h, |s| pr.choose(s), |s| po.choose(s))
        };
        nodes += po.nodes;
        moves += po.moves;
        match w {
            Some(x) if (x == 1) == (i % 2 == 0) => v += 1,
            Some(_) => d += 1,
            None => dr += 1,
        }
    }
    let tag = if fuel { "duelf" } else { "duel" };
    let cap_s = if fuel { format!("fuel{}", FUEL_BUDGET) } else { format!("cap{}", cap) };
    let npc = if moves > 0 { nodes / moves } else { 0 };
    println!(
        "{} L{} {} [{}+{}]: {}V-{}D-{}N ({} Elo), ~{} nœuds/coup [{:.0} s]",
        tag, lvl, cap_s, off, off + n, v, d, dr, elo(v, d, dr), npc, t0.elapsed().as_secs_f64()
    );
}

/// Sanity : Obsidienne default doit battre L1/L3/L5 ; rejeu déterministe vérifié.
/// Cartes jouées deux fois à couleurs inversées (i/2 = carte, i%2 = couleur d'Obs).
fn sanity(n: usize) {
    let boards = load_boards();
    let default = parse_spec("default");
    for lvl in [1u8, 3, 5] {
        let mut wins = 0;
        for i in 0..n {
            let h = boards[(i / 2) % boards.len()];
            let seed = 0x5A17u64 + i as u64;
            let first = {
                let mut po = ObsPlayer::new(h, default, false, 20_000, 40);
                let mut pr = RefPlayer::new(lvl, seed);
                if i % 2 == 0 {
                    play(h, |s| po.choose(s), |s| pr.choose(s))
                } else {
                    play(h, |s| pr.choose(s), |s| po.choose(s))
                }
            };
            let replay = {
                let mut po = ObsPlayer::new(h, default, false, 20_000, 40);
                let mut pr = RefPlayer::new(lvl, seed);
                if i % 2 == 0 {
                    play(h, |s| po.choose(s), |s| pr.choose(s))
                } else {
                    play(h, |s| pr.choose(s), |s| po.choose(s))
                }
            };
            assert_eq!(first, replay, "rejeu non déterministe (L{}, partie {})", lvl, i);
            if let (Some(x), _) = first {
                if (x == 1) == (i % 2 == 0) {
                    wins += 1;
                }
            }
        }
        println!("sanity L{}: {}/{}", lvl, wins, n);
    }
    let fan = unsafe { *std::ptr::addr_of!(MAXFAN) };
    println!("SANITY OK (fanout max {})", fan);
}

/// Vitesse : recherche à profondeur fixe depuis l'ouverture, nœuds/s.
fn speed(n: usize, depth: u8, spec: Spec) {
    let boards = load_boards();
    let mut total_nodes = 0u64;
    let t0 = Instant::now();
    for b in 0..n {
        let h = boards[b % boards.len()];
        let mut eng = Engine::new(h, spec.cfg, spec.flags);
        let s = new_game(h);
        set_cap(u64::MAX);
        let tb = Instant::now();
        eng.search(&s, depth, harness_stop);
        total_nodes += eng.nodes;
        println!("  carte {}: {} nœuds, {:.2} s", b, eng.nodes, tb.elapsed().as_secs_f64());
    }
    let dt = t0.elapsed().as_secs_f64();
    let nps = if dt > 0.0 { (total_nodes as f64 / dt) as u64 } else { 0 };
    println!("speed: {} nœuds/s ({:.2} s)", nps, dt);
}

/// Une partie complète par le chemin de production (encode → tampon → traversal_move).
fn play_prod(h: [u8; SQUARES], seed: u64) -> (Option<u8>, u16) {
    let mut s = new_game(h);
    while !s.over {
        let moves = legal_moves(&s);
        let bytes = encode(&s, &moves, move_seed(seed, s.ply), 2_000_000_000);
        assert!(bytes.len() <= BUFFER_SIZE, "dépassement du tampon");
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), obsidienne::traversal_buffer(), bytes.len());
        }
        let idx = obsidienne::traversal_move(bytes.len() as i32);
        assert!(idx >= 0 && (idx as usize) < moves.len(), "indice illégal: {}", idx);
        s = apply_move(&s, &moves[idx as usize]);
    }
    (s.winner, s.ply)
}

fn prodpath(n: usize) {
    let boards = load_boards();
    let t0 = Instant::now();
    for b in 0..n {
        let h = boards[b % boards.len()];
        let seed = BASE_SEED + b as u64;
        let r1 = play_prod(h, seed);
        let r2 = play_prod(h, seed);
        assert_eq!(r1, r2, "prodpath non déterministe (carte {})", b);
        println!("  carte {}: {:?}", b, r1);
    }
    println!("PRODPATH OK [{:.0} s]", t0.elapsed().as_secs_f64());
}

/// Alpha-beta brut de référence (sans TT, sans élagages) pour le scénario `check`.
fn plain(s: &State, d: i32, mut a: i32, b: i32, cfg: &EvalCfg, costs: &[[u16; SQUARES]; 2], flags: &Flags) -> i32 {
    if s.over || d <= 0 {
        return eval(s, s.to_move, cfg, costs, flags);
    }
    let moves = legal_moves(s);
    let mut best = i32::MIN + 1;
    for m in &moves {
        let child = apply_move(s, m);
        let sc = -plain(&child, d - 1, -b, -a, cfg, costs, flags);
        if sc > best {
            best = sc;
        }
        if best > a {
            a = best;
        }
        if a >= b {
            break;
        }
    }
    if moves.is_empty() {
        eval(s, s.to_move, cfg, costs, flags)
    } else {
        best
    }
}

/// Concordance moteur vs alpha-beta brut (divergences attendues : quiescence, élagages).
fn check(n: usize, spec: Spec) {
    let boards = load_boards();
    let mut agree = 0;
    let mut total = 0;
    for b in 0..n {
        let h = boards[b % boards.len()];
        let costs = [exit_costs(&h, 1), exit_costs(&h, 2)];
        let mut eng = Engine::new(h, spec.cfg, spec.flags);
        let mut s = new_game(h);
        let mut steps = 0;
        while !s.over && steps < 40 {
            set_cap(4000);
            let (mv, score, _) = eng.search(&s, 3, harness_stop);
            let ref_val = plain(&s, 2, i32::MIN + 1, i32::MAX - 1, &spec.cfg, &costs, &spec.flags);
            if score == ref_val {
                agree += 1;
            }
            total += 1;
            s = apply_move(&s, &mv);
            steps += 1;
        }
    }
    println!("check: {}/{} accords", agree, total);
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let arg = |i: usize, d: usize| a.get(i + 1).and_then(|x| x.parse().ok()).unwrap_or(d);
    let arg_u8 = |i: usize, d: u8| a.get(i + 1).and_then(|x| x.parse().ok()).unwrap_or(d);
    let arg_s = |i: usize, d: &str| a.get(i + 1).cloned().unwrap_or_else(|| d.to_string());
    let cmd = arg_s(0, "sanity");
    match cmd.as_str() {
        "sanity" => sanity(arg(1, 4)),
        "selfdump" => selfdump(arg(1, 10), arg(2, 0), parse_spec(&arg_s(3, "default"))),
        "distill" => distill(arg(1, 10), arg(2, 0), parse_spec(&arg_s(3, "default")), arg(4, 40_000) as u64, arg_u8(5, 12)),
        "sprt" => sprt_mirror(
            arg(1, 200), arg(2, 15_000) as u64, &arg_s(3, "default"), parse_spec(&arg_s(3, "default")),
            &arg_s(4, "default"), parse_spec(&arg_s(4, "default")), arg(5, 0), false,
            arg(6, 0) as f64 / 100.0, arg(7, 50) as f64 / 100.0,
        ),
        "fsprt" => sprt_mirror(
            arg(1, 200), 0, &arg_s(2, "default"), parse_spec(&arg_s(2, "default")),
            &arg_s(3, "default"), parse_spec(&arg_s(3, "default")), arg(4, 0), true,
            arg(5, 0) as f64 / 100.0, arg(6, 50) as f64 / 100.0,
        ),
        "speed" => speed(arg(1, 4), arg_u8(2, 6), parse_spec(&arg_s(3, "default"))),
        "duel" => duel_run(
            arg_u8(1, 5),
            arg(2, 6),
            arg(3, 30_000) as u64,
            arg_u8(4, 40),
            arg(5, 0),
            parse_spec(&arg_s(6, "default")),
            false,
        ),
        "duelf" => duel_run(arg_u8(1, 5), arg(2, 12), 0, MAX_TURNS, arg(3, 0), parse_spec(&arg_s(4, "default")), true),
        "mirror" => mirror_run(
            arg(1, 10),
            arg(2, 15_000) as u64,
            &arg_s(3, "default"),
            parse_spec(&arg_s(3, "default")),
            &arg_s(4, "default"),
            parse_spec(&arg_s(4, "default")),
            arg(5, 0),
            false,
        ),
        "fmirror" => mirror_run(
            arg(1, 10),
            0,
            &arg_s(2, "default"),
            parse_spec(&arg_s(2, "default")),
            &arg_s(3, "default"),
            parse_spec(&arg_s(3, "default")),
            arg(4, 0),
            true,
        ),
        "check" => check(arg(1, 4), parse_spec(&arg_s(2, "default"))),
        "prodpath" => prodpath(arg(1, 2)),
        _ => println!("scénarios: sanity [n], speed [n] [depth] [spec], duel <lvl> [n] [cap] [depth] [off] [spec], duelf <lvl> [n] [off] [spec], mirror [n] [cap] <A> <B> [off], fmirror [n] <A> <B> [off], check [n] [spec], prodpath [n]"),
    }
}
