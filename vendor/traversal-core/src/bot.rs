//! Bot : alpha-beta à approfondissement itératif, borné en temps.
//!
//! Avec `insight = 0`, c'est exactement le bot de référence du serveur (même évaluation,
//! même ordre des coups) : c'est ce que vérifient les tests d'équivalence. Au-delà, chaque
//! niveau de compréhension ajoute une notion du jeu (cumulatives), propre au bot Rust ;
//! chacune a été gardée parce qu'elle gagne en tournoi (des centaines de parties par
//! duel, mesurées avec `Tuning`) :
//!
//! 1. **les échanges** — au bout de l'horizon, la recherche continue tant qu'il reste des
//!    captures ou des sorties : plus de coup joué au milieu d'un échange, ni de sortie
//!    adverse ignorée parce qu'elle tombe juste après l'horizon.
//! 2. **le terrain et les parades** — la distance à la sortie tient compte des pièces (une
//!    Sentinelle adverse est un mur, les autres pièces se franchissent en les abattant, une
//!    alliée doit s'écarter) ; et les coups qui barrent le chemin du coureur adverse sont
//!    toujours examinés, même quand la recherche n'en garde que quelques-uns.
//! 3. **la course des Sentinelles** (R8.2) — personne n'est sorti : qui sortira sa
//!    Sentinelle le premier, sachant qu'une sortie adverse quelconque annule cette victoire.
//!
//! Les niveaux ne sont PAS définis ici : l'appelant passe le sien (`Level`). Ceux du site
//! (Galet à Mont Blanc) sont recopiés dans `bots/starter`. Les égalités de score peuvent
//! se départager autrement que sur le serveur (ordre de génération des coups) : à niveau
//! égal, même force, pas forcément même coup.

use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::time::{Duration, Instant};

use crate::config::{Kind, SIZE, SQUARES};
use crate::movegen::{legal_moves, neighbors_of, step_cost, Move, MoveKind};
use crate::rules::apply_move;
use crate::state::{State, MAX_PIECES};

pub const WIN: i32 = 100_000;
const INF: i32 = 1_000_000_000;

/// Poids des notions de compréhension (réglables depuis `traversal-cli` pour les tournois).
#[derive(Clone, Copy, Debug)]
pub struct Tuning {
    /// Niveau 1 : profondeur de la recherche de calme (captures et sorties seulement).
    pub qplies: u8,
    /// Niveau 2 : coût d'une case occupée par une pièce alliée (elle devra s'écarter).
    pub ally: u16,
    /// Niveau 2 : coût pour passer par une pièce adverse qu'on peut abattre (Explorateur,
    /// Avant-garde) ; une Sentinelle adverse reste un mur. 0 : toutes sont des murs.
    pub soft: u16,
    /// Niveau 2 : plafond du détour imposé par les pièces (au-delà, elles finiront par bouger).
    pub detour: u16,
    /// Niveau 2 : part (en %) de la distance « avec obstacles » dans la distance retenue.
    pub blend: i32,
    /// Niveau 2 : coups qui barrent le chemin du coureur adverse, ajoutés aux `beam`
    /// meilleurs (jamais élagués).
    pub guard_extra: usize,
    /// Niveau 3 : course des Sentinelles gagnée / perdue, et à partir de combien de tours
    /// restants elle compte.
    pub srace: i32,
    pub srace_horizon: i32,
}

impl Default for Tuning {
    fn default() -> Tuning {
        Tuning { qplies: 4, ally: 2, soft: 2, detour: 6, blend: 100, guard_extra: 2, srace: 150, srace_horizon: 5 }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Level {
    /// Profondeur maximale (demi-coups).
    pub depth: u8,
    /// Nombre de coups explorés sous la racine.
    pub beam: usize,
    /// Bruit sur l'évaluation des coups racine (erreurs « humaines »).
    pub noise: f64,
    pub budget: Duration,
    /// Compréhension du jeu (0 : bot de référence, 1 à 3 : voir le module).
    pub insight: u8,
    pub tuning: Tuning,
    /// Arrêt de la recherche décidé par l'appelant, à la place de `budget` (ignoré
    /// alors). En WebAssembly il n'y a pas d'horloge : un bot de l'arène s'arrête sur le
    /// fuel qui lui reste (`traversal-bot`), et la recherche devient reproductible.
    pub stop: Option<fn() -> bool>,
}

/// Coût minimal (sur le relief, sans tenir compte des pièces) pour sortir depuis chaque
/// case, pour le camp `owner`.
pub fn exit_costs(heights: &[u8; SQUARES], owner: u8) -> [u16; SQUARES] {
    let row = if owner == 1 { SIZE - 1 } else { 0 };
    let mut dist = [u16::MAX; SQUARES];
    let mut heap = BinaryHeap::new();
    for x in 0..SIZE {
        let s = row * SIZE + x;
        dist[s] = 1;
        heap.push(Reverse((1u16, s as u8)));
    }
    while let Some(Reverse((d, cur))) = heap.pop() {
        if d > dist[cur as usize] {
            continue;
        }
        for &nb in neighbors_of(cur) {
            let nd = d + step_cost(heights, cur, nb) as u16;
            if nd < dist[nb as usize] {
                dist[nb as usize] = nd;
                heap.push(Reverse((nd, nb)));
            }
        }
    }
    dist
}

/// Niveau 2 : coût pour sortir depuis chaque case en tenant compte des pièces — une
/// Sentinelle adverse est un mur, les autres pièces adverses se franchissent en les
/// abattant (`t.soft`), traverser une alliée coûte `t.ally` (elle doit s'écarter). La valeur
/// sur la case d'une pièce `owner` est son propre coût (sa case n'est pas pénalisée pour elle).
fn blocked_costs(state: &State, owner: u8, t: &Tuning) -> [u16; SQUARES] {
    let row = if owner == 1 { SIZE - 1 } else { 0 };
    let other = |sq: u8| state.at(sq).map(|i| state.piece(i)).filter(|p| p.owner != owner);
    let wall = |sq: u8| other(sq).is_some_and(|p| t.soft == 0 || p.kind == Kind::Sentinel);
    let ally = |sq: u8| state.at(sq).is_some_and(|i| state.piece(i).owner == owner);
    let mut dist = [u16::MAX; SQUARES];
    let mut heap = BinaryHeap::new();
    for x in 0..SIZE {
        let s = (row * SIZE + x) as u8;
        if !wall(s) {
            dist[s as usize] = 1;
            heap.push(Reverse((1u16, s)));
        }
    }
    while let Some(Reverse((d, cur))) = heap.pop() {
        if d > dist[cur as usize] {
            continue;
        }
        // Coût pour ENTRER sur `cur` depuis une voisine : la marche, plus la pièce à écarter
        // (alliée) ou à abattre (adverse).
        let enter = if ally(cur) {
            t.ally
        } else if other(cur).is_some() {
            t.soft
        } else {
            0
        };
        for &nb in neighbors_of(cur) {
            if wall(nb) {
                continue;
            }
            let nd = d + step_cost(&state.heights, nb, cur) as u16 + enter;
            if nd < dist[nb as usize] {
                dist[nb as usize] = nd;
                heap.push(Reverse((nd, nb)));
            }
        }
    }
    dist
}

/// Données d'une recherche (le relief ne change pas pendant la partie).
pub struct Ctx {
    costs: [[u16; SQUARES]; 2],
    insight: u8,
    t: Tuning,
}

impl Ctx {
    pub fn new(heights: &[u8; SQUARES]) -> Ctx {
        Ctx::with_insight(heights, 0, Tuning::default())
    }

    pub fn with_insight(heights: &[u8; SQUARES], insight: u8, t: Tuning) -> Ctx {
        Ctx { costs: [exit_costs(heights, 1), exit_costs(heights, 2)], insight, t }
    }

    #[inline]
    fn cost(&self, owner: u8, sq: u8) -> i32 {
        self.costs[owner as usize - 1][sq as usize] as i32
    }

    #[inline]
    fn turns_for(cost: i32, kind: Kind) -> i32 {
        let m = kind.movement() as i32;
        (cost + m - 1) / m
    }

    /// Tours (au mieux) pour sortir, pour chaque pièce : sur le relief seul, ou (niveau 2)
    /// en tenant compte des pièces qui barrent le chemin.
    fn piece_turns(&self, state: &State) -> [i32; MAX_PIECES] {
        let mut out = [0; MAX_PIECES];
        let blocked = (self.insight >= 2)
            .then(|| [blocked_costs(state, 1, &self.t), blocked_costs(state, 2, &self.t)]);
        for (i, p) in state.pieces().iter().enumerate() {
            let free = self.cost(p.owner, p.sq);
            let cost = match &blocked {
                None => free,
                Some(b) => {
                    let with = (b[p.owner as usize - 1][p.sq as usize] as i32).min(free + self.t.detour as i32);
                    (free * (100 - self.t.blend) + with * self.t.blend + 50) / 100
                }
            };
            out[i] = Ctx::turns_for(cost, p.kind);
        }
        out
    }

    /// Évaluation du point de vue de `side`.
    pub fn evaluate(&self, state: &State, side: u8) -> i32 {
        if state.over {
            return match state.winner {
                None => 0,
                Some(w) if w == side => WIN - state.ply as i32,
                Some(_) => -WIN + state.ply as i32,
            };
        }
        let nobody_out = state.finished == 0;
        let other = if side == 1 { 2 } else { 1 };
        let mut total = 100 * (state.scores[side as usize - 1] as i32 - state.scores[other as usize - 1] as i32);
        let turns = self.piece_turns(state);
        let mut first = [i32::MAX; 2]; // pièce la plus rapide de chaque camp (niveau 3)
        let mut sentinel = [i32::MAX; 2]; // Sentinelle la plus rapide de chaque camp (niveau 3)
        for (i, p) in state.pieces().iter().enumerate() {
            let strength = p.kind.strength() as i32;
            let t = turns[i];
            let mut v = 14 * strength + (7 - t).max(0) * (3 + 2 * strength);
            if p.kind == Kind::Sentinel && nobody_out {
                v += (6 - t).max(0).pow(2) * 5;
            }
            total += if p.owner == side { v } else { -v };
            let o = p.owner as usize - 1;
            first[o] = first[o].min(t);
            if p.kind == Kind::Sentinel {
                sentinel[o] = sentinel[o].min(t);
            }
        }
        if self.insight >= 3 && nobody_out {
            total += self.sentinel_race(state, side, &first, &sentinel);
        }
        total
    }

    /// Niveau 3 : course des Sentinelles (R8.2). Personne n'est sorti : une Sentinelle gagne
    /// si elle sort AVANT la Sentinelle adverse ET avant toute pièce adverse (une sortie
    /// adverse, même d'un Explorateur, annule la victoire immédiate). En demi-coups, le camp
    /// au trait joue le premier.
    fn sentinel_race(&self, state: &State, side: u8, first: &[i32; 2], sentinel: &[i32; 2]) -> i32 {
        let plies = |owner: u8, turns: i32| match turns {
            i32::MAX => i32::MAX,
            t if state.to_move == owner => 2 * t - 1,
            t => 2 * t,
        };
        let wins = |me: u8| {
            let them = if me == 1 { 2 } else { 1 };
            let mine = plies(me, sentinel[me as usize - 1]);
            mine < plies(them, first[them as usize - 1]) && sentinel[me as usize - 1] <= self.t.srace_horizon
        };
        let other = if side == 1 { 2 } else { 1 };
        match (wins(side), wins(other)) {
            (true, false) => self.t.srace,
            (false, true) => -self.t.srace,
            _ => 0,
        }
    }

    fn order_key(&self, state: &State, m: &Move) -> i32 {
        match m.kind {
            MoveKind::Exit => 1000 + state.piece(m.piece).kind.strength() as i32,
            MoveKind::Attack => {
                let target = state.piece(state.at(m.to).expect("cible"));
                500 + 10 * target.kind.strength() as i32
            }
            MoveKind::Pass => 0,
            MoveKind::Move => {
                let p = state.piece(m.piece);
                self.cost(p.owner, p.sq) - self.cost(p.owner, m.to)
            }
        }
    }

    /// Coups légaux, les plus prometteurs d'abord (tri stable).
    pub fn ordered(&self, state: &State) -> Vec<Move> {
        let mut keyed: Vec<(i32, Move)> = legal_moves(state).into_iter().map(|m| (self.order_key(state, &m), m)).collect();
        keyed.sort_by_key(|(k, _)| Reverse(*k));
        keyed.into_iter().map(|(_, m)| m).collect()
    }

    /// Coups explorés sous la racine : les `beam` meilleurs, plus (niveau 2) quelques coups
    /// qui barrent le chemin du coureur adverse — une parade n'est jamais élaguée d'emblée.
    fn candidates(&self, state: &State, beam: usize) -> Vec<Move> {
        let all = self.ordered(state);
        if self.insight < 2 || all.len() <= beam {
            return all.into_iter().take(beam).collect();
        }
        let guard = self.guard_squares(state);
        let mut out: Vec<Move> = all[..beam].to_vec();
        out.extend(
            all[beam..]
                .iter()
                .filter(|m| m.kind == MoveKind::Move && guard[m.to as usize])
                .take(self.t.guard_extra),
        );
        out
    }

    /// Niveau 2 : cases sur un plus court chemin du coureur adverse le plus rapide (celui
    /// du camp qui n'a pas le trait) — s'y placer le ralentit.
    fn guard_squares(&self, state: &State) -> [bool; SQUARES] {
        let mut out = [false; SQUARES];
        let them = if state.to_move == 1 { 2 } else { 1 };
        let Some(runner) = state
            .pieces()
            .iter()
            .filter(|p| p.owner == them)
            .min_by_key(|p| Ctx::turns_for(self.cost(p.owner, p.sq), p.kind))
        else {
            return out;
        };
        // Distance depuis le coureur (relief seul) : une case est sur un plus court chemin
        // si (coureur → case) + (case → sortie) = (coureur → sortie).
        let mut from = [u16::MAX; SQUARES];
        let mut heap = BinaryHeap::new();
        from[runner.sq as usize] = 0;
        heap.push(Reverse((0u16, runner.sq)));
        while let Some(Reverse((d, cur))) = heap.pop() {
            if d > from[cur as usize] {
                continue;
            }
            for &nb in neighbors_of(cur) {
                let nd = d + step_cost(&state.heights, cur, nb) as u16;
                if nd < from[nb as usize] {
                    from[nb as usize] = nd;
                    heap.push(Reverse((nd, nb)));
                }
            }
        }
        let total = self.cost(them, runner.sq);
        for sq in 0..SQUARES {
            if sq != runner.sq as usize && from[sq] != u16::MAX && from[sq] as i32 + self.cost(them, sq as u8) == total {
                out[sq] = true;
            }
        }
        out
    }
}

struct Timeout;

/// Recherche bornée en temps.
pub struct Search<'a> {
    ctx: &'a Ctx,
    level: Level,
    /// `None` avec `Level::stop` : jamais d'appel à l'horloge (absente en WebAssembly).
    deadline: Option<Instant>,
    pub nodes: u64,
}

impl<'a> Search<'a> {
    pub fn new(ctx: &'a Ctx, level: Level) -> Search<'a> {
        let deadline = level.stop.is_none().then(|| Instant::now() + level.budget);
        Search { ctx, level, deadline, nodes: 0 }
    }

    fn tick(&mut self) -> Result<(), Timeout> {
        self.nodes += 1;
        let out = match (self.level.stop, self.deadline) {
            // Plus fréquent : l'appelant doit pouvoir s'arrêter avant d'épuiser sa réserve.
            (Some(stop), _) => self.nodes % 64 == 0 && stop(),
            (None, Some(deadline)) => self.nodes % 512 == 0 && Instant::now() > deadline,
            (None, None) => false,
        };
        if out {
            return Err(Timeout);
        }
        Ok(())
    }

    fn negamax(&mut self, state: &State, depth: u8, mut alpha: i32, beta: i32) -> Result<i32, Timeout> {
        if state.over {
            return Ok(self.ctx.evaluate(state, state.to_move));
        }
        if depth == 0 {
            return if self.ctx.insight >= 1 {
                self.quiesce(state, alpha, beta, self.ctx.t.qplies)
            } else {
                Ok(self.ctx.evaluate(state, state.to_move))
            };
        }
        self.tick()?;
        let mut best = -INF;
        for m in self.ctx.candidates(state, self.level.beam) {
            let score = -self.negamax(&apply_move(state, &m), depth - 1, -beta, -alpha)?;
            best = best.max(score);
            alpha = alpha.max(score);
            if alpha >= beta {
                break;
            }
        }
        Ok(best)
    }

    /// Niveau 1 : recherche de calme — au bout de l'horizon, on poursuit les seules
    /// captures et sorties, jusqu'à une position calme (le camp au trait peut aussi « ne
    /// rien faire » : sa note sur place sert de plancher).
    fn quiesce(&mut self, state: &State, mut alpha: i32, beta: i32, plies: u8) -> Result<i32, Timeout> {
        let stand = self.ctx.evaluate(state, state.to_move);
        if state.over || plies == 0 || stand >= beta {
            return Ok(stand);
        }
        self.tick()?;
        alpha = alpha.max(stand);
        let mut best = stand;
        for m in self.ctx.ordered(state) {
            if !matches!(m.kind, MoveKind::Exit | MoveKind::Attack) {
                break; // triés : sorties et captures d'abord
            }
            let score = -self.quiesce(&apply_move(state, &m), -beta, -alpha, plies - 1)?;
            best = best.max(score);
            alpha = alpha.max(score);
            if alpha >= beta {
                break;
            }
        }
        Ok(best)
    }
}

/// Résultat d'une recherche (pour le diagnostic et les outils).
#[derive(Clone, Copy, Debug)]
pub struct Choice {
    pub mv: Move,
    /// Profondeur complètement explorée.
    pub depth: u8,
    pub score: i32,
    pub nodes: u64,
}

/// Petit générateur pseudo-aléatoire (SplitMix64) : reproductible, sans dépendance.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniforme dans [lo, hi).
    pub fn uniform(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * ((self.next_u64() >> 11) as f64 / (1u64 << 53) as f64)
    }

    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }
}

/// Coup choisi par le bot (avec `insight = 0` : même algorithme que le bot de référence
/// du serveur).
pub fn choose_move(state: &State, level: Level, rng: &mut Rng) -> Choice {
    let ctx = Ctx::with_insight(&state.heights, level.insight, level.tuning);
    let mut search = Search::new(&ctx, level);
    let mut root: Vec<(Move, f64)> = ctx.ordered(state).into_iter().map(|m| (m, 0.0)).collect();
    if root.len() == 1 {
        return Choice { mv: root[0].0, depth: 0, score: 0, nodes: 0 };
    }
    for entry in root.iter_mut() {
        entry.1 = rng.uniform(-level.noise, level.noise);
    }
    let mut best = Choice { mv: root[0].0, depth: 0, score: 0, nodes: 0 };
    'deepen: for depth in 1..=level.depth {
        let mut scored: Vec<(f64, i32, Move, f64)> = Vec::with_capacity(root.len());
        let mut alpha = -INF;
        for &(m, noise) in &root {
            let child = apply_move(state, &m);
            // Niveau 1 : même à la profondeur 1, la réponse « calme » (captures, sorties) compte.
            let score = if depth > 1 || level.insight >= 1 {
                match search.negamax(&child, depth - 1, -INF, -alpha + 1) {
                    Ok(s) => -s,
                    Err(Timeout) => break 'deepen,
                }
            } else {
                ctx.evaluate(&child, state.to_move)
            };
            scored.push((score as f64 + noise, score, m, noise));
            alpha = alpha.max(score);
        }
        // Tri stable : à égalité, l'ordre de l'itération précédente est gardé.
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        best = Choice { mv: scored[0].2, depth, score: scored[0].1, nodes: search.nodes };
        // Coups racine triés par le résultat de cette itération : meilleure coupe ensuite.
        root = scored.into_iter().map(|(_, _, m, noise)| (m, noise)).collect();
    }
    best.nodes = search.nodes;
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Piece;

    fn flat() -> [u8; SQUARES] {
        [1; SQUARES]
    }

    #[test]
    fn enemy_sentinel_is_a_wall_and_lighter_pieces_can_be_cut_through() {
        // Sentinelle rouge en (4,7), la sortie est en rangée 9 ; une pièce bleue juste devant.
        let red = Piece { owner: 1, kind: Kind::Sentinel, sq: 74 };
        let t = Tuning::default();
        let free = exit_costs(&flat(), 1)[74];
        let wall = State::new(flat(), &[red, Piece { owner: 2, kind: Kind::Sentinel, sq: 84 }], 1, 0, [0, 0], 0, 200);
        assert_eq!(free, 3);
        assert_eq!(blocked_costs(&wall, 1, &t)[74], 4, "contournement d'une case");
        let soft = State::new(flat(), &[red, Piece { owner: 2, kind: Kind::Explorer, sq: 84 }], 1, 0, [0, 0], 0, 200);
        assert!(blocked_costs(&soft, 1, &t)[74] <= 4);
    }

    #[test]
    fn sentinel_race_counts_any_enemy_exit() {
        // Rouge au trait, sa Sentinelle à 1 tour de la sortie ; Bleu n'a rien de proche.
        let pieces = [Piece { owner: 1, kind: Kind::Sentinel, sq: 84 }, Piece { owner: 2, kind: Kind::Explorer, sq: 45 }];
        let s = State::new(flat(), &pieces, 1, 0, [0, 0], 0, 200);
        let (with, without) = (Ctx::with_insight(&s.heights, 3, Tuning::default()), Ctx::with_insight(&s.heights, 2, Tuning::default()));
        assert_eq!(with.evaluate(&s, 1) - without.evaluate(&s, 1), Tuning::default().srace);
        // Un Explorateur bleu prêt à sortir le premier annule la course : plus de bonus.
        let pieces = [Piece { owner: 1, kind: Kind::Sentinel, sq: 64 }, Piece { owner: 2, kind: Kind::Explorer, sq: 5 }];
        let s = State::new(flat(), &pieces, 2, 0, [0, 0], 0, 200);
        assert!(with.evaluate(&s, 1) <= without.evaluate(&s, 1));
    }
}
