//! Position de jeu. Les pièces gardent le même ordre que dans le moteur du serveur (une
//! pièce retirée décale les suivantes) : l'indice d'une pièce est ce qui circule entre
//! les deux moteurs, et entre l'arène et les bots.

use crate::config::{Kind, SQUARES};

pub const MAX_PIECES: usize = 12;
/// Case vide (`occ`) / pas de case (`Move::to`).
pub const NONE: u8 = u8::MAX;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub struct Piece {
    /// 1 = rouge (part de y=0), 2 = bleu (part de y=9).
    pub owner: u8,
    pub kind: Kind,
    pub sq: u8,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Reason {
    Score,
    Sentinel,
    MoveLimit,
}

impl Reason {
    pub fn name(self) -> &'static str {
        match self {
            Reason::Score => "score",
            Reason::Sentinel => "sentinel",
            Reason::MoveLimit => "move_limit",
        }
    }
}

#[derive(Clone, Debug)]
pub struct State {
    pub heights: [u8; SQUARES],
    pieces: [Piece; MAX_PIECES],
    len: u8,
    /// Indice de la pièce sur chaque case, ou `NONE`.
    occ: [u8; SQUARES],
    pub to_move: u8,
    pub ply: u16,
    pub scores: [u8; 2],
    /// Nombre de pièces sorties (tous camps) : seul compte pour les règles R8.
    pub finished: u8,
    pub over: bool,
    pub winner: Option<u8>,
    pub reason: Option<Reason>,
    pub move_limit: u16,
}

impl State {
    pub fn new(
        heights: [u8; SQUARES],
        pieces: &[Piece],
        to_move: u8,
        ply: u16,
        scores: [u8; 2],
        finished: u8,
        move_limit: u16,
    ) -> State {
        assert!(pieces.len() <= MAX_PIECES, "trop de pièces");
        let mut s = State {
            heights,
            pieces: [Piece { owner: 1, kind: Kind::Explorer, sq: 0 }; MAX_PIECES],
            len: pieces.len() as u8,
            occ: [NONE; SQUARES],
            to_move,
            ply,
            scores,
            finished,
            over: false,
            winner: None,
            reason: None,
            move_limit,
        };
        s.pieces[..pieces.len()].copy_from_slice(pieces);
        s.reindex();
        s
    }

    fn reindex(&mut self) {
        self.occ = [NONE; SQUARES];
        for i in 0..self.len as usize {
            self.occ[self.pieces[i].sq as usize] = i as u8;
        }
    }

    pub fn pieces(&self) -> &[Piece] {
        &self.pieces[..self.len as usize]
    }

    pub fn piece(&self, i: u8) -> Piece {
        self.pieces[i as usize]
    }

    /// Indice de la pièce sur `sq`, s'il y en a une.
    pub fn at(&self, sq: u8) -> Option<u8> {
        let i = self.occ[sq as usize];
        (i != NONE).then_some(i)
    }

    pub(crate) fn remove(&mut self, i: u8) {
        let n = self.len as usize;
        self.pieces.copy_within(i as usize + 1..n, i as usize);
        self.len -= 1;
        self.reindex();
    }

    pub(crate) fn relocate(&mut self, i: u8, sq: u8) {
        let p = &mut self.pieces[i as usize];
        self.occ[p.sq as usize] = NONE;
        p.sq = sq;
        self.occ[sq as usize] = i;
    }
}
