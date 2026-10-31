//! Obsidienne : bot de l'arène Traversal (WASM). Toutes les règles viennent de
//! `traversal-bot` (vendored) : rien n'est réimplémenté.

pub mod book;
pub mod engine;
pub mod eval;
mod tt;

use traversal_bot::core::movegen::rows_to_go;
use traversal_bot::core::{Kind, MoveKind, State};
use traversal_bot::{bot, fuel_left, index_of, log, Turn};

use crate::engine::{Engine, Flags, MAX_TURNS};
use crate::eval::EvalCfg;

/// Fuel gardé en réserve pour répondre proprement (3/16 du budget d'un coup).
const FUEL_RESERVE: u64 = 2_000_000_000 / 16 * 3;
/// En dessous : plus de recherche, choix glouton sûr.
const FUEL_FLOOR: u64 = 400_000;

static mut ENGINE: Option<Engine> = None;
static mut PREV_PLY: u16 = u16::MAX;

fn out_of_fuel() -> bool {
    fuel_left() < FUEL_RESERVE
}

/// Choix glouton de secours (jamais d'indice illégal).
fn pick_greedy(turn: &Turn) -> usize {
    let s = &turn.state;
    if let Some(i) = turn.moves.iter().position(|m| m.kind == MoveKind::Exit) {
        return i;
    }
    let mut best = 0;
    let mut bestk = -1i32;
    for (i, m) in turn.moves.iter().enumerate() {
        if m.kind == MoveKind::Attack {
            let t = s.piece(s.at(m.to).unwrap_or(0));
            let k = t.kind.strength() as i32;
            if k > bestk {
                bestk = k;
                best = i;
            }
        }
    }
    if bestk >= 0 {
        return best;
    }
    let mut best = 0;
    let mut bestk = i32::MIN;
    for (i, m) in turn.moves.iter().enumerate() {
        if m.kind == MoveKind::Move {
            let p = s.piece(m.piece);
            let k = rows_to_go(p.owner, p.sq) as i32 - rows_to_go(p.owner, m.to) as i32;
            if k > bestk {
                bestk = k;
                best = i;
            }
        }
    }
    best
}

fn choose(turn: &Turn) -> usize {
    let s = &turn.state;
    let t0 = fuel_left();

    // Moteur persistant : reconstruit si le relief change ou si le demi-coup redescend
    // (nouvelle partie dans la même instance) — tables repartent à zéro.
    unsafe {
        let ptr = std::ptr::addr_of_mut!(ENGINE);
        let rebuild = match (*ptr).as_ref() {
            None => true,
            Some(e) => e.heights != s.heights || s.ply < PREV_PLY,
        };
        if rebuild {
            *ptr = Some(Engine::new(s.heights, EvalCfg::default(), Flags::best()));
        }
        PREV_PLY = s.ply;
    }

    // Raccourci : première sortie = Sentinelle = victoire immédiate (R8.2).
    if s.finished == 0 {
        if let Some(m) = turn
            .moves
            .iter()
            .find(|m| m.kind == MoveKind::Exit && s.piece(m.piece).kind == Kind::Sentinel)
        {
            log(&format!("PLY {} FUEL {} shortcut", s.ply, t0 - fuel_left()));
            return index_of(turn, m);
        }
    }

    if fuel_left() < FUEL_FLOOR {
        let i = pick_greedy(turn);
        log(&format!("PLY {} FUEL {} greedy", s.ply, t0 - fuel_left()));
        return i;
    }

    // Opening book : coup précalculé hors-ligne (bookgen) → zéro fuel dépensé.
    if let Some(i) = book_probe(turn) {
        log(&format!("PLY {} FUEL {} book", s.ply, t0 - fuel_left()));
        return i;
    }

    let eng = unsafe { (*std::ptr::addr_of_mut!(ENGINE)).as_mut().unwrap() };
    let (mv, _score, depth) = eng.search(s, MAX_TURNS, out_of_fuel);
    let used = t0 - fuel_left();
    log(&format!("PLY {} FUEL {} NODES {} DEPTH {}", s.ply, used, eng.total_nodes, depth));
    index_of(turn, &mv)
}

/// Clé de livre d'une position (FNV-1a — doit rester identique à `bookgen`).
fn book_key(s: &State) -> u64 {
    let mut key: u64 = 0xcbf2_9ce4_8422_2325;
    for &x in s.heights.iter() {
        key = (key ^ x as u64).wrapping_mul(0x1000_0000_01b3);
    }
    for pc in s.pieces() {
        key = (key ^ ((pc.owner as u64) << 24 | (pc.kind.strength() as u64) << 16 | pc.sq as u64)).wrapping_mul(0x1000_0000_01b3);
    }
    key ^= s.to_move as u64;
    key.rotate_left(13) ^ (s.scores[0] as u64) ^ ((s.scores[1] as u64) << 8)
}

/// Cherche le coup du livre pour la position courante ; None si hors livre.
fn book_probe(turn: &Turn) -> Option<usize> {
    let s = &turn.state;
    let key = book_key(s);
    let idx = crate::book::BOOK.binary_search_by(|(k, _, _, _)| k.cmp(&key)).ok()?;
    let (_, piece, mvtype, to) = crate::book::BOOK[idx];
    // mvtype : 0=Move, 1=Attack, 2=Exit, 3=Pass (encodage bookgen = MoveKind as u8)
    let i = turn
        .moves
        .iter()
        .position(|m| {
            let want = (piece, mvtype, to);
            let got = (
                m.piece,
                match m.kind {
                    MoveKind::Move => 0u8,
                    MoveKind::Attack => 1,
                    MoveKind::Exit => 2,
                    MoveKind::Pass => 3,
                },
                m.to,
            );
            want == got
        })?;
    Some(i)
}

bot!(choose);
