# Obsidienne — bot de l'arène Traversal

Bot Rust/WebAssembly pour l'arène [Traversal](https://github.com/Sylv-Lej/traversal-arena)
(SDK [`traversal-rust-sdk`](https://github.com/Sylv-Lej/traversal-rust-sdk) v0.2.0, vendored
dans `vendor/` — aucune règle du jeu n'est réimplémentée).

## Le jeu en bref

Grille 10×10 de hauteurs (dénivelé ≤ 2), 6 pièces par joueur (2× Explorateur M5/F1,
Avant-garde M4/F2, Sentinelle M3/F3). Déplacement par Dijkstra sous budget M
(coût |Δh|+1), combat légal si atk ≥ def (Force ± hauteur + soutiens adjacents + charge).
Sortie par le bord opposé avec ≥ 1 M restant ; 2 pièces sorties = fin ; **première
sortie = Sentinelle = victoire immédiate (R8.2)** ; limite 200 demi-coups. Budget d'un
coup : 2 Md de fuel WASM, module ≤ 1 Mo, mémoire ≤ 64 Mo.

## Architecture

- `obsidienne/src/engine.rs` — négamax alpha-beta (PVS) à approfondissement itératif :
  table de transposition (2^20 × 24 o), tri TT/killers/history à gravité, null-move
  gardé, LMR, LMP, futility, extension des sorties (+1), quiescence (Exit/Attack avec
  delta pruning), fenêtres d'aspiration optionnelles, cache d'évaluations Zobrist.
- `obsidienne/src/eval.rs` — évaluation : score, distance à la sortie (Dijkstra),
  prime de course des Sentinelles (R8.2), tempo, + termes togglables : pièces menacées
  (`hang`), distance barrée par les Sentinelles adverses (`blocked`), soutien.
- `obsidienne/src/tt.rs` — table de transposition compacte (coup packé sur 20 bits).
- `obsidienne/src/lib.rs` — interface arène (`bot!`), moteur persistant, raccourci
  Sentinelle (R8.2), réserve de fuel 3/16 + repli glouton sûr.
- `wasitest/src/main.rs` — harnais de mesure sous WASI : duels contre les niveaux du
  site (Galet→Mont Blanc), miroirs A/B (cap en nœuds ou parité fuel), sanity, vitesse,
  chemin de production exact, concordance vs alpha-beta brut.
- `vendor/` — moteur du SDK v0.2.0 (vendored, sans dépendance).
- `run.mjs` — hôte WASI de test (Node 22+, simule `fuel_left`/`log` de l'arène).
- `sdk/boards.json` — 200 plateaux officiels (lecture par le harnais).

## Build

```sh
# bot de production (à charger dans l'arène)
cargo build --release --target wasm32-unknown-unknown -p obsidienne
# → target/wasm32-unknown-unknown/release/obsidienne.wasm (~214 Ko)

# harnais de test local (Node 22+ requis)
cargo build --release --target wasm32-wasip1 -p wasitest
node run.mjs sanity 8          # L1/L3/L5 + rejeu déterministe
node run.mjs mirror 100 15000 hang=30,blocked=140,ec=1 hang=30   # A/B cap
node run.mjs fmirror 104 hang=30,blocked=140,ec=1 hang=30        # A/B fuel
node run.mjs duelf 5 120 0 hang=30,blocked=140,ec=1              # vs Mont Blanc
```

## Configuration livrée

`Flags::best()` = `hang=30,blocked=140,ec=1` : pièces menacées (−30×Force), distance
de progression mélangée à 40 % avec un Dijkstra « avec obstacles », cache d'évaluations
Zobrist (2^18 entrées, états terminaux exclus). Choix et mesures : voir
[`EXPERIMENTS.md`](EXPERIMENTS.md).

## Historique des versions

- **v1** — moteur complet +TT/killers/NMP/LMR/quiescence ; ~+437 Elo revendiqués vs
  Mont Blanc (chiffre historique, non reproduit depuis les reconstructions).
- **v4** — `hang=30` (pièces menacées).
- **v5** — `blocked=140` + cache d'évaluations ; +165 Elo vs Mont Blanc (120 p.).
- **v5.1 (ce build)** — reconstruction intégrale après perte de l'environnement ;
  voir la note de reconstruction dans `EXPERIMENTS.md`.
