//! Évaluation : squelette du bot de référence, élargi (constantes validées en miroir)
//! et termes optionnels activés par `Flags` (soutien, menace, terrain barré).

use traversal_bot::core::config::{SIZE, SQUARES};
use traversal_bot::core::movegen::{neighbors_of, rows_to_go, step_cost};
use traversal_bot::core::{legal_moves, Kind, MoveKind, State};

use crate::engine::{Flags, WIN};

const ALLY_TOLL: u16 = 2;
const SOFT_TOLL: u16 = 2;
const DETOUR: i32 = 6;
/// Capacité du tas binaire (pile) des Dijkstra de terrain barré.
const HCAP: usize = 512;

#[derive(Clone, Copy)]
pub struct EvalCfg {
    /// points de score par point marqué
    pub score: i32,
    /// valeur de base par point de Force
    pub base: i32,
    /// portée du bonus de progression (en tours)
    pub prog_span: i32,
    pub prog_a: i32,
    pub prog_b: i32,
    /// prime de course de Sentinelle (portée en tours, poids par tour²)
    pub sent_span: i32,
    pub sent_w: i32,
    /// course des Sentinelles : bonus/malus et horizon (en tours)
    pub race: i32,
    pub race_h: i32,
    /// prime d'avoir le trait
    pub tempo: i32,
    /// marge d'élagage delta (quiescence)
    pub delta: i32,
    /// marge d'élagage futility (par profondeur)
    pub futility: i32,
    /// table de progression apprise : PROG[kind][tours] (0..=15), si `ptab`
    pub ptab: [[i32; 16]; 3],
}

impl Default for EvalCfg {
    fn default() -> EvalCfg {
        EvalCfg {
            score: 100,
            base: 30,
            prog_span: 13,
            prog_a: 4,
            prog_b: 3,
            sent_span: 8,
            sent_w: 5,
            race: 200,
            race_h: 5,
            tempo: 8,
            delta: 60,
            futility: 140,
            ptab: ptab_from_linear(13, 4, 3),
        }
    }
}

/// Table PROG[kind][t] équivalente à la formule (span − t)+ · (a + b·F).
pub fn ptab_from_linear(span: i32, a: i32, b: i32) -> [[i32; 16]; 3] {
    let mut t = [[0i32; 16]; 3];
    for k in 0..3 {
        let f = (k + 1) as i32;
        for tt in 0..16 {
            t[k][tt] = (span - tt as i32).max(0) * (a + b * f);
        }
    }
    t
}

#[inline]
fn turns_for(cost: i32, kind: Kind) -> i32 {
    let m = kind.movement() as i32;
    (cost + m - 1) / m
}

/// Terrain barré : coût pour sortir en tenant compte des pièces — une Sentinelle
/// adverse est un mur, une autre pièce adverse se franchise (SOFT_TOLL), une
/// alliée doit s'écarter (ALLY_TOLL). Tas binaire sur pile (HCAP cases).
fn blocked_costs(s: &State, owner: u8) -> [u16; SQUARES] {
    let row = if owner == 1 { SIZE - 1 } else { 0 };
    let other = |sq: u8| s.at(sq).map(|i| s.piece(i)).filter(|p| p.owner != owner);
    let wall = |sq: u8| other(sq).is_some_and(|p| p.kind == Kind::Sentinel);
    let ally = |sq: u8| s.at(sq).is_some_and(|i| s.piece(i).owner == owner);

    let mut dist = [u16::MAX; SQUARES];
    // Tas binaire minimier : (dist << 8) | case, dist < 1<<8 suffit (coûts petits).
    let mut heap = [0u32; HCAP];
    let mut hn = 0usize;
    let mut push = |heap: &mut [u32; HCAP], hn: &mut usize, v: u32| {
        let mut i = *hn;
        if i >= HCAP {
            return;
        }
        heap[i] = v;
        *hn += 1;
        while i > 0 {
            let p = (i - 1) / 2;
            if heap[p] <= heap[i] {
                break;
            }
            heap.swap(p, i);
            i = p;
        }
    };
    let pop = |heap: &mut [u32; HCAP], hn: &mut usize| -> Option<u32> {
        if *hn == 0 {
            return None;
        }
        let top = heap[0];
        *hn -= 1;
        heap[0] = heap[*hn];
        let n = *hn;
        let mut i = 0usize;
        loop {
            let (l, r) = (2 * i + 1, 2 * i + 2);
            let mut m = i;
            if l < n && heap[l] < heap[m] {
                m = l;
            }
            if r < n && heap[r] < heap[m] {
                m = r;
            }
            if m == i {
                break;
            }
            heap.swap(m, i);
            i = m;
        }
        Some(top)
    };

    for x in 0..SIZE {
        let sq = (row * SIZE + x) as u8;
        if !wall(sq) {
            dist[sq as usize] = 1;
            push(&mut heap, &mut hn, (1u32 << 8) | sq as u32);
        }
    }
    while let Some(v) = pop(&mut heap, &mut hn) {
        let (d, cur) = ((v >> 8) as u16, (v & 0xFF) as u8);
        if d > dist[cur as usize] {
            continue;
        }
        // Coût pour ENTRER sur `cur` depuis une voisine.
        let enter = if ally(cur) {
            ALLY_TOLL
        } else if other(cur).is_some() {
            SOFT_TOLL
        } else {
            0
        };
        for &nb in neighbors_of(cur) {
            if wall(nb) {
                continue;
            }
            let nd = d + step_cost(&s.heights, nb, cur) as u16 + enter;
            if nd < dist[nb as usize] {
                dist[nb as usize] = nd;
                push(&mut heap, &mut hn, ((nd as u32) << 8) | nb as u32);
            }
        }
    }
    dist
}

/// Course des Sentinelles (R8.2) : personne n'est sorti — qui sortira sa Sentinelle
/// en premier, sachant qu'une sortie adverse quelconque annule la victoire immédiate ?
fn sentinel_race(s: &State, side: u8, first: &[i32; 2], sentinel: &[i32; 2], cfg: &EvalCfg) -> i32 {
    let plies = |owner: u8, turns: i32| match turns {
        i32::MAX => i32::MAX,
        t if s.to_move == owner => 2 * t - 1,
        t => 2 * t,
    };
    let wins = |me: u8| {
        let them = if me == 1 { 2 } else { 1 };
        let mine = plies(me, sentinel[me as usize - 1]);
        mine < plies(them, first[them as usize - 1]) && sentinel[me as usize - 1] <= cfg.race_h
    };
    let other = if side == 1 { 2 } else { 1 };
    match (wins(side), wins(other)) {
        (true, false) => cfg.race,
        (false, true) => -cfg.race,
        _ => 0,
    }
}

/// Course graduée (rdif) : différentiel de temps de sortie Sentinelle, au lieu
/// du bonus binaire. Rendu nul si une sortie quelconque a déjà eu lieu.
fn sentinel_race_grad(s: &State, side: u8, sentinel: &[i32; 2], cfg: &EvalCfg) -> i32 {
    if s.finished != 0 {
        return 0;
    }
    let plies = |owner: u8, turns: i32| match turns {
        i32::MAX => i32::MAX,
        t if s.to_move == owner => 2 * t - 1,
        t => 2 * t,
    };
    let other = if side == 1 { 2 } else { 1 };
    let mine = plies(side, sentinel[side as usize - 1]);
    let theirs = plies(other, sentinel[other as usize - 1]);
    if mine >= i32::MAX / 2 || theirs >= i32::MAX / 2 {
        return 0;
    }
    let d = theirs as i64 - mine as i64;
    // avance en plies → prime proportionnelle, cap ±race
    (d as i32 * (cfg.race / 4)).clamp(-cfg.race, cfg.race)
}

/// Le camp au trait peut-il sortir une pièce tout de suite ?
/// (garde du null-move : si l'adversaire peut sortir, passer est interdit.)
pub fn can_exit_now(s: &State) -> bool {
    if s.over {
        return false;
    }
    legal_moves(s).iter().any(|m| m.kind == MoveKind::Exit)
}

pub fn eval(s: &State, side: u8, cfg: &EvalCfg, costs: &[[u16; SQUARES]; 2], flags: &Flags) -> i32 {
    if s.over {
        return match s.winner {
            None => 0,
            Some(w) if w == side => WIN - s.ply as i32,
            Some(_) => -(WIN - s.ply as i32),
        };
    }
    let other = if side == 1 { 2 } else { 1 };
    let mut total = cfg.score * (s.scores[side as usize - 1] as i32 - s.scores[other as usize - 1] as i32);
    let nobody_out = s.finished == 0;

    let bc = if flags.blocked > 0 {
        [blocked_costs(s, 1), blocked_costs(s, 2)]
    } else {
        [[u16::MAX; SQUARES], [u16::MAX; SQUARES]]
    };

    let mut first = [i32::MAX; 2];
    let mut sentinel = [i32::MAX; 2];

    for p in s.pieces() {
        let o = (p.owner - 1) as usize;
        let strength = p.kind.strength() as i32;
        let free = costs[o][p.sq as usize] as i32;
        let cost = if flags.blocked > 0 {
            let with = (bc[o][p.sq as usize] as i32).min(free + DETOUR);
            (free * (100 - flags.blocked) + with * flags.blocked + 50) / 100
        } else {
            free
        };
        let t = turns_for(cost, p.kind);

        let prog = if flags.ptab {
            cfg.ptab[strength as usize - 1][(t as usize).min(15)]
        } else {
            (cfg.prog_span - t).max(0) * (cfg.prog_a + cfg.prog_b * strength)
        };
        let mut v = cfg.base * strength + prog;
        if nobody_out && p.kind == Kind::Sentinel {
            v += (cfg.sent_span - t).max(0).pow(2) * cfg.sent_w;
        }

        // Soutien : alliés adjacents (cap 3).
        if flags.support_w != 0 {
            let mut allies = 0;
            for &nb in neighbors_of(p.sq) {
                if let Some(j) = s.at(nb) {
                    if s.piece(j).owner == p.owner {
                        allies += 1;
                    }
                }
            }
            v += flags.support_w * allies.min(3);
        }

        // Pièce menacée : un voisin ennemi adjacent gagnerait le combat.
        if flags.hang_w != 0 && !flags.hangtab {
            'threat: for &nb in neighbors_of(p.sq) {
                let Some(j) = s.at(nb) else { continue };
                let e = s.piece(j);
                if e.owner == p.owner {
                    continue;
                }
                let (ha, ht) = (s.heights[e.sq as usize] as i32, s.heights[p.sq as usize] as i32);
                let mut atk = e.kind.strength() as i32 + (ha - ht).signum();
                let mut def = strength;
                for &q in neighbors_of(p.sq) {
                    if let Some(k) = s.at(q) {
                        let a = s.piece(k);
                        if a.owner == e.owner && q != e.sq {
                            atk += a.kind.strength() as i32;
                        }
                        if a.owner == p.owner && q != p.sq {
                            def += 1;
                        }
                    }
                }
                if atk >= def {
                    v -= if flags.hangd && def > strength {
                        flags.hang_w * strength / 2
                    } else {
                        flags.hang_w * strength
                    };
                    break 'threat;
                }
            }
        }
        // Variante tabulaire du hang (hangtab) : pénalité par type testée en A/B.
        if flags.hangtab && flags.hang_w == 0 {
            'threat2: for &nb in neighbors_of(p.sq) {
                let Some(j) = s.at(nb) else { continue };
                let e = s.piece(j);
                if e.owner == p.owner {
                    continue;
                }
                let (ha, ht) = (s.heights[e.sq as usize] as i32, s.heights[p.sq as usize] as i32);
                let mut atk = e.kind.strength() as i32 + (ha - ht).signum();
                let mut def = strength;
                for &q in neighbors_of(p.sq) {
                    if let Some(k) = s.at(q) {
                        let a = s.piece(k);
                        if a.owner == e.owner && q != e.sq {
                            atk += a.kind.strength() as i32;
                        }
                        if a.owner == p.owner && q != p.sq {
                            def += 1;
                        }
                    }
                }
                if atk >= def {
                    v -= [25, 35, 40][(strength - 1) as usize];
                    break 'threat2;
                }
            }
        }

        total += if p.owner == side { v } else { -v };
        first[o] = first[o].min(t);
        if p.kind == Kind::Sentinel {
            sentinel[o] = sentinel[o].min(t);
        }
    }

    if nobody_out {
        if flags.rdif {
            total += sentinel_race_grad(s, side, &sentinel, cfg);
        } else {
            total += sentinel_race(s, side, &first, &sentinel, cfg);
        }
    }
    // Sûreté de la Sentinelle : pénalité si un ennemi capable de la capturer est proche.
    if flags.ssafe && nobody_out {
        for p in s.pieces() {
            if p.kind == Kind::Sentinel && p.owner == side {
                let mut worst = 0i32;
                for e in s.pieces() {
                    if e.owner != side {
                        let dist = (e.sq / 10).abs_diff(p.sq / 10) + (e.sq % 10).abs_diff(p.sq % 10);
                        if e.kind.strength() + 3 > p.kind.strength() {
                            // menace croissante quand la distance tombe
                            worst = worst.max(20 - 4 * dist as i32);
                        }
                    }
                }
                total -= worst.max(0) * 3;
            }
        }
    }
    // Contrôle des zones de sortie : prime par pièce à ≤ 3 tours de la sortie.
    if flags.zc && nobody_out {
        for p in s.pieces() {
            let o = (p.owner - 1) as usize;
            let t = turns_for(costs[o][p.sq as usize] as i32, p.kind);
            if t <= 3 {
                let v = (4 - t) * 15;
                total += if p.owner == side { v } else { -v };
            }
        }
    }
    // Matériel : différentiel de Force des pièces vivantes.
    if flags.mat {
        let mut m = 0i32;
        for p in s.pieces() {
            m += if p.owner == side { p.kind.strength() as i32 } else { -(p.kind.strength() as i32) };
        }
        total += 12 * m;
    }
    // Contrôle du relief (hi) : la hauteur ne paie qu'en interaction de combat.
    // Pour chaque paire de pièces adjacentes adverses, le camp en hauteur
    // gagne l'avantage tactique (atk/def ± Δh dans la règle de combat).
    if flags.hi {
        for p in s.pieces() {
            for &nb in neighbors_of(p.sq) {
                if let Some(j) = s.at(nb) {
                    let e = s.piece(j);
                    if e.owner == p.owner {
                        continue;
                    }
                    let dh = s.heights[p.sq as usize] as i32 - s.heights[nb as usize] as i32;
                    if dh > 0 {
                        // p domine e : avantage pour le camp de p
                        let v = dh * 75;
                        total += if p.owner == side { v } else { -v };
                    }
                }
            }
        }
    }
    // Adjudication R8.3 : en fin de partie sans sortie, le départage se joue
    // sur la pièce la plus avancée (rangées restantes, puis Force). Au-delà de
    // ply ≥ 120, valoriser l'avance sur cette clé exacte.
    if flags.adj && s.finished == 0 && s.ply >= 120 {
        let mut best: [i32; 2] = [i32::MAX, i32::MAX];
        for p in s.pieces() {
            let o = (p.owner - 1) as usize;
            // clé de départage : rangées × 10, Force départage
            let k = rows_to_go(p.owner, p.sq) as i32 * 10 - p.kind.strength() as i32;
            best[o] = best[o].min(k);
        }
        let mine = best[side as usize - 1];
        let theirs = best[if side == 1 { 1 } else { 0 }];
        if mine < i32::MAX && theirs < i32::MAX {
            // avance en unités de départage × poids croissant avec la fin
            let urgency = ((s.ply as i32 - 120) / 10).clamp(0, 8) as i32;
            total += (theirs - mine) * (6 + 2 * urgency);
        }
    }
    total += if s.to_move == side { cfg.tempo } else { -cfg.tempo };
    total
}
