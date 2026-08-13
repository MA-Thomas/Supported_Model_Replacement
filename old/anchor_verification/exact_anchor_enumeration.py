from fractions import Fraction as F
from itertools import product, permutations
from collections import Counter

def ap(labels, pi):
    npos = sum(labels); nneg = len(labels) - npos
    tp = fp = 0; tot = F(0)
    for y in labels:
        if y == 1: tp += 1
        else: fp += 1
        if y == 1:
            r = F(tp, npos); f = F(fp, nneg)
            tot += pi*r/(pi*r + (1-pi)*f)
    return tot/npos

def cnap(labels, pi):
    return (ap(labels, pi) - pi)/(1 - pi)

def robust(labels, PS):
    return min(cnap(labels, pi) for pi in PS)

def jitter_dist(sample):
    """sample: list of (score,label). Return {label_tuple: prob} over ties resolved uniformly."""
    idx = range(len(sample))
    ok = [p for p in permutations(idx)
          if all(sample[p[i]][0] >= sample[p[i+1]][0] for i in range(len(p)-1))]
    c = Counter(tuple(sample[i][1] for i in p) for p in ok)
    return {k: F(v, len(ok)) for k, v in c.items()}

def label_arrangements(scores, npos):
    """All ways to put npos positives on the fixed score vector, uniform."""
    n = len(scores)
    out = []
    for pos in permutations(range(n)):
        pass
    from itertools import combinations
    combs = list(combinations(range(n), npos))
    for c in combs:
        lab = [1 if i in c else 0 for i in range(n)]
        out.append((list(zip(scores, lab)), F(1, len(combs))))
    return out

def strat_boot(vec):
    """Stratified bootstrap: resample n+ from positives, n- from negatives, with replacement."""
    pos = [u for u in vec if u[1] == 1]
    neg = [u for u in vec if u[1] == 0]
    w = F(1, len(pos)**len(pos) * len(neg)**len(neg))
    out = []
    for pc in product(pos, repeat=len(pos)):
        for nc in product(neg, repeat=len(neg)):
            out.append((list(pc) + list(nc), w))
    return out

def S2(dist):
    """dist: {z: prob}. Return E[min(Z1,Z2)] for iid Z1,Z2."""
    items = sorted(dist.items())
    tot = F(0)
    for i, (zi, pi_) in enumerate(items):
        for j, (zj, pj) in enumerate(items):
            tot += pi_*pj*min(zi, zj)
    return tot

def merge(pairs):
    d = {}
    for z, p in pairs:
        d[z] = d.get(z, F(0)) + p
    return d

# ---------- population reference law: fresh labels + jitter, no bootstrap ----------
def a0_population(scores, npos, PS):
    pairs = []
    for vec, wv in label_arrangements(scores, npos):
        for seq, wj in jitter_dist(vec).items():
            pairs.append((robust(list(seq), PS), wv*wj))
    return S2(merge(pairs))

# ---------- estimator: permute labels, then stratified bootstrap + jitter ----------
def a0_bootstrap(scores, npos, PS):
    """Paper Eq.21: S2 applied WITHIN each permutation, then averaged over permutations."""
    arrs = label_arrangements(scores, npos)
    tot = F(0)
    for vec, wv in arrs:
        pairs = []
        for samp, wb in strat_boot(vec):
            for seq, wj in jitter_dist(samp).items():
                pairs.append((robust(list(seq), PS), wb*wj))
        tot += wv*S2(merge(pairs))
    return tot

vectors = {
    "distinct  (4,3,2,1)": [4,3,2,1],
    "two blocks(2,2,1,1)": [2,2,1,1],
    "one block (1,1,1,1)": [1,1,1,1],
    "3+1       (2,2,2,1)": [2,2,2,1],
}

for name, PS in [("PS={1/2}", [F(1,2)]),
                 ("PS={0.1,0.3,0.5}", [F(1,10), F(3,10), F(1,2)])]:
    print("="*74); print(name, "   n+=2, n-=2, K=2"); print("="*74)
    print(f"{'score vector':22} {'a0 population':>22} {'a0 through bootstrap':>24}")
    for nm, s in vectors.items():
        p = a0_population(s, 2, PS)
        b = a0_bootstrap(s, 2, PS)
        print(f"{nm:22} {str(p):>13} ={float(p):7.4f} {str(b):>14} ={float(b):7.4f}")
    print()
