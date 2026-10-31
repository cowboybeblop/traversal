//! Moteur Traversal en Rust : règles + bot.
//!
//! Le serveur du site a son propre moteur (Python), qui reste la RÉFÉRENCE (API, notation,
//! détail des combats) ; celui-ci sert au bot du site, à l'arène des bots et aux
//! simulations. Les deux sont tenus identiques : le site les compare sur des milliers de
//! positions (coups légaux, positions, fins de partie, évaluation du bot, constantes).

pub mod bot;
pub mod config;
pub mod movegen;
pub mod rules;
pub mod state;
pub mod wire;

pub use bot::{choose_move, Choice, Ctx, Level, Rng, Tuning};
pub use config::Kind;
pub use movegen::{legal_moves, Move, MoveKind, Special};
pub use rules::{apply_move, find_move, new_game};
pub use state::{Piece, State};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SQUARES;
    use std::time::Duration;

    fn flat(h: u8) -> [u8; SQUARES] {
        [h; SQUARES]
    }

    #[test]
    fn opening_moves_and_symmetry() {
        let s = new_game(flat(1));
        assert_eq!(s.pieces().len(), 12);
        let moves = legal_moves(&s);
        assert!(moves.iter().all(|m| s.piece(m.piece).owner == 1));
        // Sur le plat, le bond n'apporte rien (la case est atteignable à pied) : pas proposé.
        assert!(moves.iter().all(|m| m.special == Special::None));
        // Falaise (rangée 1 à 2, rangée 2 à 4) : à pied c'est trop cher, le bond passe.
        let mut cliff = flat(0);
        cliff[10..20].fill(2);
        cliff[20..30].fill(4);
        let s = new_game(cliff);
        let leap = legal_moves(&s).into_iter().find(|m| m.special == Special::Leap).expect("bond");
        assert_eq!((leap.to / 10, leap.cost), (2, 0));
    }

    #[test]
    fn sentinel_first_out_wins() {
        // Sentinelle rouge sur la dernière rangée : elle sort et gagne.
        let pieces = [Piece { owner: 1, kind: Kind::Sentinel, sq: 94 }, Piece { owner: 2, kind: Kind::Sentinel, sq: 5 }];
        let s = State::new(flat(1), &pieces, 1, 10, [0, 0], 0, 200);
        let exit = legal_moves(&s).into_iter().find(|m| m.kind == MoveKind::Exit).expect("sortie");
        let after = apply_move(&s, &exit);
        assert!(after.over && after.winner == Some(1));
        // Le bot la trouve, quel que soit le niveau.
        let level = Level { depth: 2, beam: 12, noise: 0.0, budget: Duration::from_millis(500), insight: 0, tuning: Default::default(), stop: None };
        let c = choose_move(&s, level, &mut Rng::new(1));
        assert_eq!(c.mv.kind, MoveKind::Exit);
    }

    #[test]
    fn capture_removes_target_and_keeps_order() {
        let pieces = [
            Piece { owner: 2, kind: Kind::Explorer, sq: 44 },
            Piece { owner: 1, kind: Kind::Sentinel, sq: 43 },
            Piece { owner: 2, kind: Kind::Sentinel, sq: 99 },
        ];
        let s = State::new(flat(1), &pieces, 1, 0, [0, 0], 0, 200);
        let atk = find_move(&s, 1, MoveKind::Attack, 44).expect("attaque");
        let after = apply_move(&s, &atk);
        assert_eq!(after.pieces().len(), 2);
        assert_eq!(after.piece(0), Piece { owner: 1, kind: Kind::Sentinel, sq: 44 });
        assert_eq!(after.at(44), Some(0));
    }

    #[test]
    fn deterministic_with_seed() {
        let s = new_game(flat(2));
        let level = Level { depth: 2, beam: 12, noise: 8.0, budget: Duration::from_secs(2), insight: 0, tuning: Default::default(), stop: None };
        let a = choose_move(&s, level, &mut Rng::new(42)).mv;
        let b = choose_move(&s, level, &mut Rng::new(42)).mv;
        assert_eq!(a, b);
    }
}
