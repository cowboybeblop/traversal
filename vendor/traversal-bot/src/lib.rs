//! SDK des bots de l'arène Traversal.
//!
//! Un bot est un module WebAssembly (`wasm32-unknown-unknown`) : à chaque coup, l'arène
//! lui donne la position et la liste des coups légaux, il répond l'indice du coup choisi.
//! Le SDK s'occupe de l'interface ; il ne reste qu'à écrire la fonction de choix :
//!
//! ```ignore
//! use traversal_bot::{bot, Turn};
//!
//! fn choose(turn: &Turn) -> usize {
//!     0 // le premier coup légal
//! }
//!
//! bot!(choose);
//! ```
//!
//! Le moteur complet est fourni (`traversal_bot::core` : coups légaux, application des
//! coups, bot de référence) : rien à réimplémenter. Le temps se compte en **fuel** (à peu
//! près une instruction WebAssembly) : `fuel_left()` dit ce qu'il reste pour ce coup. À
//! zéro, le coup est perdu, et la partie avec.

pub use traversal_core as core;
pub use traversal_core::wire::{Turn, BUFFER_SIZE};
use traversal_core::Move;

#[cfg(target_arch = "wasm32")]
#[link(wasm_import_module = "traversal")]
extern "C" {
    #[link_name = "fuel_left"]
    fn host_fuel_left() -> i64;
    #[link_name = "log"]
    fn host_log(ptr: *const u8, len: usize);
}

/// Fuel restant pour ce coup. Hors WebAssembly (tests natifs) : illimité.
pub fn fuel_left() -> u64 {
    #[cfg(target_arch = "wasm32")]
    {
        // SAFETY : fonction fournie par l'arène, sans argument.
        unsafe { host_fuel_left().max(0) as u64 }
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        u64::MAX
    }
}

/// Message de débogage : affiché par `traversal-arena duel --verbose`, ignoré en tournoi.
pub fn log(msg: &str) {
    #[cfg(target_arch = "wasm32")]
    // SAFETY : l'arène lit `len` octets à `ptr`, dans la mémoire du module.
    unsafe {
        host_log(msg.as_ptr(), msg.len())
    }
    #[cfg(not(target_arch = "wasm32"))]
    eprintln!("{msg}");
}

/// Indice d'un coup dans la liste de l'arène (même pièce, même type, même case).
pub fn index_of(turn: &Turn, m: &Move) -> usize {
    turn.moves.iter().position(|x| x.piece == m.piece && x.kind == m.kind && x.to == m.to).unwrap_or(0)
}

#[doc(hidden)]
pub mod __rt {
    use super::*;

    /// Tampon où l'arène écrit chaque tour.
    pub static mut BUFFER: [u8; BUFFER_SIZE] = [0; BUFFER_SIZE];

    pub fn run(len: i32, choose: fn(&Turn) -> usize) -> i32 {
        // SAFETY : un module WebAssembly n'a qu'un fil d'exécution, et l'arène n'écrit
        // pas dans le tampon pendant l'appel.
        let n = (len.max(0) as usize).min(BUFFER_SIZE);
        let buf = unsafe { std::slice::from_raw_parts(std::ptr::addr_of!(BUFFER) as *const u8, n) };
        match traversal_core::wire::decode(buf) {
            Some(turn) => choose(&turn) as i32,
            None => -1,
        }
    }
}

/// Exporte `choose` comme bot de l'arène (`traversal_buffer`, `traversal_move`).
#[macro_export]
macro_rules! bot {
    ($choose:path) => {
        #[no_mangle]
        pub extern "C" fn traversal_buffer() -> *mut u8 {
            // SAFETY : adresse du tampon statique, transmise à l'arène.
            unsafe { ::std::ptr::addr_of_mut!($crate::__rt::BUFFER) as *mut u8 }
        }

        #[no_mangle]
        pub extern "C" fn traversal_move(len: i32) -> i32 {
            $crate::__rt::run(len, $choose)
        }
    };
}
