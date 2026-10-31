//! Format binaire d'un tour de bot de l'arène : ce que l'hôte (`traversal-arena`) écrit
//! dans la mémoire du module WebAssembly avant chaque coup, et que le SDK
//! (`traversal-bot`) relit. Cet en-tête fait office de spécification pour les bots écrits
//! dans d'autres langages.
//!
//! Petit-boutiste, version 1 :
//!
//! ```text
//! 0    u8        version (1)
//! 1    u8        camp au trait (1 = rouge, 2 = bleu) : c'est le bot
//! 2    u16       demi-coups joués
//! 4    u16       limite de demi-coups
//! 6    u8 × 2    scores rouge, bleu
//! 8    u8        pièces sorties (tous camps)
//! 9    u8        nombre de pièces n
//! 10   u64       graine aléatoire de ce coup
//! 18   u64       fuel accordé pour ce coup
//! 26   u8 × 100  relief (case = y × 10 + x)
//! 126  n × 3     pièces : camp, type (0 Explorateur, 1 Avant-garde, 2 Sentinelle), case
//! ..   u16       nombre de coups légaux m
//! ..   m × 8     coups : pièce, type (0 déplacement, 1 attaque, 2 sortie, 3 passe), case,
//!                spécial (0, 1 diagonale, 2 bond), coût, attaque, défense, 0
//! ```
//!
//! Une pièce est désignée par son indice dans la liste des pièces ; 255 = aucune (passe),
//! et aucune case (sortie, passe). Le bot répond par l'indice du coup choisi.

use crate::config::{Kind, SQUARES};
use crate::movegen::{Move, MoveKind, Special};
use crate::state::{Piece, State, MAX_PIECES};

pub const VERSION: u8 = 1;
/// Taille minimale du tampon que le bot met à disposition (`traversal_buffer`).
pub const BUFFER_SIZE: usize = 16 * 1024;
const HEADER: usize = 26 + SQUARES;

/// Un tour de bot, décodé.
#[derive(Clone, Debug)]
pub struct Turn {
    pub state: State,
    pub moves: Vec<Move>,
    pub seed: u64,
    pub fuel: u64,
}

fn kind_code(k: Kind) -> u8 {
    match k {
        Kind::Explorer => 0,
        Kind::Vanguard => 1,
        Kind::Sentinel => 2,
    }
}

fn move_code(k: MoveKind) -> u8 {
    match k {
        MoveKind::Move => 0,
        MoveKind::Attack => 1,
        MoveKind::Exit => 2,
        MoveKind::Pass => 3,
    }
}

fn special_code(s: Special) -> u8 {
    match s {
        Special::None => 0,
        Special::Diag => 1,
        Special::Leap => 2,
    }
}

pub fn encode(state: &State, moves: &[Move], seed: u64, fuel: u64) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER + 3 * MAX_PIECES + 2 + 8 * moves.len());
    out.push(VERSION);
    out.push(state.to_move);
    out.extend_from_slice(&state.ply.to_le_bytes());
    out.extend_from_slice(&state.move_limit.to_le_bytes());
    out.extend_from_slice(&state.scores);
    out.push(state.finished);
    out.push(state.pieces().len() as u8);
    out.extend_from_slice(&seed.to_le_bytes());
    out.extend_from_slice(&fuel.to_le_bytes());
    out.extend_from_slice(&state.heights);
    for p in state.pieces() {
        out.extend_from_slice(&[p.owner, kind_code(p.kind), p.sq]);
    }
    out.extend_from_slice(&(moves.len() as u16).to_le_bytes());
    for m in moves {
        out.extend_from_slice(&[m.piece, move_code(m.kind), m.to, special_code(m.special), m.cost, m.attack, m.defense, 0]);
    }
    out
}

pub fn decode(buf: &[u8]) -> Option<Turn> {
    let u16_at = |i: usize| Some(u16::from_le_bytes(buf.get(i..i + 2)?.try_into().ok()?));
    let u64_at = |i: usize| Some(u64::from_le_bytes(buf.get(i..i + 8)?.try_into().ok()?));
    if *buf.first()? != VERSION {
        return None;
    }
    let n = *buf.get(9)? as usize;
    if n > MAX_PIECES {
        return None;
    }
    let heights: [u8; SQUARES] = buf.get(26..HEADER)?.try_into().ok()?;
    let mut pieces = Vec::with_capacity(n);
    for c in buf.get(HEADER..HEADER + 3 * n)?.chunks_exact(3) {
        let kind = [Kind::Explorer, Kind::Vanguard, Kind::Sentinel].get(c[1] as usize).copied()?;
        if c[2] as usize >= SQUARES {
            return None;
        }
        pieces.push(Piece { owner: c[0], kind, sq: c[2] });
    }
    let at = HEADER + 3 * n;
    let m = u16_at(at)? as usize;
    let mut moves = Vec::with_capacity(m);
    for c in buf.get(at + 2..at + 2 + 8 * m)?.chunks_exact(8) {
        let kind = [MoveKind::Move, MoveKind::Attack, MoveKind::Exit, MoveKind::Pass].get(c[1] as usize).copied()?;
        let special = [Special::None, Special::Diag, Special::Leap].get(c[3] as usize).copied()?;
        moves.push(Move { piece: c[0], kind, to: c[2], cost: c[4], special, attack: c[5], defense: c[6] });
    }
    let state = State::new(heights, &pieces, buf[1], u16_at(2)?, [buf[6], buf[7]], buf[8], u16_at(4)?);
    Some(Turn { state, moves, seed: u64_at(10)?, fuel: u64_at(18)? })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{apply_move, legal_moves, new_game};

    #[test]
    fn roundtrip() {
        let mut s = new_game([2; SQUARES]);
        for _ in 0..7 {
            let m = legal_moves(&s)[3];
            s = apply_move(&s, &m);
        }
        let moves = legal_moves(&s);
        let buf = encode(&s, &moves, 42, 1_000_000);
        assert!(buf.len() <= BUFFER_SIZE);
        let t = decode(&buf).expect("décodage");
        assert_eq!((t.seed, t.fuel, t.moves), (42, 1_000_000, moves));
        assert_eq!((t.state.pieces(), t.state.to_move, t.state.ply), (s.pieces(), s.to_move, s.ply));
        assert_eq!(legal_moves(&t.state), legal_moves(&s));
        assert!(decode(&buf[..buf.len() - 1]).is_none());
    }
}
