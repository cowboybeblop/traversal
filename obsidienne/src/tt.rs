//! Table de transposition : 2^20 entrées de 24 octets, remplacement systématique
//! sauf même clé plus profonde (la profondeur domine le « toujours remplacer »).

use traversal_bot::core::movegen::Move;

/// Coup absent (poids 20 bits : case 8 + pièce 8 + type 2, la passe = 0xFF FF 3).
pub const NO_MOVE: u32 = u32::MAX;

const N: usize = 1 << 20;
const MASK: u32 = (N - 1) as u32;

/// Coup compacté (pièce, type, case) pour TT, killers et history.
#[inline]
pub fn pack_move(m: &Move) -> u32 {
    let kind = match m.kind {
        traversal_bot::core::movegen::MoveKind::Move => 0u32,
        traversal_bot::core::movegen::MoveKind::Attack => 1,
        traversal_bot::core::movegen::MoveKind::Exit => 2,
        traversal_bot::core::movegen::MoveKind::Pass => 3,
    };
    m.to as u32 | (m.piece as u32) << 8 | kind << 16
}

#[derive(Clone, Copy)]
pub struct Slot {
    pub key: u64,
    pub score: i32,
    pub depth: u8,
    /// 0 = exacte, 1 = borne inférieure (fail-high), 2 = borne supérieure.
    pub bound: u8,
    pub mv: u32,
    pad: u32,
}

pub struct TT {
    slots: Vec<Slot>,
}

impl TT {
    pub fn new() -> TT {
        TT { slots: vec![Slot { key: 0, score: 0, depth: 0, bound: 0, mv: NO_MOVE, pad: 0 }; N] }
    }

    #[inline]
    pub fn probe(&self, key: u64) -> Option<Slot> {
        let s = &self.slots[(key as u32 & MASK) as usize];
        (s.key == key).then_some(*s)
    }

    #[inline]
    pub fn store(&mut self, key: u64, score: i32, depth: u8, bound: u8, mv: u32) {
        let i = (key as u32 & MASK) as usize;
        let s = &mut self.slots[i];
        if s.key == key {
            // même position : garder le plus profond, et le coup si on n'en a pas.
            if s.depth > depth {
                return;
            }
            if mv != NO_MOVE {
                s.mv = mv;
            }
        } else {
            s.mv = mv;
        }
        s.key = key;
        s.score = score;
        s.depth = depth;
        s.bound = bound;
    }
}

/// Le coup compacté est-il celui-ci ? (diagnostics)
pub fn is_packed(m: &Move, packed: u32) -> bool {
    pack_move(m) == packed
}
