#!/usr/bin/env python3
"""Feasibility benchmark for the same-design (2540/5698) AUROC case-mix calculation.

Purpose
-------
Answers the question "can the exact adversarial calculation scale to the full
holdout design?" *before* any Rust work is committed. It does not replace the
certified Rust optimizer; it establishes (a) where the Gamma breakdown lands at
full design, and (b) how tight a cheap separable bound is as a function of Gamma,
which determines how much branch-and-bound effort is actually required.

Reformulation used
------------------
Every vertex of the capped simplex puts the cap u_+ = Gamma/n_+ on a set S of
size floor(n_+/Gamma), residual weight on at most one more case, and zero
elsewhere.  Writing E for the excluded positives and H for the excluded
negatives, and ignoring the residual term,

    Z(E,H) = u_+ u_- [ T - sum_{i in E} R_i - sum_{k in H} C_k + sum_{E x H} G_ik ]

with T the grand sum, R the row sums and C the column sums of the gain matrix.
The first three terms are separable; only the cross term couples the classes.
Because |G_ik| <= 1 the cross term is bounded by |E||H|, giving a free global
lower bound.  The alternating best response gives a feasible upper bound.

Caveat: the separable bound is the ROOT NODE of a branch and bound.  It is not a
substitute for one in the regime where the two bounds do not already straddle
the policy threshold.
"""

from __future__ import annotations

import time
from pathlib import Path

import numpy as np
import pandas as pd

PREDICTIONS = (
    Path(__file__).resolve().parent
    / "outputs"
    / "predictions"
    / "temporal_holdout_predictions.csv"
)

COMPARISONS = (
    ("score_full_logistic", "score_demographic_logistic", "full logistic - demographic logistic"),
    ("score_random_forest", "score_full_logistic", "random forest - full logistic"),
)

GAMMAS = (1.01, 1.02, 1.05, 1.10, 1.15, 1.18, 1.20, 1.50, 2.00, 3.00)


def gain_matrix(pos: pd.DataFrame, neg: pd.DataFrame, a: str, b: str) -> np.ndarray:
    """Pairwise gain matrix G_ik = phi_A(i,k) - phi_B(i,k), half credit for ties."""
    sa_p, sb_p = pos[a].to_numpy(), pos[b].to_numpy()
    sa_n, sb_n = neg[a].to_numpy(), neg[b].to_numpy()
    ga = (sa_p[:, None] > sa_n[None, :]).astype(np.int8) * 2
    ga += (sa_p[:, None] == sa_n[None, :]).astype(np.int8)
    gb = (sb_p[:, None] > sb_n[None, :]).astype(np.int8) * 2
    gb += (sb_p[:, None] == sb_n[None, :]).astype(np.int8)
    return (ga - gb).astype(np.float32) / 2.0


def vertex_sizes(gamma: float, n: int) -> tuple[float, int]:
    cap = gamma / n
    return cap, int(np.floor(1.0 / cap))


def separable_lower_bound(g: np.ndarray, gamma: float) -> tuple[float, float]:
    """Return (separable term, cross-term slack). Lower bound is their difference."""
    n_pos, n_neg = g.shape
    up, full_pos = vertex_sizes(gamma, n_pos)
    un, full_neg = vertex_sizes(gamma, n_neg)
    excluded_pos, excluded_neg = n_pos - full_pos, n_neg - full_neg
    row_top = np.sort(g.sum(1))[::-1].cumsum()
    col_top = np.sort(g.sum(0))[::-1].cumsum()
    separable = up * un * (
        g.sum()
        - (row_top[excluded_pos - 1] if excluded_pos else 0.0)
        - (col_top[excluded_neg - 1] if excluded_neg else 0.0)
    )
    return float(separable), float(up * un * excluded_pos * excluded_neg)


def alternating_upper_bound(
    g: np.ndarray, gamma: float, restarts: int = 8, seed: int = 0
) -> float:
    """Best-response alternation. Feasible, so a valid upper bound on the infimum."""
    n_pos, n_neg = g.shape
    up, full_pos = vertex_sizes(gamma, n_pos)
    un, full_neg = vertex_sizes(gamma, n_neg)
    rng = np.random.default_rng(seed)
    best = np.inf
    for start in range(restarts):
        if start == 0:
            w_pos = np.full(n_pos, 1.0 / n_pos)
        else:
            w_pos = np.zeros(n_pos)
            w_pos[rng.permutation(n_pos)[:full_pos]] = up
            w_pos[rng.permutation(n_pos)[0]] += 1.0 - w_pos.sum()
        w_neg = np.zeros(n_neg)
        for _ in range(80):
            w_neg = _greedy(w_pos @ g, un, full_neg)
            candidate = _greedy(g @ w_neg, up, full_pos)
            if np.allclose(candidate, w_pos):
                w_pos = candidate
                break
            w_pos = candidate
        best = min(best, float(w_pos @ g @ w_neg))
    return best


def _greedy(coefficients: np.ndarray, cap: float, full_count: int) -> np.ndarray:
    """Exact minimiser of a linear form over a capped simplex."""
    order = np.argsort(coefficients, kind="stable")
    weights = np.zeros(coefficients.size)
    weights[order[:full_count]] = cap
    residual = 1.0 - weights.sum()
    if residual > 1e-12:
        weights[order[full_count]] = residual
    return weights


def main() -> None:
    frame = pd.read_csv(PREDICTIONS)
    pos, neg = frame[frame.label == 1], frame[frame.label == 0]
    print(f"holdout: n+ = {len(pos)}, n- = {len(neg)}")
    for a, b, name in COMPARISONS:
        started = time.perf_counter()
        g = gain_matrix(pos, neg, a, b)
        n_pos, n_neg = g.shape
        print(f"\n=== {name}")
        print(f"    ordinary paired AUROC difference = {g.sum() / (n_pos * n_neg):+.6f}")
        print(
            f"    {'Gamma':>6} {'|E|':>6} {'|H|':>6} "
            f"{'lower':>10} {'upper':>10} {'gap':>9}"
        )
        for gamma in GAMMAS:
            separable, slack = separable_lower_bound(g, gamma)
            lower = separable - slack
            upper = alternating_upper_bound(g, gamma)
            _, full_pos = vertex_sizes(gamma, n_pos)
            _, full_neg = vertex_sizes(gamma, n_neg)
            print(
                f"    {gamma:>6.2f} {n_pos - full_pos:>6d} {n_neg - full_neg:>6d} "
                f"{lower:>+10.5f} {upper:>+10.5f} {upper - lower:>9.5f}"
            )
        print(f"    elapsed {time.perf_counter() - started:.1f}s")


if __name__ == "__main__":
    main()
