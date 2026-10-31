//! Application des coups et fin de partie (R4.7, R6, R7, R8), identiques au moteur du
//! serveur.

use crate::config::{Kind, FINISHERS_TO_END, SIZE};
use crate::movegen::{legal_moves, rows_to_go, Move, MoveKind};
use crate::state::{Piece, Reason, State};

/// Mise en place symétrique (R3.2) sur les colonnes 2..7, de la pièce la plus rapide au
/// centre à la plus lente sur les côtés : S V E E V S (celle du serveur pour les nouvelles
/// parties, « speed »).
pub fn new_game(heights: [u8; crate::config::SQUARES]) -> State {
    const ORDER: [Kind; 6] = [Kind::Sentinel, Kind::Vanguard, Kind::Explorer, Kind::Explorer, Kind::Vanguard, Kind::Sentinel];
    let mut pieces = Vec::with_capacity(12);
    for (owner, row) in [(1u8, 0usize), (2u8, SIZE - 1)] {
        for (k, kind) in ORDER.iter().enumerate() {
            let col = 2 + k;
            let x = if owner == 1 { col } else { SIZE - 1 - col };
            pieces.push(Piece { owner, kind: *kind, sq: (row * SIZE + x) as u8 });
        }
    }
    State::new(heights, &pieces, 1, 0, [0, 0], 0, crate::config::MOVE_LIMIT)
}

/// R8.3 Départage à la limite de coups.
pub fn adjudicate(state: &State) -> Option<u8> {
    let [s1, s2] = state.scores;
    if s1 > 0 || s2 > 0 {
        return if s1 > s2 {
            Some(1)
        } else if s2 > s1 {
            Some(2)
        } else {
            None
        };
    }
    // Pièce la plus avancée de chaque camp : (rangées à franchir, -Force), le plus petit gagne.
    let mut best: [Option<(u8, i8)>; 2] = [None, None];
    for p in state.pieces() {
        let key = (rows_to_go(p.owner, p.sq), -(p.kind.strength() as i8));
        let slot = &mut best[p.owner as usize - 1];
        if slot.map_or(true, |v| key < v) {
            *slot = Some(key);
        }
    }
    match best {
        [Some(a), Some(b)] if a < b => Some(1),
        [Some(a), Some(b)] if b < a => Some(2),
        [Some(_), None] => Some(1),
        [None, Some(_)] => Some(2),
        _ => None,
    }
}

/// Applique un coup supposé légal.
pub fn apply_move(state: &State, m: &Move) -> State {
    debug_assert!(!state.over, "partie terminée");
    let mut s = state.clone();
    let other = if state.to_move == 1 { 2 } else { 1 };
    match m.kind {
        MoveKind::Pass => {}
        MoveKind::Exit => {
            // R6.2 La pièce quitte le plateau et marque sa Force.
            let p = s.piece(m.piece);
            s.remove(m.piece);
            s.finished += 1;
            s.scores[p.owner as usize - 1] += p.kind.strength();
            if s.finished == 1 && p.kind == Kind::Sentinel {
                (s.over, s.winner, s.reason) = (true, Some(p.owner), Some(Reason::Sentinel)); // R8.2
            } else if s.finished >= FINISHERS_TO_END {
                let [s1, s2] = s.scores;
                s.over = true;
                s.reason = Some(Reason::Score); // R8.1
                s.winner = if s1 > s2 {
                    Some(1)
                } else if s2 > s1 {
                    Some(2)
                } else {
                    None
                };
            }
        }
        MoveKind::Move | MoveKind::Attack => {
            let mut i = m.piece;
            if m.kind == MoveKind::Attack {
                // R7.2 La pièce vaincue sort du plateau, sans marquer.
                let t = s.at(m.to).expect("cible");
                s.remove(t);
                if t < i {
                    i -= 1;
                }
            }
            s.relocate(i, m.to);
        }
    }
    s.to_move = other;
    s.ply += 1;
    if !s.over && s.ply >= s.move_limit {
        (s.over, s.winner, s.reason) = (true, adjudicate(&s), Some(Reason::MoveLimit));
    }
    s
}

/// Le coup légal `(pièce, type, case)`, s'il existe.
pub fn find_move(state: &State, piece: u8, kind: MoveKind, to: u8) -> Option<Move> {
    legal_moves(state).into_iter().find(|m| m.piece == piece && m.kind == kind && m.to == to)
}
