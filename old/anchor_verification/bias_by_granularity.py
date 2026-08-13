import numpy as np
rng=np.random.default_rng(99)
def robust_rows(lab,PS):
    npos=int(lab[0].sum()); n=lab.shape[1]; nneg=n-npos
    tp=np.cumsum(lab,1,dtype=np.float64); fp=np.cumsum(1-lab,1,dtype=np.float64)
    r=tp/npos; f=fp/nneg; best=None
    for pi in PS:
        p=pi*r/(pi*r+(1-pi)*f); ap=(p*lab).sum(1)/npos; c=(ap-pi)/(1-pi)
        best=c if best is None else np.minimum(best,c)
    return best
def S2(z):
    z=np.sort(z); B=len(z); w=(B-1-np.arange(B)).astype(float); return 2.0*np.dot(w,z)/(B*(B-1))
def boot_z(sc,lab,B,PS):
    n=len(sc); npos=int(lab.sum()); pidx=np.where(lab==1)[0]; nidx=np.where(lab==0)[0]
    pick=np.concatenate([rng.choice(pidx,(B,npos)),rng.choice(nidx,(B,n-npos))],1)
    s=sc[pick]+1e-9*rng.random((B,n)); l=lab[pick].astype(float)
    return robust_rows(np.take_along_axis(l,np.argsort(-s,axis=1),1),PS)
PS=(0.5,)
print("PART 3 - does the ESTIMATOR BIAS depend on granularity when models carry signal?")
print("Each granularity has its own true S^L. A distortion-free board needs equal bias.\n")
for n,delta in [(60,1.0),(200,1.0),(200,1.5)]:
    m=n//2
    print(f"  n={n}, delta={delta}   {'granularity':>14} {'true S^L':>9} {'E[est]':>9} {'bias':>9} {'se':>7}")
    for name,lev in [("continuous",None),("16 levels",16),("4 levels",4),("2 levels",2),("1 level",0)]:
        co=(lambda x:x) if lev is None else ((lambda x: np.zeros_like(x)) if lev==0 else (lambda x: np.floor(x*lev)/lev))
        zs=[]
        for _ in range(10):
            R=20000
            s=co(np.concatenate([rng.normal(delta,1,(R,m)),rng.normal(0,1,(R,n-m))],1))+1e-9*rng.random((R,n))
            lab=np.concatenate([np.ones((R,m)),np.zeros((R,n-m))],1)
            zs.append(robust_rows(np.take_along_axis(lab,np.argsort(-s,axis=1),1),PS))
        truth=S2(np.concatenate(zs))
        es=[]
        for _ in range(300):
            sc=co(np.concatenate([rng.normal(delta,1,m),rng.normal(0,1,n-m)]))
            lab=np.concatenate([np.ones(m),np.zeros(n-m)])
            es.append(S2(boot_z(sc,lab,400,PS)))
        E=np.mean(es); se=np.std(es)/np.sqrt(len(es))
        print(f"  {'':>13} {name:>14} {truth:9.4f} {E:9.4f} {E-truth:9.4f} {se:7.4f}")
    print()
