//! Génération des coups légaux (R4 à R7), identique à celle du moteur du serveur.
//!
//! On ne garde que ce dont le bot a besoin (pièce, type, case, coût, forces du combat) :
//! les chemins et le détail des forces restent calculés par le moteur du serveur.

use crate::config::{Kind, LEAP_OVER_PIECES, SIZE, SPECIAL_MOVES_CAN_ATTACK, SQUARES, VANGUARD_CHARGE_COST};
use crate::state::{State, NONE};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum MoveKind {
    Move,
    Attack,
    Exit,
    Pass,
}

impl MoveKind {
    pub fn name(self) -> &'static str {
        match self {
            MoveKind::Move => "move",
            MoveKind::Attack => "attack",
            MoveKind::Exit => "exit",
            MoveKind::Pass => "pass",
        }
    }

    pub fn parse(s: &str) -> Option<MoveKind> {
        match s {
            "move" => Some(MoveKind::Move),
            "attack" => Some(MoveKind::Attack),
            "exit" => Some(MoveKind::Exit),
            "pass" => Some(MoveKind::Pass),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Special {
    None,
    Diag,
    Leap,
}

impl Special {
    pub fn name(self) -> Option<&'static str> {
        match self {
            Special::None => None,
            Special::Diag => Some("diag"),
            Special::Leap => Some("leap"),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub struct Move {
    /// Indice de la pièce (`NONE` pour une passe).
    pub piece: u8,
    pub kind: MoveKind,
    /// Case d'arrivée (`NONE` pour une sortie ou une passe).
    pub to: u8,
    pub cost: u8,
    pub special: Special,
    /// Forces du combat (attaques seulement).
    pub attack: u8,
    pub defense: u8,
}

impl Move {
    pub const PASS: Move = Move {
        piece: NONE,
        kind: MoveKind::Pass,
        to: NONE,
        cost: 0,
        special: Special::None,
        attack: 0,
        defense: 0,
    };
}

/// Voisins orthogonaux de chaque case (même ordre que le moteur du serveur : y+1, x+1,
/// y-1, x-1).
pub static NEIGHBORS: [[u8; 4]; SQUARES] = neighbors();
pub static NEIGHBOR_COUNT: [u8; SQUARES] = neighbor_count();

const fn neighbors() -> [[u8; 4]; SQUARES] {
    let mut out = [[NONE; 4]; SQUARES];
    let mut sq = 0;
    while sq < SQUARES {
        let (x, y) = ((sq % SIZE) as i32, (sq / SIZE) as i32);
        let cands = [(x, y + 1), (x + 1, y), (x, y - 1), (x - 1, y)];
        let mut k = 0;
        let mut n = 0;
        while k < 4 {
            let (nx, ny) = cands[k];
            if nx >= 0 && nx < SIZE as i32 && ny >= 0 && ny < SIZE as i32 {
                out[sq][n] = (ny * SIZE as i32 + nx) as u8;
                n += 1;
            }
            k += 1;
        }
        sq += 1;
    }
    out
}

const fn neighbor_count() -> [u8; SQUARES] {
    let all = neighbors();
    let mut out = [0u8; SQUARES];
    let mut sq = 0;
    while sq < SQUARES {
        let mut n = 0;
        while n < 4 && all[sq][n] != NONE {
            n += 1;
        }
        out[sq] = n as u8;
        sq += 1;
    }
    out
}

#[inline]
pub fn neighbors_of(sq: u8) -> &'static [u8] {
    &NEIGHBORS[sq as usize][..NEIGHBOR_COUNT[sq as usize] as usize]
}

/// R4.3 : 1 sur le plat, sinon |Δh| + 1.
#[inline]
pub fn step_cost(heights: &[u8; SQUARES], a: u8, b: u8) -> u8 {
    let dh = heights[a as usize].abs_diff(heights[b as usize]);
    if dh == 0 {
        1
    } else {
        dh + 1
    }
}

pub fn exit_row(owner: u8) -> u8 {
    if owner == 1 {
        (SIZE - 1) as u8
    } else {
        0
    }
}

pub fn rows_to_go(owner: u8, sq: u8) -> u8 {
    let y = sq / SIZE as u8;
    if owner == 1 {
        (SIZE as u8 - 1) - y
    } else {
        y
    }
}

/// Dijkstra borné : coût minimal vers chaque case vide atteignable (`NONE` sinon).
/// Les coûts sont petits (≤ 5) : une file par coût suffit.
fn reach(state: &State, i: u8) -> [u8; SQUARES] {
    let piece = state.piece(i);
    let budget = piece.kind.movement();
    let mut dist = [NONE; SQUARES];
    dist[piece.sq as usize] = 0;
    let mut buckets: [[u8; SQUARES]; 8] = [[0; SQUARES]; 8];
    let mut counts = [0usize; 8];
    buckets[0][0] = piece.sq;
    counts[0] = 1;
    for d in 0..=budget as usize {
        let mut k = 0;
        while k < counts[d] {
            let cur = buckets[d][k];
            k += 1;
            if dist[cur as usize] as usize != d {
                continue;
            }
            for &nb in neighbors_of(cur) {
                if state.at(nb).is_some() {
                    continue;
                }
                let nd = d + step_cost(&state.heights, cur, nb) as usize;
                if nd <= budget as usize && nd < dist[nb as usize] as usize {
                    dist[nb as usize] = nd as u8;
                    buckets[nd][counts[nd]] = nb;
                    counts[nd] += 1;
                }
            }
        }
    }
    dist
}

/// R7.3 / R7.4 / R7.5 — forces effectives (attaquant, défenseur).
pub fn combat(state: &State, attacker: u8, target: u8, charge: bool) -> (u8, u8) {
    let a = state.piece(attacker);
    let t = state.piece(target);
    let mut attack = a.kind.strength() as i32;
    let (ha, ht) = (state.heights[a.sq as usize], state.heights[t.sq as usize]);
    if ha > ht {
        attack += 1;
    } else if ha < ht {
        attack -= 1;
    }
    let mut defense = t.kind.strength() as i32;
    for &nb in neighbors_of(t.sq) {
        if let Some(j) = state.at(nb) {
            let ally = state.piece(j);
            if ally.owner == a.owner && j != attacker {
                attack += ally.kind.strength() as i32;
            }
            if ally.owner == t.owner {
                defense += 1;
            }
        }
    }
    if charge {
        attack += 1;
    }
    (attack.max(0) as u8, defense as u8)
}

/// Coups d'une pièce, ajoutés à `out`.
pub fn piece_moves(state: &State, i: u8, out: &mut Vec<Move>) {
    let piece = state.piece(i);
    let budget = piece.kind.movement();
    let dist = reach(state, i);
    let start = out.len();
    let mut taken = [false; SQUARES]; // cases déjà atteintes (déplacement ou attaque)

    // R4 Déplacements vers les cases vides (jamais la case de départ).
    for sq in 0..SQUARES as u8 {
        let d = dist[sq as usize];
        if d != NONE && sq != piece.sq {
            out.push(Move { piece: i, kind: MoveKind::Move, to: sq, cost: d, special: Special::None, attack: 0, defense: 0 });
            taken[sq as usize] = true;
        }
    }

    // R7 Attaques : dernière marche vers une case ennemie, coût d'entrée compris.
    let mut best = [NONE; SQUARES];
    for sq in 0..SQUARES as u8 {
        let d = dist[sq as usize];
        if d == NONE {
            continue;
        }
        for &nb in neighbors_of(sq) {
            let Some(j) = state.at(nb) else { continue };
            if state.piece(j).owner == piece.owner {
                continue;
            }
            let cost = d + step_cost(&state.heights, sq, nb);
            if cost <= budget && cost < best[nb as usize] {
                best[nb as usize] = cost;
            }
        }
    }
    for tsq in 0..SQUARES as u8 {
        let cost = best[tsq as usize];
        if cost == NONE {
            continue;
        }
        let target = state.at(tsq).expect("cible");
        let charge = piece.kind == Kind::Vanguard && cost == VANGUARD_CHARGE_COST;
        let (attack, defense) = combat(state, i, target, charge);
        if attack >= defense {
            out.push(Move { piece: i, kind: MoveKind::Attack, to: tsq, cost, special: Special::None, attack, defense });
            taken[tsq as usize] = true;
        }
    }

    // R6 Sortie : atteindre la dernière rangée avec au moins 1 point restant.
    let row = exit_row(piece.owner);
    let mut exit_cost = NONE;
    for x in 0..SIZE as u8 {
        let d = dist[(row * SIZE as u8 + x) as usize];
        if d != NONE && d + 1 <= budget && d < exit_cost {
            exit_cost = d;
        }
    }

    // R5 Coups spéciaux de l'Explorateur (ignorent la hauteur).
    if piece.kind == Kind::Explorer {
        let (x, y) = ((piece.sq % SIZE as u8) as i32, (piece.sq / SIZE as u8) as i32);
        let specials: [(Special, i32, i32); 8] = [
            (Special::Diag, 1, 1),
            (Special::Diag, 1, -1),
            (Special::Diag, -1, 1),
            (Special::Diag, -1, -1),
            (Special::Leap, 0, 2),
            (Special::Leap, 2, 0),
            (Special::Leap, 0, -2),
            (Special::Leap, -2, 0),
        ];
        for (kind, dx, dy) in specials {
            let (nx, ny) = (x + dx, y + dy);
            if !(0..SIZE as i32).contains(&nx) || !(0..SIZE as i32).contains(&ny) {
                continue;
            }
            if kind == Special::Leap && !LEAP_OVER_PIECES {
                let middle = ((y + dy / 2) * SIZE as i32 + (x + dx / 2)) as u8;
                if state.at(middle).is_some() {
                    continue;
                }
            }
            let dest = (ny * SIZE as i32 + nx) as u8;
            if taken[dest as usize] {
                continue; // déjà atteignable normalement : même résultat
            }
            match state.at(dest) {
                None => {
                    out.push(Move { piece: i, kind: MoveKind::Move, to: dest, cost: 0, special: kind, attack: 0, defense: 0 });
                    taken[dest as usize] = true;
                }
                Some(j) if state.piece(j).owner != piece.owner && SPECIAL_MOVES_CAN_ATTACK => {
                    let (attack, defense) = combat(state, i, j, false);
                    if attack >= defense {
                        out.push(Move { piece: i, kind: MoveKind::Attack, to: dest, cost: 0, special: kind, attack, defense });
                        taken[dest as usize] = true;
                    }
                }
                Some(_) => {}
            }
        }
    }

    if exit_cost != NONE {
        out.push(Move { piece: i, kind: MoveKind::Exit, to: NONE, cost: exit_cost + 1, special: Special::None, attack: 0, defense: 0 });
    }
    debug_assert!(out[start..].iter().all(|m| m.piece == i));
}

pub fn legal_moves(state: &State) -> Vec<Move> {
    let mut out = Vec::with_capacity(64);
    if state.over {
        return out;
    }
    for (i, p) in state.pieces().iter().enumerate() {
        if p.owner == state.to_move {
            piece_moves(state, i as u8, &mut out);
        }
    }
    if out.is_empty() {
        // R4.8 [?] Aucun coup : le joueur passe.
        out.push(Move::PASS);
    }
    out
}
