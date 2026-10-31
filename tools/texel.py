#!/usr/bin/env python3
"""Tuning Texel d'Obsidienne : régression logistique sur les constantes d'éval.

Corpus : lignes du mode `selfdump` du harnais (résultat final W/D/L côté trait).
Modèle : eval(side) = Σ w_k · f_k (features agrégées, param-indépendantes pour
span fixés), perte BCE sur sigmoid(eval/scale), optimisation Adam + gradient
numérique (robuste aux max/clamp de l'eval).

Paramètres tunés : score, base, prog_a, prog_b, sent_w, hang_w, race (rdif), tempo.
Fixés : prog_span=13, sent_span=8, race_h=5 (formes/gates, non-linéaires).

Usage : python3 tools/texel.py corpus.txt [--iters 3000]
"""

import heapq
import math
import re
import sys

SIZE = 10
SQUARES = 100
MOVEMENT = {1: 5, 2: 4, 3: 3}
WIN = 100_000
PROG_SPAN = 13
SENT_SPAN = 8

# noms et valeurs initiales (v5.3)
PARAMS = ["score", "base", "prog_a", "prog_b", "sent_w", "hang_w", "race", "tempo"]
INIT = dict(score=100, base=30, prog_a=4, prog_b=3, sent_w=5, hang_w=30, race=200, tempo=8)


def neighbors(sq):
    x, y = sq % SIZE, sq // SIZE
    r = []
    if x > 0:
        r.append(sq - 1)
    if x < SIZE - 1:
        r.append(sq + 1)
    if y > 0:
        r.append(sq - SIZE)
    if y < SIZE - 1:
        r.append(sq + SIZE)
    return r


def step_cost(h, a, b):
    dh = abs(int(h[a]) - int(h[b]))
    return 1 if dh == 0 else dh + 1


def exit_costs(h, owner):
    row = SIZE - 1 if owner == 1 else 0
    dist = [math.inf] * SQUARES
    heap = []
    for x in range(SIZE):
        s = row * SIZE + x
        dist[s] = 1
        heapq.heappush(heap, (1, s))
    while heap:
        d, cur = heapq.heappop(heap)
        if d > dist[cur]:
            continue
        for nb in neighbors(cur):
            nd = d + step_cost(h, cur, nb)
            if nd < dist[nb]:
                dist[nb] = nd
                heapq.heappush(heap, (nd, nb))
    return dist


def turns_for(cost, strength):
    m = MOVEMENT[strength]
    return (cost + m - 1) // m


LINE_RE = re.compile(r"^(\d+) (\d+) (\d+) (\d+) \| (.*?)\| (.*?)\| ([0-9.]+)$")
LINE_RE_D = re.compile(r"^(\d+) (\d+) (\d+) \| (.*?)\| (.*?)\| (-?[0-9.]+)$")


def parse(path):
    data = []
    with open(path) as f:
        for line in f:
            m = LINE_RE.match(line.strip())
            if m:
                to_move, s1, s2 = int(m.group(1)), int(m.group(3)), int(m.group(4))
                heights = [int(v) for v in m.group(5).split()]
                pieces = []
                for tok in m.group(6).split():
                    o, st, sq = tok.split(":")
                    pieces.append((int(o), int(st), int(sq)))
                data.append((to_move, s1, s2, heights, pieces, float(m.group(7))))
                continue
            m = LINE_RE_D.match(line.strip())
            if m:
                to_move, s1, s2 = int(m.group(1)), int(m.group(2)), int(m.group(3))
                heights = [int(v) for v in m.group(4).split()]
                pieces = []
                for tok in m.group(5).split():
                    o, st, sq = tok.split(":")
                    pieces.append((int(o), int(st), int(sq)))
                data.append((to_move, s1, s2, heights, pieces, float(m.group(6))))
    return data


def agg(data):
    """Agrégats param-indépendants par position (côté 1 / côté 2)."""
    out = []
    for (to_move, s1, s2, h, pieces, z) in data:
        costs = {1: exit_costs(h, 1), 2: exit_costs(h, 2)}
        nobody_out = (s1 + s2) == 0
        S = {1: 0, 2: 0}      # Σ Force (base)
        A = {1: 0.0, 2: 0.0}  # Σ (span-t)+
        B = {1: 0.0, 2: 0.0}  # Σ (span-t)+·Force
        C = {1: 0.0, 2: 0.0}  # Σ sentinelles (span-t)+²
        H = {1: 0.0, 2: 0.0}  # Σ Force menacée (hang)
        first = {1: math.inf, 2: math.inf}
        sentinel_t = {1: math.inf, 2: math.inf}
        for (o, st, sq) in pieces:
            t = turns_for(costs[o][sq], st)
            first[o] = min(first[o], t)
            S[o] += st
            pf = max(0, PROG_SPAN - t)
            A[o] += pf
            B[o] += pf * st
            if st == 3:
                sentinel_t[o] = min(sentinel_t[o], t)
                C[o] += max(0, SENT_SPAN - t) ** 2
            # hang : un voisin ennemi gagne le combat
            for (eo, est, esq) in pieces:
                if eo == o or esq not in neighbors(sq):
                    continue
                ha, ht = h[esq], h[sq]
                atk = est + (1 if ha > ht else (-1 if ha < ht else 0))
                dfn = st
                for (ao, ast, asq) in pieces:
                    if asq in neighbors(sq) and asq not in (esq, sq):
                        if ao == eo:
                            atk += ast
                        else:
                            dfn += 1
                if atk >= dfn:
                    H[o] += st
                    break
        # rdif : différentiel de plies
        def plies(owner, t):
            if t is math.inf:
                return math.inf
            return 2 * t - 1 if to_move == owner else 2 * t

        d_plies = None
        if nobody_out and sentinel_t[1] is not math.inf and sentinel_t[2] is not math.inf:
            d_plies = plies(2, sentinel_t[2]) - plies(1, sentinel_t[1])
        out.append((to_move, s1, s2, S, A, B, C, H, d_plies, z))
    return out


def eval_side(f, side, p, scale):
    """eval côté `side` (points internes), et aussi /scale pour sigmoid."""
    to_move, s1, s2, S, A, B, C, H, d_plies, _ = f
    o = 2 if side == 1 else 1
    my_sc = s1 if side == 1 else s2
    op_sc = s2 if side == 1 else s1
    total = p["score"] * (my_sc - op_sc)
    me = p["base"] * S[side] + p["prog_a"] * A[side] + p["prog_b"] * B[side]
    op = p["base"] * S[o] + p["prog_a"] * A[o] + p["prog_b"] * B[o]
    me -= p["hang_w"] * H[side]
    op -= p["hang_w"] * H[o]
    if (s1 + s2) == 0:
        me += p["sent_w"] * C[side]
        op += p["sent_w"] * C[o]
        if d_plies is not None:
            r = p["race"]
            grad = max(-r, min(r, d_plies * (r / 4.0)))
            if side == 1:
                me += grad
            else:
                op += grad
    total += me - op
    total += p["tempo"] if to_move == side else -p["tempo"]
    return total / scale


def loss(rows, p, scale):
    eps = 1e-9
    L = 0.0
    for f in rows:
        side = f[0]
        e = eval_side(f, side, p, scale)
        q = 1.0 / (1.0 + math.exp(-max(-30.0, min(30.0, e))))
        z = f[9]
        L += -(z * math.log(q + eps) + (1 - z) * math.log(1 - q + eps))
    return L / len(rows)


def loss_mse(rows, p, scale):
    """Distillation : MSE entre eval linéaire et score de recherche.
    Les positions au voisinage de la victoire (|z| ≥ WIN/2) sont exclues :
    elles encodent WIN - ply, pas un avis positionnel."""
    L = 0.0
    n = 0
    for f in rows:
        z = f[9]
        if abs(z) >= WIN / 2:
            continue
        side = f[0]
        e = eval_side(f, side, p, 1.0)
        L += (e - z) ** 2
        n += 1
    return L / max(n, 1)


def main():
    path = sys.argv[1]
    iters = 3000
    if "--iters" in sys.argv:
        iters = int(sys.argv[sys.argv.index("--iters") + 1])
    distill_mode = "--distill" in sys.argv
    data = parse(path)
    rows = agg(data)
    print(f"{len(rows)} positions, {len(data)} lignes, mode={'distill' if distill_mode else 'wl'}")

    p = dict(INIT)
    scale = 250.0 if not distill_mode else 1.0
    Lfn = loss if not distill_mode else (lambda r, pp, sc: loss_mse(r, pp, sc))
    L0 = Lfn(rows, p, scale)
    print(f"loss initiale {L0:.2f}")
    lr = {"score": 1.0, "base": 0.4, "prog_a": 0.05, "prog_b": 0.05, "sent_w": 0.05,
          "hang_w": 0.3, "race": 2.0, "tempo": 0.1}
    m = {k: 0.0 for k in PARAMS}
    v = {k: 0.0 for k in PARAMS}
    b1, b2, eps = 0.9, 0.999, 1e-8
    L0 = loss(rows, p, scale)
    print(f"loss initiale {L0:.5f} (scale={scale:.0f})")
    h = 1e-3
    for it in range(1, iters + 1):
        g = {}
        for k in PARAMS:
            pp = dict(p)
            step = max(h, abs(p[k]) * h)
            pp[k] += step
            lp = Lfn(rows, pp, scale)
            g[k] = (lp - Lfn(rows, p, scale)) / step
        for k in PARAMS:
            m[k] = b1 * m[k] + (1 - b1) * g[k]
            v[k] = b2 * v[k] + (1 - b2) * g[k] * g[k]
            mh = m[k] / (1 - b1 ** it)
            vh = v[k] / (1 - b2 ** it)
            p[k] -= lr[k] * mh / (math.sqrt(vh) + eps)
        # garder les signes physiques
        p["hang_w"] = max(0.0, p["hang_w"])
        for k in PARAMS:
            if k != "tempo":
                p[k] = max(0.0, p[k])
        if it % 500 == 0:
            print(f"  it {it}: loss {Lfn(rows, p, scale):.2f}")
    print("\nparamètres tunés :")
    for k in PARAMS:
        cur = INIT[k]
        print(f"  {k:7s} {cur:6.1f} → {p[k]:7.2f}")
    print(f"loss finale {Lfn(rows, p, scale):.2f} vs initiale {L0:.2f}")
    # suggestion d'arrondis entiers
    print("\nsuggestion (arrondi) :")
    print(", ".join(f"{k}={max(1, round(p[k]))}" for k in PARAMS))


if __name__ == "__main__":
    main()
