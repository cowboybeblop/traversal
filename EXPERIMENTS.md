# Obsidienne — carnet d'expériences

Bot de l'arène Traversal (`traversal-arena`, SDK Rust `traversal-rust-sdk` v0.2.0).
Mesures : miroirs A/B (cap = budget de nœuds égal) et fuel A/B (fmirror/duelf :
parité de **fuel réel**, un nœud coûte selon les instructions WASM qu'il exécute).
Elo = −400·log10(1/p−1). Discipline : une idée par expérience, mesure avant merge,
aucune règle du jeu réimplémentée (tout vient du SDK vendored).

## Configurations

- `default` = v1 : négamax PVS + TT + killers + history + NMP gardé + LMR + LMP +
  futility + extension sortie +1 + quiescence (Exit/Attack, delta pruning).
- `hang=30` (v4 livrée) : + terme « pièce menacée » (un voisin ennemi adjacent
  gagnerait le combat → −30×Force).
- `blocked=140` : distance de progression mélangée à 60/40 avec un Dijkstra
  « avec obstacles » (Sentinelle adverse = mur, péages alliés) — voit les blocages.
- `ec=1` : cache d'évaluations Zobrist direct-mapped (2^18 entrées, 4 Mo),
  états terminaux exclus (score dépend du demi-coup).
- **v5 livrée : `hang=30,blocked=140,ec=1`** (`Flags::best()`).

## Coût fuel par nœud (instructions ≈ vitesse mesurée, WASI)

| spec | nœuds/s | fuel/nœud |
|---|---|---|
| default | 660 k | 23 400 (référence) |
| hang=30 | 551 k | 28 033 |
| hang=30,ec=1 | 628 k | 24 594 |
| hang=30,blocked=140 | 136 k | 113 455 |
| hang=30,blocked=140,ec=1 | 170 k | 90 530 |

⚠️ Le poids `blocked` de la session précédente (+1 300) sous-estimait le coût
réel de ~65× : blocked sans cache = 4,7× le fuel/nœud de default. Le cache
récupère −74 % du coût de hang et −25 % (à froid) de celui de blocked.
En production le cache persiste d'un coup à l'autre → encore meilleur.

## Résultats de la session (échantillons ≥ 100 parties sauf mention)

### A/B à budget de nœuds égal (cap 15 000)

- `hang=30,blocked=140` vs `hang=30` : **61V-35D-4N, +92 Elo (100 p.)**
- `hang=30` vs `default` : 51V-43D-6N, +28 Elo (100 p.)
- `hang=30,blocked=140,ec=1` vs `hang=30,blocked=140` : 3V-4D-1N (8 p., bruit)
  → le cache est transparent pour le jeu (même valeur retournée).

### A/B à parité fuel réelle (fmirror, poids mesurés ci-dessus)

- `hang=30,blocked=140,ec=1` vs `hang=30` : **55V-47D-2N, +27 Elo (104 p.)**
  (poids cache-froid → estimation basse ; production attendue meilleure)
- `hang=30,ec=1` vs `hang=30` : 32V-25D-7N, +38 Elo (64 p.)
- `hang=30,blocked=140,ec=1` vs `hang=30,ec=1` : **32V-28D-4N, +22 Elo (64 p.)**
- `blocked` seul **rejeté sans cache** : 4,7× le fuel/nœud ≈ −1 ply.

### Contre le bot de référence, mode fuel (L5 « Mont Blanc »)

- v5 `hang=30,blocked=140,ec=1` : **84V-31D-5N, +165 Elo (120 p., ~17,6 k nœuds/coup)**
- v4 `hang=30` (livraison précédente), mêmes 120 cartes : 83V-33D-4N, +157 Elo
  (~57 k nœuds/coup) → la nouvelle éval compense 3,2× moins de nœuds.
- v5 vs L1 : 8V-0D (8 p.) ; vs L3 : 7V-1D (8 p.).

### Historique (sessions précédentes, petits échantillons)

- v1 vs Mont Blanc : 42V-5D-1N, ~+437 Elo (48 p., cap 30 k) — chiffre historique
  non reproduit depuis (rebuilds successifs ~52 % sur cap 30 k) ; les A/B internes
  restent valides.
- Rejetés : support=6 (−417), aspiration (neutre, −58 fuel), hang=60 (−154),
  miroirs d'éval (score=260, race=350, sent_w=8, prog élargi), blocked sans cache.

## Décision

`hang=30,blocked=140,ec=1` retenu : à nœuds égaux c'est le meilleur terme
connu (+92), le cache le rend abordable en fuel (+27 vs v4 sur 104 p.) et le
tête-à-tête contre l'alternative « hang + cache seul » donne +22 sur 64 p.
Pas de régression vs Mont Blanc (+165 vs +157 sur 120 p. appariées).

## Validation finale (build de production)

- `wasm32-unknown-unknown` : 216 434 octets (limite 1 Mo).
- prodpath (encodage fil exact de l'arène, rejeu intégral) : déterministe.
- Fuel : 17,6 k nœuds/coup × ~90 k = 1,59 Md < budget 1,625 Md + réserve 3/16.
- Mémoire : TT 24 Mo + cache éval 4 Mo + tables ≈ 30 Mo (limite 64 Mo).

## Pistes non retenues / à creuser plus tard

- `blocked` sans cache (trop cher en fuel), `support` (dégrade), aspiration.
- Tuning hang=20/45, blocked=100/200 à grande échelle, race_h, prog_span.
- Instrumenter le taux de succès réel du cache en partie (compteurs hits).
- Le mystère « plus de cap ≠ plus de gain vs Mont Blanc » (20 k→40 k) reste
  non résolu — possiblement du bruit.

## Reconstruction v5.1 (31/10/2026, build courant)

L'environnement de développement a été entièrement réinitialisé (sources perdues) ;
le projet a été reconstruit à l'identique à partir des notes d'architecture. Ce qui suit
mesure ce build reconstruit.

### Correctif de harnais (méthodologique)

Le harnais reconstruit jouait la carte `boards[off+i]` avec couleur `i%2` : or les
plateaux officiels alternent la couleur avantagée par parité d'indice (carte paire →
bleu, carte impaire → rouge ; vérifié : carte 0 gagnée par le bleu quel que soit le
joueur). Ce couplage mettait le joueur A sur la couleur défavorisée à CHAQUE partie
(default vs default : 0V-8D au lieu de 4V-4D). Rétabli le schéma d'origine : chaque
carte est jouée deux fois à couleurs inversées (`i/2` = carte, `i%2` = couleur de A).
Toutes les mesures ci-dessous utilisent ce schéma équitable ; les nombres historiques
plus haut ont été pris avec ce même schéma sur le build original.

### Poids fuel re-mesurés sur ce build (par nœud, base 23 400)

- `hang=30` : +651 → 24 051 (24 k nœuds/coup en fuel).
- `blocked=140` : +61 660 (2 Dijkstra par éval — bien plus cher que l'ancienne
  estimation +1 300, moins cher que la mesure originale +90 130 : reconstruction).
- `ec=1` : −21 317 avec blocked, −1 112 avec hang seul, +915 seul (à froid ; en
  production le cache persiste d'un coup à l'autre → meilleur).
- `hang=30,blocked=140,ec=1` : 64 394 → ~25,2 k nœuds/coup (l'original v5 : ~17,6 k).

### Validations du build reconstruit

- Compilation 0 erreur ; vitesse ~390-500 k nœuds/s en WASI (original : ~660 k).
- Mirror default/default : 4V-4D exact (harnais équilibré) ; rejeu déterministe.
- Sanity : L1 8/8, L3 5/8, L5 2/8 (cap 20k, spec default — la note historique
  « 8/8, 8/8, 7/8 » portait des cartes/spécs différentes).
- prodpath (encodage fil exact de l'arène + rejeu intégral) : déterministe.
- vs Mont Blanc (L5), mode fuel, spec v5 : [0+18] 12V-6D-0N = 67 % ≈ +115 Elo
  (historique : +165 sur 120 p. — cohérent à ce seuil d'échantillon).
- vs Mont Blanc, mode fuel, spec default : [0+18] 8V-9D-1N ≈ 44 % → tout l'avantage
  vient des termes hang/blocked/ec, à re-valider sur ≥ 100 parties.

### À faire (prochaine session)

- Étendre duelf L5 spec v5 à ≥ 100 parties ; fmirror v5 vs hang=30 à ≥ 100 parties.
- Confronter le mystère « 44 % default vs 52-67 % selon spec » à grande échelle.
- Pousser sur GitHub dès que le connecteur a la permission Contents:write.

## Session v5.2 — re-criblage complet (NMP/LMP)

Constat initial : le build v5.1 livrait `hang=30,blocked=140,ec=1`. Re-criblage
systématique en cap puis fuel (fmirror 24, confirmation 48 à offset disjoint) :

| Expérience | Résultat | Décision |
|---|---|---|
| `blocked=140,ec=1` vs `hang=30` (fuel, 24 p.) | 9V-14D-1N, **−73 Elo** | écarté définitivement |
| `hang=20` / `hang=40` vs `hang=30` (fuel) | −29 / −89 | hang=30 confirmé |
| **`nmp=0`** vs `hang=30` (cap, fuel 24, fuel 48) | +104 / +104 / **+191 (33V-9D-6N)** | ✅ **RETENU** |
| **`nmp=0,lmp=0`** vs `nmp=0` (fuel 24, 48) | +44 / **+51 (27V-20D-1N)** | ✅ **RETENU** |
| `fut=0`, `lmr=0`, `tt=0` sur nouvelle base (cap) | −120 / −29 / −29 | tous gardés |
| `asp=1` / `ec=1` sur nouvelle base (fuel, 24 p.) | 0 / −14 | neutres → off |
| `hang=25` / `hang=35` sur nouvelle base (fuel 24/48) | +29 puis −14 / −73 | hang=30 confirmé |
| `delta=80` (cap +104, fuel +29, confirm. 48) | **−51** | bruit cap, rejeté ; 60 gardé |
| `futility=100` (cap +104, fuel −29) | — | rejeté ; 140 gardé |
| `ord=1` (base Dijkstra dans l'ordering interne) | −73 cap, **−137 fuel** | rejeté |
| `lmr1=1` (réduction LMR fixe r=1) | −29 cap | rejeté |

Pourquoi NMP devait sauter : le null-move est primitif ici (tour inversé sans
véritable tempo — l'adversaire « passe » dans la simulation), il crée des coupures
fictives sur des positions de course. LMP, conçu pour élaguer du silencieux,
perdait des coups de progression utiles dès que NMP ne masquait plus les scores.

Validation externe `duelf 5` (Mont Blanc, L5), `hang=30,nmp=0,lmp=0` :
- [0+12] 11V-1D-0N ; [6+30] 18V-2D-4N ; [30+54] 21V-3D-0N → cumul **39V-5D-4N ≈ +300 Elo**.
- Miroir fuel vs l'ancienne config livrée : **39V-9D-0N (+255 Elo)** sur 48 p.
- ~67 k nœuds/coup, wasm 209 Ko, sanity/rejeu OK.

## Livraison

- `Flags::best()` = { hang_w: 30, nmp: false, lmp: false } — le reste off.
