# Self-gated adaptive Hill-q L2 selection v2

This is a post-hoc model-development selection performed before a subsequent
directed round robin. It evaluated a fixed list of **8** Hill orders crossed
with **31753** `(c, kappa)` pairs, for **254024** triples per component/metric group.

The complete joint grid was selected independently under two policies: PDAC-only
and equal-weight PDAC, COVID SPIKE, and COVID NONSPIKE. The aggregation solves
the authoritative implicit self-gated equation by bisection and uses
`ln(1e-12)` for uncomputed candidates.

## Selected component- and metric-specific parameters

```text
policy                     branch  model            q_id  c        kappa    mean_fractional_rank
all_contexts_equal_weight  pr      focal_hla        q0    -0.65    0.0003112274226232321 0.08474626313365326
all_contexts_equal_weight  pr      full_hla         q0    -0.65    0.005318295896944991 0.04644986215158995
all_contexts_equal_weight  pr      full_q_mono_pn   q0    -1.25    0.0020647823694200057 0.07323155777232769
all_contexts_equal_weight  pr      mono_q_full_pn   q0    -0.25    0.005318295896944991 0.13585252778947837
all_contexts_equal_weight  pr      old_monoallelic  q0p5  -0.1     0.0009686250859269985 0.07108345832201546
all_contexts_equal_weight  roc     focal_hla        q0    -0.65    0.0003112274226232321 0.027897736294220075
all_contexts_equal_weight  roc     full_hla         q0    -0.65    0.00013282183136484435 0.024761537341106907
all_contexts_equal_weight  roc     full_q_mono_pn   q0    -0.95    0.0017088133982921087 0.037055043572169974
all_contexts_equal_weight  roc     mono_q_full_pn   q0    -0.15    0.0017088133982921087 0.035697554945812
all_contexts_equal_weight  roc     old_monoallelic  q0    -0.15    0.00021316631165338437 0.03872877652810966
pdac_only                  pr      focal_hla        q1p5  -1.25    0.0033137778446013506 0.000013778279919534845
pdac_only                  pr      full_hla         q2    -1.2     0.0005490561801685379 0.00003542986265023246
pdac_only                  pr      full_q_mono_pn   q1p5  -1.1     0.0005490561801685379 0.00003542986265023246
pdac_only                  pr      mono_q_full_pn   qinf  -0.4     0.0040040826438662224 0.000076764702408837
pdac_only                  pr      old_monoallelic  qinf  -0.35    0.005318295896944991 0.0
pdac_only                  roc     focal_hla        q1    -1.25    0.0033137778446013506 0.00008857465662558115
pdac_only                  roc     full_hla         q2    -1.2     0.0005490561801685379 0.00003542986265023246
pdac_only                  roc     full_q_mono_pn   q1p5  -1.1     0.0005490561801685379 0.00003542986265023246
pdac_only                  roc     mono_q_full_pn   qinf  -0.4     0.0040040826438662224 0.000076764702408837
pdac_only                  roc     old_monoallelic  qinf  -0.3     0.0014142135623730968 0.00005511311967813938
```

The compressed full surfaces are retained under `surfaces/`. Because the same
cohorts supplied selection, later tournament evidence using these
scores is conditional post-selection evidence. COVID results for PDAC-only are
directional transport evidence rather than selection inputs.
