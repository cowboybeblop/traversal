//! Constantes du jeu, identiques à celles du moteur du serveur (comparées par les tests
//! d'équivalence du site).

pub const SIZE: usize = 10;
pub const SQUARES: usize = SIZE * SIZE;

pub const MIN_HEIGHT: u8 = 0;
pub const MAX_HEIGHT: u8 = 6;
pub const MAX_STEP: u8 = 2;

/// R5.2 Le bond de l'Explorateur ne passe jamais au-dessus d'une pièce.
pub const LEAP_OVER_PIECES: bool = false;
/// R5 [?] Diagonale / bond peuvent attaquer une pièce ennemie sur la case d'arrivée.
pub const SPECIAL_MOVES_CAN_ATTACK: bool = true;
/// R7.5 Bonus de charge de l'Avant-garde : plus court chemin == ce coût exact.
pub const VANGUARD_CHARGE_COST: u8 = 4;
/// R8.1 La partie s'arrête quand ce nombre de pièces (tous camps) est sorti.
pub const FINISHERS_TO_END: u8 = 2;
/// R8.3 Limite de coups (demi-coups, les deux joueurs confondus).
pub const MOVE_LIMIT: u16 = 200;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Kind {
    Explorer,
    Vanguard,
    Sentinel,
}

impl Kind {
    /// R3 Mouvement = coins de la base.
    pub const fn movement(self) -> u8 {
        match self {
            Kind::Explorer => 5,
            Kind::Vanguard => 4,
            Kind::Sentinel => 3,
        }
    }

    /// R3 Force = pointes au sommet (M + F = 6).
    pub const fn strength(self) -> u8 {
        match self {
            Kind::Explorer => 1,
            Kind::Vanguard => 2,
            Kind::Sentinel => 3,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Kind::Explorer => "explorer",
            Kind::Vanguard => "vanguard",
            Kind::Sentinel => "sentinel",
        }
    }

    pub fn parse(s: &str) -> Option<Kind> {
        match s {
            "explorer" => Some(Kind::Explorer),
            "vanguard" => Some(Kind::Vanguard),
            "sentinel" => Some(Kind::Sentinel),
            _ => None,
        }
    }
}
