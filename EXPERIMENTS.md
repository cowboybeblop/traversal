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

### Tuning v5.2 (base `hang=30,nmp=0,lmp=0`)

| Paramètre | Variante | Cap 15 k (24 p.) | Fuel | Décision |
|---|---|---|---|---|
| tempo | 4 / 12 | +44 / −29 | tempo=4 : **−120** | 8 confirmé |
| race | 100 / 150 / 250 | — | +29 / +120 puis **+14 (48 p.)** / −29 | 200 confirmé |
| race_h | 4 / 6 | 0 / 0 | — | 5 confirmé |
| base | 26 / 34 | −29 / +29 | — | 30 confirmé |
| sent_w | 8 / 12 | −58 / +58 | sent_w=12 : 0 | 5 confirmé (profil en cloche) |
| prog_a | 6 | −14 | — | 4 confirmé |
| score | 120 | −104 | — | 100 confirmé |
| exta (extension des attaques +1) | on | −14 | — | rejeté |

Aucun paramètre d'éval ne bouge : la v1 était déjà bien calée, et le criblage
confirme un optimum plat autour des valeurs livrées. Les gains de la v5.2
viennent exclusivement de la recherche (NMP/LMP coupés).

### Expériences risquées v5.2 (toutes rejetées)

| Idée | Principe | Résultat | Décision |
|---|---|---|---|
| `raz=1` | razoring : stand-pat + 300·d ≤ alpha à prof ≤ 2 → quiescence | −29 cap | rejeté |
| `see=1` | filtre SEE en quiescence (attaques non rentables ignorées) | 0 cap, idem combinée | rejeté |
| `sing=1` | extension singulière du meilleur coup racine (+1) | 0 cap | rejeté |
| `qext=1` | quiescence étendue aux coups à distance 1 de sortie | −14 puis +29/24, **−14/48** | rejeté |
| `qext` focalisé | idem, seulement si l'adversaire menace de sortir au trait suivant | +29 cap, +29 fuel/24, **−14/48** | rejeté |

La quiescence Exit/Attack + delta pruning est déjà bien dimensionnée : toute
extension paie son coût en nœuds sans gain stratégique mesurable. Le moteur est
sur un optimum local solide ; les prochains gains devront venir de changements
plus structurels (policy racine, terms d'éval appris), pas de micro-tuning.

## Session v5.3 — pentanomial, race graduée (rdif)

Infrastructure : scoring pentanomial ajouté au harnais (paires inversées LL/LT/TT/WT/WW
+ Elo pentanomial) — filtre les faux positifs du binomial sur petits échantillons.

Flags implémentés et criblés (cap 48 p., puis fuel 48, puis 100+ pour finalistes) :

| Flag | Principe | Cap 48 | Fuel | Décision |
|---|---|---|---|---|
| **`rdif=1`** | race graduée : différentiel de plies de sortie Sentinelle (cap ±race), au lieu du ±200 binaire | **+129, penta +120** | +36/48 puis **+64 (54V-35D-15N)/104 p., penta +84** | ✅ **RETENU** |
| `hb=1` | malus history sur coups silencieux perdants | +14, penta 0 | — | rejeté (bruit) |
| `cm=1` | counter-move table (12 Ko) | +14 puis −44 | — | rejeté |
| `ssafe=1` | sûreté Sentinelle (distance aux ennemis capables de capturer) | −44 puis −29 | — | rejeté |
| `rn=1` | concentration racine top-4 dès prof 6 | 0, penta 0 | 0, penta −17 | rejeté |
| `rs=1` | stabilité racine | — | — | déjà le comportement par défaut (retrait du flag) |
| `rdif` + `race=300` / `race=120` | re-tuning du cap gradué | +51/+44 cap | **−73** / 0 | bruit cap ; race=200 confirmé |

Note : le flag `rs` a révélé au passage que le best move n'était déjà mis à jour que
sur itération complétée — la « stabilité fuel » existait ; le flag a été retiré.

Validation externe `duelf 5` (Mont Blanc L5), `hang=30,nmp=0,lmp=0,rdif=1` :
- [0+36] 32V-4D-0N ; [40+76] 31V-4D-1N ; [120+156] 32V-4D-0N
- Cumul 108 p. : **95V-12D-1N ≈ +350 Elo** (v5.2 : ~+300/48 p.)

## Livraison

- `Flags::best()` = { hang_w: 30, nmp: false, lmp: false, rdif: true }.

### Session v5.3 bis — catalogue restant (tous rejetés)

Base : v5.3 (`hang=30,nmp=0,lmp=0,rdif=1`). Cap 48 p. + fuel 48 p. pour les neutres.

| Flag | Principe | Résultat | Décision |
|---|---|---|---|
| `ch=1` | continuation history (kind,to du coup précédent) | 0 cap, 0 fuel, penta 0 | rejeté |
| `seet=1` | SEE signé : attaques perdantes triées après les silencieux | 0 cap, 0 fuel | rejeté |
| `zc=1` | contrôle des zones de sortie (prime ≤3 tours) | **−44** cap, penta −41 | rejeté |
| `mat=1` | terme matériel (12×Force vive) | −7 cap, penta −17 | rejeté (matériel déjà implicite dans prog/score) |

Notes d'implémentation : `chist` (360 Ko) doit vivre sur le heap WASM (Vec),
sinon la structure Engine copiée par valeur crashe la mémoire (fix documenté).
`zc` dégrade : la prime de zone duplique le terme de progrès et pousse à
s'entasser près de la sortie. `mat` n'apporte rien : la Force est déjà capturée
par prog_b et le différentiel de score de sortie.

Bilan catalogue : search/eval « classique » épuisé sur ce moteur — toutes les
heuristiques standard restantes sont neutres ou perdantes. Prochaine étape
rentable : pipeline de données (Texel #15 → policy racine #16 → NNUE #18).

## Session v6 — pipeline data : Texel tuning + SPRT (mode expert)

Infrastructure livrée :

1. **`selfdump`** (harnais) : self-play fuel avec dump de positions labellisées
   (W/D/L final côté trait), une partie sur deux contre un bot de référence pour
   la diversité. Format : `to_move ply scores | hauteurs | pièces | z`.
2. **`tools/texel.py`** : réimplémentation Python de l'eval (Dijkstra exit_costs,
   turns_for, prog, sent, hang, rdif, tempo) + régression logistique (BCE,
   scale 250, Adam sur gradient numérique). Params tunés : score, base, prog_a,
   prog_b, sent_w, hang_w, race, tempo.
3. **`sprt`/`fsprt`** (harnais) : test séquentiel (likelihood-ratio trinomial,
   seuils ±2.944 = α/β 5 %) avec arrêt automatique — remplace les seuils
   manuels « 24/48/104 ».

### Résultats Texel

- Corpus 1 (self-play pur, 7 717 pos.) : loss 0.637→0.624, mais la config
  suggérée **perd −51 Elo en fuel**. corpus biaisé (le bot s'entraîne à prédire
  ses propres décisions).
- Corpus 2 (mixte vs réf., 9 142 pos.) : loss 0.547→0.514, suggestions encore
  plus extrêmes (hang_w→100, score→17) : biais « camp fort = v5.3 », la
  régression apprend l'eval v5.3 elle-même plutôt que l'optimum.
- Application **un paramètre à la fois** (leçon du skill : ne jamais livrer un
  tuning global d'un coup) : `prog_a=10` semblait gagner (+27 Elo/104 p. fuel)
  mais le **SPRT formel (elo0=0, elo1=+5, α=β=5 %) est INCONCLUSIF** :
  106V-112D-22N sur 240 p., llr=−0.01. Les faux positifs de moindre ampleur
  (`prog_a=8` : +58 cap puis **−37/104 fuel** ; `hang_w=55` : −51 ; `prog_b=11` :
  −97 ; `sent_w=7` : −51) confirment la nature bruitée du corpus.

Conclusions d'expert :
- Le label « résultat final » sur corpus mono-bot est trop bruité pour les
  constantes de second ordre (hang, sent, race) : il faudrait des evals de
  recherche profonde comme cibles (distillation), pas des W/L de parties.
- Le Texel W/L sur corpus mono-bot ne produit **aucun paramètre livrable** :
  la config v5.3 reste inchangée. Pour débloquer : labels de distillation
  (evals de recherche profonde plutôt que résultats de partie) — chantier
  documenté ci-dessous.

### Chantier suivant (documenté, non lancé) : #16 policy racine / #18 NNUE

1. **Labels de distillation** : rejouer le corpus en enregistrant (position →
   score de recherche v5.3 à prof 12+) au lieu du W/L final ; le tuner régresse
   sur ces scores — c'est la méthode « Lc0→SF » du skill, la seule qui casse
   le biais mono-bot.
2. **Policy racine (#16)** : entraîner un petit modèle (20-30 features par coup
   racine : delta Dijkstra, attaque/défense, hang, rdif…) à prédire le coup
   choisi par la recherche ; utilisation **ordering racine uniquement**.
3. **NNUE (#18)** : feature-set (type, camp, case) + hauteur de case +
   menaces explicites ; accumulator incrémental obligatoire ; ~1-2 M positions
   d'entraînement nécessaires ; quantification int8. Infra GPU requise —
   hors de ce sandbox, à documenter dans un RFC dédié.

## Livraison v6

- Config **inchangée** (v5.3) : `Flags::best()` = { hang_w: 30, nmp: false,
  lmp: false, rdif: true }.
- Nouveaux outils livrés : `selfdump`, `tools/texel.py`, `sprt`/`fsprt`.

### Session v6 bis — IIR, 3 killers, distillation MSE

| Expérience | Principe | Résultat | Décision |
|---|---|---|---|
| `k3=1` (vrais 3 killers) | 3e slot killer, bonus 360k | −14 cap | rejeté |
| `iir=1` | internal iterative reduction (prof ≥ 4 sans TT) | 0 cap, +7 fuel/48, **SPRT INCONCLUSIF** (106V-118D-16N, llr −0,01/240 p.) | neutre → non livré (code dispo) |
| `distill` (nouvel outil) | corpus labellisé par scores de recherche (cap 40 k, prof 12) | 6 114 pos. ; MSE 10 145 ; suggestions ≈ valeurs actuelles | outil livré |
| `prog_a=7` (suggestion distill) | — | +73 cap (penta +64) puis **−58 fuel** | rejeté : 3e faux positif cap sur prog_a |

Leçon structurelle : les scores de recherche v5.3 sont produits par l'eval v5.3 —
la distillation MSE converge vers les constantes courantes (auto-consistance),
seule la W/L de parties diversifiées peut les déplacer, et son bruit l'interdit
à ce jour. Trois méthodes indépendantes (criblage manuel, Texel W/L, distill)
convergent : **les constantes v5.3 sont un point fixe** — seul un apprentissage
hors-ligne de plus grande ampleur (policy/NNUE, infra GPU) peut les déplacer.

## Livraison v6 bis

- Config inchangée (v5.3). Nouveaux flags disponibles : k3, iir. Nouvel outil :
  `distill` (+ support MSE/`--distill` dans tools/texel.py).
