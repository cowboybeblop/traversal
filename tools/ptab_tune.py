#!/usr/bin/env python3
"""Tuning de la table de progression PROG[kind][t] par distillation.

Méthode : pour chaque position du corpus distill (labels = scores de recherche
v5.3, excluant |z| ≥ WIN/2), calculer le résidu (z − eval) et le distribuer
uniformément sur les contributions PROG[k][t] présentes (une pièce = une
contribution). PROG[k][t] += η · résidu_moyen(k,t), quelques passes.

Usage : python3 tools/ptab_tune.py corpus.txt
"""

import sys

sys.path.insert(0, "tools")
import texel


def main():
    path = sys.argv[1]
    data = texel.parse(path)
    p = dict(texel.INIT)
    PROG_SPAN, PA, PB = 13, 4, 3

    # base linéaire
    prog = [[max(0, PROG_SPAN - t) * (PA + PB * (k + 1)) for t in range(16)] for k in range(3)]

    # précalcul par position : liste (k, t) et résidu
    samples = []
    for pos in data:
        z = pos[5]
        if abs(z) >= texel.WIN / 2:
            continue
        # features
        to_move, s1, s2, h, pieces = pos[0], pos[1], pos[2], pos[3], pos[4]
        costs = {1: texel.exit_costs(h, 1), 2: texel.exit_costs(h, 2)}
        side = to_move
        other = 2 if side == 1 else 1
        # eval complète avec table courante (itérative)
        kt = []
        for (o, st, sq) in pieces:
            t = texel.turns_for(costs[o][sq], st)
            kt.append((o, st - 1, min(t, 15)))
        samples.append((side, s1, s2, h, pieces, kt, z))

    print(f"{len(samples)} échantillons")

    def eval_with(side, s1, s2, pieces, kt, table, hang_w, base_v):
        other = 2 if side == 1 else 1
        my_sc = s1 if side == 1 else s2
        op_sc = s2 if side == 1 else s1
        total = p["score"] * (my_sc - op_sc)
        for (o, k, t) in kt:
            v = p["base"] * (k + 1) + table[k][t]
            if o == side:
                total += v
            else:
                total -= v
        # (sent, hang, rdif omis : second ordre, constants entre les deux camps le plus souvent)
        total += p["tempo"]
        return total

    eta = 0.35
    for it in range(6):
        # résidus par (k,t)
        sums = [[0.0] * 16 for _ in range(3)]
        cnts = [[0] * 16 for _ in range(3)]
        for (side, s1, s2, h, pieces, kt, z) in samples:
            e = eval_with(side, s1, s2, pieces, kt, prog, p["hang_w"], p["base"])
            r = (z - e) / max(1, len(kt))
            for (o, k, t) in kt:
                sgn = 1 if o == side else -1
                sums[k][t] += sgn * r
                cnts[k][t] += 1
        for k in range(3):
            for t in range(16):
                if cnts[k][t] >= 5:
                    prog[k][t] += eta * sums[k][t] / cnts[k][t]

    print("table apprise PROG[kind][t] :")
    for k in range(3):
        print(f"  kind {k+1}: {[round(v) for v in prog[k]]}")
    print("diff vs linéaire (kind 1..3) :")
    base = texel.ptab_lin(13, 4, 3)
    for k in range(3):
        print(f"  kind {k+1}: {[round(prog[k][t] - base[k][t]) for t in range(16)]}")


if __name__ == "__main__":
    main()
