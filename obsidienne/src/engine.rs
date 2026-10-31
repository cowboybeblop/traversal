//! Moteur de recherche d'Obsidienne : négamax alpha-beta (PVS) à approfondissement
//! itératif, table de transposition, tri de coups (TT, killers, history à gravité),
//! null-move gardé, LMR, LMP, futility, extension des sorties, quiescence (sorties +
//! attaques avec delta pruning), et cache d'évaluations Zobrist.

use traversal_bot::core::bot::exit_costs;
use traversal_bot::core::config::{Kind, SQUARES};
use traversal_bot::core::movegen::{legal_moves, Move, MoveKind};
use traversal_bot::core::rules::apply_move;
use traversal_bot::core::state::State;
use traversal_bot::core::Rng;

use crate::eval::{can_exit_now, eval, EvalCfg};
use crate::tt::{pack_move, NO_MOVE, TT};

pub const WIN: i32 = 100_000;
const INF: i32 = 1_000_000_000;
/// Profondeur maximale de la recherche (taille des tableaux killers).
pub const MAX_PLY: usize = 128;
/// Profondeur plafond demandée par la production.
pub const MAX_TURNS: u8 = 100;
/// Coups racine triés entre itérations (les positions en ont < 96).
const ROOT_CAP: usize = 96;
/// Fanout trié sans allocation en dessous de cette taille.
const FBUF: usize = 96;

#[derive(Clone, Copy)]
pub struct Flags {
    /// fenêtres d'aspiration à la racine dès la profondeur 5
    pub aspiration: bool,
    /// interrupteurs de recherche (A/B, diagnostics)
    pub nmp: bool,
    pub lmr: bool,
    pub fut: bool,
    pub lmp: bool,
    pub ttx: bool,
    /// poids du soutien (alliés adjacents)
    pub support_w: i32,
    /// poids des pièces menacées (par point de Force)
    pub hang_w: i32,
    /// part (en %) du coût « avec obstacles » dans la distance de progression
    pub blocked: i32,
    /// cache d'évaluations indexé par Zobrist (états non terminaux seulement)
    pub ecache: bool,
    /// racine : conserver le best move des itérations complétées (stabilité fuel)
    pub rs: bool,
    /// racine : ne re-rechercher en profondeur que les 4 meilleurs coups
    pub rn: bool,
    /// history : malus sur les coups explorés sans devenir best
    pub hb: bool,
    /// counter-move : le coup qui a répondu au coup précédent est trié en tête
    pub cm: bool,
    /// race graduée : différentiel de Dijkstra Sentinelle au lieu du ±200 binaire
    pub rdif: bool,
    /// sûreté de la Sentinelle : distance à l'ennemi le plus proche capable de la battre
    pub ssafe: bool,
    /// continuation history (1 ply) dans le tri des coups silencieux
    pub ch: bool,
    /// SEE signé : les attaques perdantes sont triées après les coups silencieux
    pub seet: bool,
    /// eval : contrôle des zones de sortie (bord opposé à portée)
    pub zc: bool,
    /// eval : terme matériel (différentiel de Force des pièces vivantes)
    pub mat: bool,
    /// 3 killers par ply au lieu de 2
    pub k3: bool,
    /// internal iterative reduction : pas de coup TT à prof ≥ 4 → réduire de 1
    pub iir: bool,
    /// progression par table apprise PROG[kind][t] au lieu de la formule
    pub ptab: bool,
    /// hang tabulaire par type de pièce (25/35/40) au lieu de hang_w×Force
    pub hangtab: bool,
    /// hang défendu : demi-tarif si la pièce a des soutiens (def > Force)
    pub hangd: bool,
    /// élagage adaptatif : positions tactiques (attaques légales) → futility off, LMR off
    pub crit: bool,
    /// LMR précoce : prof ≥ 2 et searched ≥ 3 (au lieu de 3/4)
    pub lmr2: bool,
    /// extension de course : les deux Sentinelles à ≤ 3 tours de sortie → +1 ply
    pub racc: bool,
    /// ProbCut : recherche superficielle prof 2 ≥ beta + marge → couper
    pub pcut: bool,
    /// contrôle du relief : prime par point de hauteur occupé (attaque/défense)
    pub hi: bool,
}

impl Default for Flags {
    fn default() -> Flags {
        Flags { aspiration: false, nmp: true, lmr: true, fut: true, lmp: true, ttx: true, support_w: 0, hang_w: 0, blocked: 0, ecache: false, rs: false, rn: false, hb: false, cm: false, rdif: false, ssafe: false, ch: false, seet: false, zc: false, mat: false, k3: false, iir: false, ptab: false, hangtab: false, hangd: false, crit: false, lmr2: false, racc: false, pcut: false, hi: false }
    }
}

/// Taille du cache d'évaluations : 2^18 entrées (u64 clé + i32 valeur) = 4 Mo.
const ECACHE_N: usize = 1 << 18;

impl Flags {
    /// Ensemble validé par criblage A/B (voir EXPERIMENTS.md) :
    /// hang=30 (pièces menacées), sans null-move ni LMP (perdants en miroir
    /// fuel), race graduée (différentiel de sortie Sentinelle, +64 Elo/104 p.).
    /// `blocked` et le cache d'évaluations sont écartés : le coût fuel de
    /// `blocked` (+2 Dijkstra/éval) annule son gain, le cache est neutre.
    pub fn best() -> Flags {
        Flags { hang_w: 30, nmp: false, lmp: false, rdif: true, ..Flags::default() }
    }
}

/// Une pièce d'indice donné est-elle vivante, et laquelle ?
fn chist_new() -> [[[[u16; SQUARES]; 3]; SQUARES]; 3] {
    [[[[0u16; SQUARES]; 3]; SQUARES]; 3]
}

fn live_piece(s: &State, owner: u8, kind: Kind, sq: u8) -> Option<traversal_bot::core::state::Piece> {
    s.pieces().iter().find(|p| p.owner == owner && p.kind == kind && p.sq == sq).copied()
}

fn kind3(k: Kind) -> usize {
    match k {
        Kind::Explorer => 0,
        Kind::Vanguard => 1,
        Kind::Sentinel => 2,
    }
}

fn never() -> bool {
    false
}

pub struct Engine {
    pub tt: TT,
    killers: [[u32; 3]; MAX_PLY + 2],
    history: [[[u32; SQUARES]; SQUARES]; 3],
    costs: [[u16; SQUARES]; 2],
    pub heights: [u8; SQUARES],
    cfg: EvalCfg,
    flags: Flags,
    zob: [[u64; SQUARES]; 6],
    z_turn: u64,
    z_s: [[u64; 16]; 2],
    z_fin: [u64; 4],
    stop: fn() -> bool,
    pub nodes: u64,
    pub total_nodes: u64,
    pub last_depth: u8,
    pub last_ply: u16,
    aborted: bool,
    root_scores: [i32; ROOT_CAP],
    /// cache d'évaluations direct-mapped (clé Zobrist → valeur), si `flags.ecache`.
    ecache: Vec<(u64, i32)>,
    /// counter-move : réponse enregistrée par (pièce, case) du coup précédent.
    cmoves: [[u32; SQUARES]; 6],
    /// dernier coup packé joué (pour counter-move), par ply.
    last_mv: [u32; MAX_PLY + 2],
    /// continuation history : history[kind][to][kind_prev][to_prev]
    chist: Vec<[[[[u16; SQUARES]; 3]; SQUARES]; 3]>,
    /// dernier coup silencieux packé, par ply (pour continuation history)
    prev_quiet: [u32; MAX_PLY + 2],
}

impl Engine {
    pub fn new(heights: [u8; SQUARES], cfg: EvalCfg, flags: Flags) -> Engine {
        let mut rng = Rng::new(0x0B51_D00D_5EED_1234);
        let mut zob = [[0u64; SQUARES]; 6];
        for k in 0..6 {
            for sq in 0..SQUARES {
                zob[k][sq] = rng.next_u64();
            }
        }
        let mut z_s = [[0u64; 16]; 2];
        for o in 0..2 {
            for i in 0..16 {
                z_s[o][i] = rng.next_u64();
            }
        }
        let mut z_fin = [0u64; 4];
        for i in 0..4 {
            z_fin[i] = rng.next_u64();
        }
        Engine {
            tt: TT::new(),
            killers: [[NO_MOVE; 3]; MAX_PLY + 2],
            history: [[[0; SQUARES]; SQUARES]; 3],
            costs: [exit_costs(&heights, 1), exit_costs(&heights, 2)],
            heights,
            cfg,
            flags,
            zob,
            z_turn: rng.next_u64(),
            z_s,
            z_fin,
            stop: never,
            nodes: 0,
            total_nodes: 0,
            last_depth: 0,
            last_ply: 0,
            aborted: false,
            root_scores: [0; ROOT_CAP],
            ecache: vec![(0u64, 0i32); ECACHE_N],
            cmoves: [[NO_MOVE; SQUARES]; 6],
            last_mv: [NO_MOVE; MAX_PLY + 2],
            chist: vec![chist_new()],
            prev_quiet: [NO_MOVE; MAX_PLY + 2],
        }
    }

    #[inline]
    fn zobrist(&self, s: &State) -> u64 {
        let mut h = 0u64;
        for p in s.pieces() {
            h ^= self.zob[(p.owner as usize - 1) * 3 + kind3(p.kind)][p.sq as usize];
        }
        if s.to_move == 2 {
            h ^= self.z_turn;
        }
        h ^= self.z_s[0][(s.scores[0] & 15) as usize]
            ^ self.z_s[1][(s.scores[1] & 15) as usize]
            ^ self.z_fin[(s.finished & 3) as usize];
        h
    }

    #[inline]
    fn eval_node(&mut self, s: &State, side: u8) -> i32 {
        // Les états terminaux dépendent du ply (WIN - s.ply) : jamais cachés.
        if s.over || !self.flags.ecache {
            return eval(s, side, &self.cfg, &self.costs, &self.flags);
        }
        let key = self.zobrist(s);
        let idx = (key as usize) & (ECACHE_N - 1);
        if self.ecache[idx].0 == key {
            return self.ecache[idx].1;
        }
        let v = eval(s, side, &self.cfg, &self.costs, &self.flags);
        self.ecache[idx] = (key, v);
        v
    }

    #[inline]
    fn node_tick(&mut self) -> bool {
        self.nodes += 1;
        if (self.nodes & 63) == 0 && (self.stop)() {
            self.aborted = true;
            return false;
        }
        true
    }

    fn root_key(&self, s: &State, m: &Move) -> i32 {
        match m.kind {
            MoveKind::Exit => 600_000 + s.piece(m.piece).kind.strength() as i32,
            MoveKind::Attack => {
                let t = s.piece(s.at(m.to).unwrap_or(0));
                500_000 + 32 * t.kind.strength() as i32 + (m.attack as i32 - m.defense as i32).max(0) * 4
            }
            MoveKind::Pass => 0,
            MoveKind::Move => {
                let p = s.piece(m.piece);
                let o = (p.owner - 1) as usize;
                (self.costs[o][p.sq as usize] as i32 - self.costs[o][m.to as usize] as i32).max(-999)
            }
        }
    }

    /// Clé d'ordonnancement d'un coup interne.
    fn move_key(&self, s: &State, m: &Move, tt_mv: u32, ply: usize) -> i32 {
        let packed = pack_move(m);
        let mut k = match m.kind {
            MoveKind::Exit => 600_000 + s.piece(m.piece).kind.strength() as i32,
            MoveKind::Attack => {
                let t = s.piece(s.at(m.to).unwrap_or(0));
                500_000 + 32 * t.kind.strength() as i32 + (m.attack as i32 - m.defense as i32).max(0) * 4
            }
            MoveKind::Pass => 0,
            MoveKind::Move => {
                let p = s.piece(m.piece);
                self.history[kind3(p.kind)][p.sq as usize][m.to as usize].min(200_000) as i32
            }
        };
        if packed == tt_mv {
            k += 1_000_000;
        }
        if packed == self.killers[ply][0] {
            k += 400_000;
        } else if packed == self.killers[ply][1] {
            k += 380_000;
        } else if self.flags.k3 && packed == self.killers[ply][2] {
            k += 360_000;
        }
        if self.flags.cm && m.kind == MoveKind::Move && ply >= 1 {
            let p = s.piece(m.piece);
            let pk = kind3(p.kind) * 2 + (p.owner as usize - 1);
            if self.cmoves[pk][p.sq as usize] == packed {
                k += 200_000;
            }
        }
        if self.flags.ch && m.kind == MoveKind::Move {
            let p = s.piece(m.piece);
            let pq = self.prev_quiet[ply];
            if pq != NO_MOVE && pq & 0x30000 == 0 {
                let prev_to = (pq & 0xFF) as usize;
                let prev_kind = ((pq >> 8) & 0xFF) as usize;
                if prev_kind < 3 {
                    k += (self.chist[0][kind3(p.kind)][m.to as usize][prev_kind][prev_to] as i32).min(2048) * 4;
                }
            }
        }
        if self.flags.seet && m.kind == MoveKind::Attack && m.attack < m.defense {
            k -= 450_000;
        }
        k
    }

    /// Coup de coupure : killers + bonus d'history à gravité (32·d², cap 2^24).
    fn bump_history(&mut self, s: &State, m: &Move, ply: usize, depth: i8) {
        let packed = pack_move(m);
        let slots = if self.flags.k3 { 3 } else { 2 };
        if self.killers[ply][0] != packed {
            let mut j = 1;
            while j < slots && self.killers[ply][j] != packed {
                j += 1;
            }
            // décaler (nouveau coup en tête)
            let mut k = j.min(slots - 1);
            while k > 0 {
                self.killers[ply][k] = self.killers[ply][k - 1];
                k -= 1;
            }
            if j >= slots {
                self.killers[ply][slots - 1] = self.killers[ply][slots - 1];
            }
            self.killers[ply][0] = packed;
        }
        let p = s.piece(m.piece);
        let k = kind3(p.kind);
        let d2 = (depth as i32 * depth as i32) as u32;
        let h = &mut self.history[k][p.sq as usize][m.to as usize];
        *h = (*h).saturating_add(32 * d2).min(1 << 24);
    }

    /// Cherche un coup : PVS + LMR (recherche réduite puis re-recherche si prometteur).
    fn try_move(
        &mut self,
        s: &State,
        m: &Move,
        depth: i8,
        alpha: i32,
        beta: i32,
        ply: usize,
        searched: usize,
        allow_lmr: bool,
    ) -> Option<i32> {
        let cd = (depth - 1 + if m.kind == MoveKind::Exit { 1 } else { 0 }).max(0);
        if self.flags.ch {
            let saved = self.prev_quiet[ply];
            let kq = if m.kind == MoveKind::Move {
                ((kind3(s.piece(m.piece).kind) as u32) << 8) | m.to as u32
            } else { NO_MOVE };
            self.prev_quiet[ply] = kq;
            let r = self.try_move_inner(s, m, cd, alpha, beta, ply, searched, allow_lmr);
            self.prev_quiet[ply] = saved;
            return r;
        }
        self.try_move_inner(s, m, cd, alpha, beta, ply, searched, allow_lmr)
    }

    fn try_move_inner(
        &mut self,
        s: &State,
        m: &Move,
        cd: i8,
        alpha: i32,
        beta: i32,
        ply: usize,
        searched: usize,
        allow_lmr: bool,
    ) -> Option<i32> {
        let depth = cd + 1 - if m.kind == MoveKind::Exit { 1 } else { 0 };
        let child = apply_move(s, m);
        if searched == 0 {
            return Some(-self.negamax(&child, cd, -beta, -alpha, ply + 1)?);
        }
        let mut r = 0i8;
        if self.flags.lmr2 {
            if allow_lmr && self.flags.lmr && depth >= 2 && searched >= 3 && m.kind == MoveKind::Move {
                r = 1 + (searched as i32 / 20) as i8;
                if cd - r < 1 {
                    r = (cd - 1).max(0);
                }
            }
        } else if allow_lmr && self.flags.lmr && depth >= 3 && searched >= 4 && m.kind == MoveKind::Move {
            r = 1 + (searched as i32 / 16) as i8;
            if cd - r < 1 {
                r = (cd - 1).max(0);
            }
        }
        let probe = -self.negamax(&child, (cd - r).max(0), -alpha - 1, -alpha, ply + 1)?;
        if probe > alpha && (r > 0 || beta - alpha > 1) {
            return Some(-self.negamax(&child, cd, -beta, -alpha, ply + 1)?);
        }
        Some(probe)
    }

    fn negamax(&mut self, s: &State, depth: i8, mut alpha: i32, beta: i32, ply: usize) -> Option<i32> {
        if s.over || ply >= MAX_PLY {
            return Some(self.eval_node(s, s.to_move));
        }
        let key = self.zobrist(s);
        if depth <= 0 {
            return self.quiesce(s, alpha, beta, ply);
        }
        if !self.node_tick() {
            return None;
        }

        let mut tt_mv = NO_MOVE;
        let tt_probe = if self.flags.ttx { self.tt.probe(key) } else { None };
        if self.flags.iir && tt_probe.is_none() && depth >= 4 {
            return self.negamax(s, depth - 1, alpha, beta, ply);
        }
        if let Some(e) = tt_probe {
            if e.depth as i8 >= depth {
                match e.bound {
                    0 => return Some(e.score),
                    1 if e.score >= beta => return Some(e.score),
                    2 if e.score <= alpha => return Some(e.score),
                    _ => {}
                }
            }
            tt_mv = e.mv;
        }

        let moves = legal_moves(s);

        // RNM, gardé anti-zugzwang : personne ne peut sortir ce tour, assez de pièces,
        // position calme — passer ne doit pas être une bonne option.
        if moves.len() > 1 {
            let my_count = s.pieces().iter().filter(|p| p.owner == s.to_move).count();
            let mover_exit = moves.iter().any(|m| m.kind == MoveKind::Exit);
            if self.flags.nmp && !mover_exit && my_count >= 3 && depth >= 3 && beta.abs() < WIN / 2 {
                let stand = self.eval_node(s, s.to_move);
                if stand >= beta {
                    let mut null = s.clone();
                    null.to_move = 3 - s.to_move;
                    if !can_exit_now(&null) {
                        let r = 2 + depth / 5;
                        let nd = (depth - 1 - r).max(0);
                        match self.negamax(&null, nd, -beta, -beta + 1, ply + 1) {
                            None => return None,
                            Some(sc) if sc >= beta => {
                                if self.flags.ttx {
                                    self.tt.store(key, sc, depth as u8, 1, NO_MOVE);
                                }
                                return Some(sc);
                            }
                            Some(_) => {}
                        }
                    }
                }
            }
        }

        // Extension de course (racc) : course de Sentinelles SERRÉE (R8.2)
        // → approfondir d'un ply, la décision y est critique.
        let mut depth = depth;
        if self.flags.racc && s.finished == 0 && depth <= 8 {
            let sent_t = |owner: u8| -> Option<i32> {
                let o = (owner - 1) as usize;
                s.pieces()
                    .iter()
                    .filter(|p| p.owner == owner && p.kind == Kind::Sentinel)
                    .map(|p| self.costs[o][p.sq as usize] as i32)
                    .min()
            };
            if let (Some(a), Some(b)) = (sent_t(1), sent_t(2)) {
                if a <= 14 && b <= 14 && (a - b).abs() <= 2 {
                    depth += 1;
                }
            }
        }
        // ProbCut (Buro) : une recherche réduite (prof 2, fenêtre large) qui
        // dépasse beta d'une marge prédit un fail-high de la recherche pleine.
        // Utile là où le null-move échoue (ici : NMP perdant, régime Othello).
        if self.flags.pcut && depth >= 4 && (beta - alpha) == 1 && beta.abs() < WIN / 2 {
            let margin = 320 + 80 * (depth as i32 - 4);
            let probe_beta = beta + margin;
            match self.negamax(s, 2, probe_beta - 1, probe_beta, ply) {
                None => return None,
                Some(sc) if sc >= probe_beta => {
                    if self.flags.ttx {
                        self.tt.store(key, sc, depth as u8, 1, NO_MOVE);
                    }
                    return Some(sc);
                }
                Some(_) => {}
            }
        }
        let n = moves.len();
        let alpha0 = alpha;
        let stand_opt = if depth <= 2 { Some(self.eval_node(s, s.to_move)) } else { None };

        // Futility : position calme sans espoir à faible profondeur → stand-pat.
        // crit (élagage adaptatif) : en position tactique (attaque légale présente),
        // la marge de futility est doublée — on élargit la fenêtre de prudence.
        let tactical = self.flags.crit && moves.iter().any(|m| m.kind == MoveKind::Attack);
        if self.flags.fut && depth <= 2 {
            if let Some(stand) = stand_opt {
                let calm = !moves.iter().any(|m| matches!(m.kind, MoveKind::Exit | MoveKind::Attack));
                let margin = if tactical { self.cfg.futility * 2 } else { self.cfg.futility };
                if calm && stand + margin * depth as i32 <= alpha {
                    return Some(stand);
                }
            }
        }

        // LMP : plafond de coups silencieux explorés à faible profondeur.
        let quiet_cap: usize = if self.flags.lmp && depth <= 4 { 6 + 4 * depth as usize } else { usize::MAX };

        // Tri par clé décroissante : tampon statique si le fanout est petit.
        let result = if n <= FBUF {
            let mut keys = [0i32; FBUF];
            let mut idx = [0usize; FBUF];
            for i in 0..n {
                keys[i] = self.move_key(s, &moves[i], tt_mv, ply);
                idx[i] = i;
            }
            idx[..n].sort_by_key(|&i| std::cmp::Reverse(keys[i]));
            self.loop_moves(s, &moves, &idx[..n], depth, alpha, beta, ply, quiet_cap)
        } else {
            let mut idx: Vec<usize> = (0..n).collect();
            idx.sort_by_key(|&i| std::cmp::Reverse(self.move_key(s, &moves[i], tt_mv, ply)));
            self.loop_moves(s, &moves, &idx, depth, alpha, beta, ply, quiet_cap)
        };

        let (best, best_packed) = result?;

        if self.flags.ttx {
            let bound = if best >= beta {
                1
            } else if best > alpha0 {
                0
            } else {
                2
            };
            self.tt.store(key, best, depth.max(0) as u8, bound, best_packed);
        }
        Some(best)
    }

    /// Boucle de coups d'un nœud interne : PVS via `try_move`, LMP déjà tranché,
    /// killers/history mis à jour à la coupure. Rend (best, coup) ; None si arrêt.
    #[allow(clippy::too_many_arguments)]
    fn loop_moves(
        &mut self,
        s: &State,
        moves: &[Move],
        order: &[usize],
        depth: i8,
        mut alpha: i32,
        beta: i32,
        ply: usize,
        quiet_cap: usize,
    ) -> Option<(i32, u32)> {
        let mut best = -INF - 1;
        let mut best_packed = NO_MOVE;
        let mut searched: usize = 0;
        let mut quiet = 0usize;
        for &mi in order {
            let m = moves[mi];
            if m.kind == MoveKind::Move {
                quiet += 1;
                if quiet > quiet_cap {
                    continue;
                }
            }
            match self.try_move(s, &m, depth, alpha, beta, ply, searched, true) {
                None => return None,
                Some(sc) => {
                    searched += 1;
                    if sc > best {
                        best = sc;
                        best_packed = pack_move(&m);
                    }
                    if self.flags.hb && m.kind == MoveKind::Move && searched > 1 && sc <= alpha {
                        let p = s.piece(m.piece);
                        let h = &mut self.history[kind3(p.kind)][p.sq as usize][m.to as usize];
                        *h = h.saturating_sub((depth as i32 * depth as i32) as u32 / 4);
                    }
                    if best > alpha {
                        alpha = best;
                        if alpha >= beta {
                            if m.kind == MoveKind::Move {
                                self.bump_history(s, &m, ply, depth);
                                if self.flags.cm {
                                    let p = s.piece(m.piece);
                                    let k = kind3(p.kind) * 2 + (p.owner as usize - 1);
                                    if ply >= 1 { self.cmoves[k][p.sq as usize] = pack_move(&m); }
                                }
                                if self.flags.ch && ply >= 1 {
                                    let pq = self.prev_quiet[ply - 1];
                                    if pq != NO_MOVE && pq & 0x30000 == 0 {
                                        let prev_to = (pq & 0xFF) as usize;
                                        let prev_kind = ((pq >> 8) & 0xFF) as usize;
                                        let p = s.piece(m.piece);
                                        if prev_kind < 3 {
                                            let h = &mut self.chist[0][kind3(p.kind)][m.to as usize][prev_kind][prev_to];
                                            *h = h.saturating_add((depth as i32 * depth as i32) as u16).min(2048);
                                        }
                                    }
                                }
                            }
                            break;
                        }
                    }
                }
            }
        }
        Some((best, best_packed))
    }

    fn quiesce(&mut self, s: &State, mut alpha: i32, beta: i32, ply: usize) -> Option<i32> {
        if s.over || ply >= MAX_PLY {
            return Some(self.eval_node(s, s.to_move));
        }
        if !self.node_tick() {
            return None;
        }
        let stand = self.eval_node(s, s.to_move);
        if stand >= beta {
            return Some(stand);
        }
        if stand > alpha {
            alpha = stand;
        }
        let moves = legal_moves(s);
        let mut buf = [Move::PASS; 64];
        let mut extra: Vec<Move> = Vec::new();
        let mut n = 0usize;
        for m in moves {
            if matches!(m.kind, MoveKind::Exit | MoveKind::Attack) {
                if n < 64 {
                    buf[n] = m;
                } else {
                    extra.push(m);
                }
                n += 1;
            }
        }
        let mut idx = [0usize; 64];
        let base = n.min(64);
        for i in 0..base {
            idx[i] = i;
        }
        idx[..base].sort_by_key(|&i| std::cmp::Reverse(self.move_key(s, &buf[i], NO_MOVE, ply)));
        for i in 0..n {
            let m = if i < 64 { buf[idx[i]] } else { extra[i - 64] };
            // Delta pruning : gain maximal insuffisant pour atteindre alpha.
            let gain = if m.kind == MoveKind::Attack {
                let t = s.piece(s.at(m.to).unwrap_or(0));
                self.cfg.base * t.kind.strength() as i32
            } else {
                let p = s.piece(m.piece);
                self.cfg.score * p.kind.strength() as i32
            };
            if stand + gain + self.cfg.delta <= alpha {
                continue;
            }
            let child = apply_move(s, &m);
            match self.quiesce(&child, -beta, -alpha, ply + 1) {
                None => return None,
                Some(sc) => {
                    let sc = -sc;
                    if sc >= beta {
                        return Some(sc);
                    }
                    if sc > alpha {
                        alpha = sc;
                    }
                }
            }
        }
        Some(alpha)
    }

    /// Recherche complète d'un coup : approfondissement itératif, tri racine par
    /// scores de l'itération précédente, fenêtres d'aspiration optionnelles.
    /// Rend (meilleur coup, score, profondeur complètement cherchée).
    pub fn search(&mut self, s: &State, max_depth: u8, stop: fn() -> bool) -> (Move, i32, u8) {
        self.stop = stop;
        self.nodes = 0;
        self.aborted = false;
        self.killers = [[NO_MOVE; 3]; MAX_PLY + 2];
        self.history = [[[0; SQUARES]; SQUARES]; 3];
        self.last_ply = s.ply;

        let moves = legal_moves(s);
        if moves.is_empty() {
            self.total_nodes += self.nodes;
            return (Move::PASS, 0, 0);
        }
        let n = moves.len().min(ROOT_CAP);
        let mut best_move = moves[0];
        let mut best_score = 0;
        let mut completed = 0u8;
        let mut scores = [0i32; ROOT_CAP];
        let mut order: Vec<usize> = (0..n).collect();
        order.sort_by_key(|&i| std::cmp::Reverse(self.root_key(s, &moves[i])));

        'deepening: for depth in 1..=max_depth.min(MAX_TURNS) {
            let d = depth as i8;
            // Fenêtre d'aspiration (une re-recherche pleine si elle échoue).
            let mut attempt = 0;
            let mut accepted: Option<(usize, i32)> = None;
            while attempt < 2 {
                let (a0, b0) = match attempt {
                    0 if self.flags.aspiration && depth >= 5 => (best_score - 60, best_score + 60),
                    _ => (-INF, INF),
                };
                let mut alpha = a0;
                let mut best_i: Option<usize> = None;
                let mut best_sc = -INF - 1;
                let mut searched = 0usize;
                let mut aborted = false;
                for &mi in &order {
                    let m = moves[mi];
                    let cd = (d - 1 + if m.kind == MoveKind::Exit { 1 } else { 0 }).max(0);
                    let child = apply_move(s, &m);
                    let res = if searched == 0 {
                        self.negamax(&child, cd, -b0, -alpha, 1)
                    } else {
                        let probe = self.negamax(&child, cd, -alpha - 1, -alpha, 1);
                        match probe {
                            None => None,
                            Some(sc) if sc > alpha => self.negamax(&child, cd, -b0, -alpha, 1),
                            Some(sc) => Some(sc),
                        }
                    };
                    let sc = match res {
                        None => {
                            aborted = true;
                            break;
                        }
                        Some(v) => -v,
                    };
                    scores[mi] = sc;
                    searched += 1;
                    if sc > best_sc {
                        best_sc = sc;
                        best_i = Some(mi);
                    }
                    if sc > alpha {
                        alpha = sc;
                    }
                }
                if aborted {
                    break 'deepening;
                }
                if attempt == 0 && (best_sc <= a0 || best_sc >= b0) {
                    attempt += 1;
                    continue;
                }
                accepted = best_i.map(|i| (i, best_sc));
                break;
            }
            if let Some((i, sc)) = accepted {
                best_move = moves[i];
                best_score = sc;
                completed = depth;
                self.root_scores = scores;
                // Re-tri : les meilleurs d'abord, ordre stable.
                order.sort_by_key(|&mi| std::cmp::Reverse(self.root_scores[mi]));
                // Concentration : aux grandes profondeurs, ne garder que le top 4.
                if self.flags.rn && depth >= 6 && n > 4 {
                    order.truncate(4);
                }
            }
        }

        self.last_depth = completed;
        self.total_nodes += self.nodes;
        (best_move, best_score, completed)
    }
}
