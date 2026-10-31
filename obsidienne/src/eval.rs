//! Évaluation : squelette du bot de référence, élargi (constantes validées en miroir)
//! et termes optionnels activés par `Flags` (soutien, menace, terrain barré).

use traversal_bot::core::config::{SIZE, SQUARES};
use traversal_bot::core::movegen::{neighbors_of, step_cost};
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
        }
    }
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

        let mut v = cfg.base * strength + (cfg.prog_span - t).max(0) * (cfg.prog_a + cfg.prog_b * strength);
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
        if flags.hang_w != 0 {
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
                    v -= flags.hang_w * strength;
                    break 'threat;
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
        total += sentinel_race(s, side, &first, &sentinel, cfg);
    }
    total += if s.to_move == side { cfg.tempo } else { -cfg.tempo };
    total
}
