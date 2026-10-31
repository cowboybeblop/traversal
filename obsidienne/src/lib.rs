//! Obsidienne : bot de l'arène Traversal (WASM). Toutes les règles viennent de
//! `traversal-bot` (vendored) : rien n'est réimplémenté.

pub mod engine;
pub mod eval;
mod tt;

use traversal_bot::core::movegen::rows_to_go;
use traversal_bot::core::{Kind, MoveKind};
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

    let eng = unsafe { (*std::ptr::addr_of_mut!(ENGINE)).as_mut().unwrap() };
    let (mv, _score, depth) = eng.search(s, MAX_TURNS, out_of_fuel);
    let used = t0 - fuel_left();
    log(&format!("PLY {} FUEL {} NODES {} DEPTH {}", s.ply, used, eng.total_nodes, depth));
    index_of(turn, &mv)
}

bot!(choose);
