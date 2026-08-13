import numpy as np
rng = np.random.default_rng(11)

def robust_rows(lab, PS):
    npos=int(lab[0].sum()); n=lab.shape[1]; nneg=n-npos
    tp=np.cumsum(lab,1,dtype=np.float64); fp=np.cumsum(1-lab,1,dtype=np.float64)
    r=tp/npos; f=fp/nneg; best=None
    for pi in PS:
        p=pi*r/(pi*r+(1-pi)*f); ap=(p*lab).sum(1)/npos; c=(ap-pi)/(1-pi)
        best=c if best is None else np.minimum(best,c)
    return best

def S2(z):
    z=np.sort(z); B=len(z); w=(B-1-np.arange(B)).astype(float)
    return 2.0*np.dot(w,z)/(B*(B-1))

def boot_z(scores, lab, B, PS):
    n=len(scores); npos=int(lab.sum())
    pidx=np.where(lab==1)[0]; nidx=np.where(lab==0)[0]
    pick=np.concatenate([rng.choice(pidx,(B,npos)),rng.choice(nidx,(B,n-npos))],1)
    s=scores[pick]+1e-9*rng.random((B,n)); l=lab[pick].astype(float)
    return robust_rows(np.take_along_axis(l,np.argsort(-s,axis=1),1),PS)

def a0_pop(n,npos,PS,R=400000,chunk=25000):
    zs=[]
    for _ in range(R//chunk):
        lab=np.zeros((chunk,n)); lab[:,:npos]=1; lab=rng.permuted(lab,axis=1)
        zs.append(robust_rows(lab,PS))      # random ranking == uniform label sequence
    return S2(np.concatenate(zs))

PS=(0.5,)
print("PART 1 — leaderboard score L_M for models that are PURE NOISE.")
print("Every model here is uninformative, so a correct board reports L_M = 0 for all.\n")
print(f"{'n':>5} {'a0':>8} | {'L: distinct':>12} {'L: blocks/2':>12} {'L: blocks/8':>12} {'L: oneblock':>12} | {'spread':>7}")
for n in [8,32,128,512]:
    m=n//2; a0=a0_pop(n,m,PS)
    outs=[]
    for sc in [list(range(n,0,-1)), [i//2 for i in range(n)][::-1],
               [i//8 for i in range(n)][::-1], [1]*n]:
        sc=np.asarray(sc,float); tot=[]
        for _ in range(600):
            lab=np.zeros(n); lab[rng.choice(n,m,replace=False)]=1
            tot.append(S2(boot_z(sc,lab,600,PS)))
        S=np.mean(tot); outs.append((S-a0)/(1-a0))
    print(f"{n:>5} {a0:8.4f} | {outs[0]:12.4f} {outs[1]:12.4f} {outs[2]:12.4f} {outs[3]:12.4f} | {max(outs)-min(outs):7.4f}")

print("\nPART 2 — two models with REAL signal and identical population S^L.")
print("Model A: coarse scores (ties).  Model B: A + tiny outcome-blind noise (uninformative refinement).")
print("Under the jitter convention these have the SAME population law, so any gap is estimator artifact.\n")
print(f"{'n':>5} {'delta':>6} {'true S^L':>9} | {'E[est] A coarse':>16} {'E[est] B refined':>17} | {'bias A':>8} {'bias B':>8} {'gap':>7}")
for n,delta,nlev in [(40,1.0,4),(80,1.0,4),(200,1.0,4),(200,1.5,8)]:
    m=n//2
    def sample_scores(size_p,size_n):
        return rng.normal(delta,1,size_p), rng.normal(0,1,size_n)
    def coarsen(x): return np.floor(x*nlev)/nlev
    # truth: fresh evaluations under R_L, jitter-resolved
    zs=[]
    for _ in range(16):
        R=25000
        sp,sn=rng.normal(delta,1,(R,m)),rng.normal(0,1,(R,n-m))
        s=coarsen(np.concatenate([sp,sn],1))+1e-9*rng.random((R,n))
        lab=np.concatenate([np.ones((R,m)),np.zeros((R,n-m))],1)
        zs.append(robust_rows(np.take_along_axis(lab,np.argsort(-s,axis=1),1),PS))
    truth=S2(np.concatenate(zs))
    estA=[];estB=[]
    for _ in range(400):
        sp,sn=sample_scores(m,n-m)
        sc=coarsen(np.concatenate([sp,sn])); lab=np.concatenate([np.ones(m),np.zeros(n-m)])
        estA.append(S2(boot_z(sc,lab,500,PS)))
        estB.append(S2(boot_z(sc+1e-6*rng.random(n),lab,500,PS)))
    A,B=np.mean(estA),np.mean(estB)
    print(f"{n:>5} {delta:6.1f} {truth:9.4f} | {A:16.4f} {B:17.4f} | {A-truth:8.4f} {B-truth:8.4f} {B-A:7.4f}")
