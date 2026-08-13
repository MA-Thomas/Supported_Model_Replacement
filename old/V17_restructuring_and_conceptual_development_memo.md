# Memo: Motivation and Design for Version 17

## Purpose of this memo

This memo records the motivation for developing Version 17 of *Supported Model Replacement Across Target Prevalences*, and translates that motivation into concrete recommendations for the manuscript's structure, definitions, interpretation, computation, illustrations, and reporting guidance.

The principal change is not merely presentational. Version 16 treats multiple observed evaluations and computational replications primarily as two alternative sources of a finite effect list:

- \(M\) separately conducted empirical evaluations supply an observed list; or
- \(J\) computational replications of one observed evaluation supply a projected list.

That presentation correctly distinguishes empirical evidence from computational projection, but it does not express a scientifically natural third setting: \(M>1\) empirical evaluations, each additionally challenged by \(J\) computational replications. The proposed Version 17 construction makes this setting central and treats the two analyses already developed in Version 16 as boundary cases of one hierarchy.

The proposed hierarchy has three levels:

1. **Condition challenge:** within an evaluation or computational replication, the claimed advantage must survive every declared condition \(c\in\mathcal C\), such as every target prevalence in \(\Pi_\star\).
2. **Within-evaluation computational challenge:** within each empirical evaluation, the claimed advantage must survive selected computational recombinations generated under a declared resampling procedure.
3. **Across-evaluation empirical challenge:** the claimed advantage must survive selected groups of separately conducted empirical evaluations admitted by the evaluation regime \(\mathcal R\).

Version 17 should preserve the evidential distinction between these levels. Computational replication adds a declared challenge conditional on the evidence contained in an empirical evaluation; it does not add empirical evaluations, new case types, new environments, or new failure mechanisms.

The recommended construction is **observed-anchored nested support**. Every nested challenge must include the retained effect actually observed in each selected empirical evaluation, together with selected computational effects generated from it. This choice ensures that computational replication can only preserve or reduce the support credited to the observed evidence. It therefore makes the phrase “additional computational challenge” true both conceptually and mathematically.

When a scientifically defensible within-evaluation resampling procedure is available, Version 17 should make this full nested construction the primary assessment for both \(M=1\) and \(M>1\). Direct observed support remains indispensable, but its primary role changes: it is the necessary empirical gate that a claim must pass before computational challenge can matter. Omitting the computational layer yields a reduced, less severe assessment. That reduction may be appropriate when resampling is scientifically unjustified or computationally infeasible, but it should be declared in advance and labeled as addressing a narrower claim.

No changes to Version 16 are made by this memo. It is a design document for the subsequent preparation of Version 17.

---

## 1. Why Version 17 is warranted

### 1.1 The central conceptual limitation of the Version 16 presentation

Version 16 introduces a generic finite retained-effect list

\[
\mathbf z=(z_1,\ldots,z_L)
\]

and defines replication-survival support as

\[
S_K(\mathbf z)
=
\binom{L}{K}^{-1}
\sum_{\substack{I\subseteq\{1,\ldots,L\}\\|I|=K}}
\min_{\ell\in I}z_\ell.
\]

The manuscript then instantiates \(L\) as either:

- \(M\), when the entries are retained effects from separately conducted empirical evaluations; or
- \(J\), when the entries are retained effects from computational replications of one observed evaluation.

This is mathematically economical, but the generic list obscures a substantive difference between its possible indices. An empirical-evaluation index and a computational-replication index do not represent the same kind of attempted refutation. The former records a new empirical test admitted by \(\mathcal R\); the latter records a computational recombination of evidence already contained in one test.

More importantly, presenting these instantiations mainly as alternatives leaves out their natural composition. If several empirical evaluations exist, computational resampling can still expose fragility within each one. The resampling does not strengthen the empirical basis, but it can deepen the declared challenge applied to that basis.

### 1.2 The Popperian motivation for a hierarchy

The manuscript's falsification-oriented language supports a hierarchical construction. A replacement claim may be challenged through different routes:

- a target condition may expose a weakness within a fixed evaluation;
- a computational perturbation may expose dependence on the realized composition of represented units;
- a separately conducted evaluation may expose a failure in another admissible collection of units or context.

These challenges differ in evidential status, but they share the same noncompensation principle: the claim should receive credit only for the effect retained after every declared level of challenge has been applied.

The clean intuition is:

> Each empirical evaluation supplies a distinct test of the replacement claim. Computational replications stress-test the result of each empirical test using only the units and failure mechanisms represented in that evaluation. A nested challenge selects several distinct empirical evaluations and several computational perturbations within each selected evaluation, and credits the claim only with the weakest retained advantage appearing anywhere in that combined challenge.

This formulation does not claim that computational perturbations are empirical falsifications. They are constructed attempts at refutation under a declared computational law. Their role is conditional and limited, but it is still scientifically useful.

### 1.3 Why the observed effect should remain an anchor

An unanchored nested projection would replace each empirical evaluation's retained effect by effects produced under its resampling law. Such a projection is coherent, but it is not guaranteed to be more adverse than the direct observed analysis. Resampling can produce effects that average above or below the retained effect calculated from the original evaluation.

If computational replication is described as an *additional* challenge, this lack of monotone severity is undesirable. The observed evaluation should not disappear when its resampling projection is introduced.

Observed anchoring resolves the problem. Each selected empirical evaluation contributes:

- its observed retained effect, which is always present; and
- selected computational retained effects, which are added to the same challenge.

The minimum over the combined entries cannot exceed the observed anchor. Consequently, nested support cannot exceed direct observed support. A failed observed evaluation cannot be rescued by favorable computational recombinations.

This property closely matches the manuscript's finite-evidence position: the evidence actually obtained remains binding, while computation can expose additional fragility but cannot overwrite the observed result.

It also creates a sequential falsification rule. If direct observed support or direct observed literal survival fails its policy threshold, anchored nested support or survival must fail as well. Computational replication is then unnecessary for the verdict. If the observed analysis passes, the computational layer determines whether that empirically supported claim also survives the more severe full assessment. Direct observed support is therefore both a reported diagnostic and a logically necessary gate, while anchored nested support is the primary final criterion whenever the computational challenge belongs to the declared assessment.

### 1.4 Why this is a substantive version change

The proposed change affects more than section order. Version 17 requires:

- two distinct challenge orders, \(K_E\) and \(K_C\);
- a hierarchical extension of the \(S_K\) operator;
- an anchored nested literal-survival functional;
- new reduction, monotonicity, domination, and survival-representation results;
- revised terminology for observed, projected, and anchored nested quantities;
- revised computational methods and Monte Carlo diagnostics;
- revised decision and reporting rules;
- reconsideration of both empirical illustrations; and
- corresponding changes to the abstract, introduction, discussion, conclusion, and appendices.

Version 17 is therefore justified as a distinct manuscript rather than a minor edit to Version 16.

---

## 2. The proposed conceptual architecture

### 2.1 Three nested indices

The Version 17 notation should make the hierarchy visible from the beginning.

Let:

- \(m=1,\ldots,M\) index separately conducted empirical evaluations;
- \(j=1,\ldots,J_m\) index computational replications generated within empirical evaluation \(m\); and
- \(c\in\mathcal C\) index conditions that can be evaluated within a complete empirical or computational evaluation.

In the AP application, \(c=\pi\) and \(\mathcal C=\Pi_\star\). When a common computational replication count is used, write \(J_m=J\) for every \(m\).

The roles of the principal counts and orders should be stated explicitly:

| Symbol | Meaning | What increasing it changes |
|---|---|---|
| \(N_m\) | Units in empirical evaluation \(m\) | The evidence within that evaluation |
| \(M\) | Separately conducted empirical evaluations | The empirical evidence and contexts available |
| \(J_m\) | Computational draws within evaluation \(m\) | Monte Carlo accuracy for its declared computational projection |
| \(K_E\) | Across-evaluation challenge order | Severity of the empirical-level challenge |
| \(K_C\) | Within-evaluation computational challenge order | Severity of the computational challenge within each evaluation |

The manuscript should emphasize that \(J\) and \(K_C\) are not interchangeable. Increasing \(J\) refines a Monte Carlo approximation. Increasing \(K_C\) changes the target functional by requiring more computational effects to survive jointly.

Likewise, \(M\) and \(K_E\) are not interchangeable. Increasing \(M\) adds empirical evaluations. Increasing \(K_E\) changes how strongly the available evaluation list is challenged.

### 2.2 The empirical and computational retained effects

For empirical evaluation \(D_m\), define its observed retained effect

\[
z_m^{\mathrm{obs}}
=
\inf_{c\in\mathcal C}\Delta_c(D_m).
\]

For computational replication \(D_m^{(j)}\) generated from \(D_m\), define

\[
z_{mj}^{\mathrm{comp}}
=
\inf_{c\in\mathcal C}\Delta_c(D_m^{(j)}).
\]

For AP model replacement,

\[
z_m^{A:B,\mathrm{obs}}
=
\inf_{\pi\in\Pi_\star}
\left\{
\widetilde{\operatorname{CNAP}}_{A,\pi}(D_m)
-
\widetilde{\operatorname{CNAP}}_{B,\pi}(D_m)
\right\},
\]

and

\[
z_{mj}^{A:B,\mathrm{comp}}
=
\inf_{\pi\in\Pi_\star}
\left\{
\widetilde{\operatorname{CNAP}}_{A,\pi}(D_m^{(j)})
-
\widetilde{\operatorname{CNAP}}_{B,\pi}(D_m^{(j)})
\right\}.
\]

The condition infimum must be taken separately within the complete observed evaluation and within every complete computational replication. Resampling separately by condition would destroy the identity of a full evaluation profile and must remain prohibited.

### 2.3 The array must not be flattened

The effects form a hierarchical object:

\[
\begin{array}{c|cccc}
&j=1&j=2&\cdots&j=J\\
\hline
D_1&z_{11}&z_{12}&\cdots&z_{1J}\\
D_2&z_{21}&z_{22}&\cdots&z_{2J}\\
\vdots&\vdots&\vdots&\ddots&\vdots\\
D_M&z_{M1}&z_{M2}&\cdots&z_{MJ}
\end{array}
\]

Flattening this array into \(MJ\) exchangeable entries would allow many resamples of one empirical evaluation to masquerade as distinct empirical tests. It would also make the empirical challenge depend arbitrarily on the number of Monte Carlo draws assigned to each row.

Version 17 should state a general preservation principle:

> Every index retains the scientific identity of the process that created it. Computational replication may enrich the challenge within an empirical evaluation, but it may not multiply the empirical-evaluation count.

---

## 3. Retaining and extending the \(S_K\) operator

### 3.1 Preserve the original operator

The existing finite-list operator should remain unchanged:

\[
S_K(\mathbf t)
=
\binom{L}{K}^{-1}
\sum_{\substack{I\subseteq\{1,\ldots,L\}\\|I|=K}}
\min_{\ell\in I}t_\ell.
\]

It remains the foundational “average weakest effect over distinct order-\(K\) subsets” construction. Version 17 should not silently redefine its domain or interpretation.

Instead, the manuscript should introduce an anchored hierarchical extension for the two-level replication structure.

### 3.2 Anchored support within one empirical evaluation

For observed anchor \(u\) and computational list \(\mathbf t=(t_1,\ldots,t_J)\), define

\[
S_{K_C}^{\mathrm{anc}}(u;\mathbf t)
=
\binom{J}{K_C}^{-1}
\sum_{\substack{A\subseteq\{1,\ldots,J\}\\|A|=K_C}}
\min\left\{
u,
\min_{j\in A}t_j
\right\}.
\]

This is an anchored, constrained-subset version of \(S_K\): the observed anchor is mandatory in every challenge, while \(K_C\) computational indices are selected without replacement from the finite Monte Carlo list.

The ordering of operations matters. In general,

\[
S_{K_C}^{\mathrm{anc}}(u;\mathbf t)
\ne
\min\{u,S_{K_C}(\mathbf t)\}.
\]

The minimum must be taken for each complete challenge before the results are averaged.

### 3.3 The full observed-anchored hierarchical operator

Let

\[
\mathbf z^{\mathrm{obs}}
=(z_1^{\mathrm{obs}},\ldots,z_M^{\mathrm{obs}})
\]

and let \(\mathbf Z^{\mathrm{comp}}=(z_{mj}^{\mathrm{comp}})\). For common \(J\), define

\[
\begin{aligned}
&S^{\mathrm{anc}}_{K_E;K_C}
\left(
\mathbf z^{\mathrm{obs}};
\mathbf Z^{\mathrm{comp}}
\right)
\\
&\quad=
\frac{1}
{\binom{M}{K_E}\binom{J}{K_C}^{K_E}}
\sum_{\substack{I\subseteq\{1,\ldots,M\}\\|I|=K_E}}
\;
\sum_{\substack{
(A_m)_{m\in I}\\
A_m\subseteq\{1,\ldots,J\}\\
|A_m|=K_C
}}
\min_{m\in I}
\left\{
z_m^{\mathrm{obs}},
\min_{j\in A_m}z_{mj}^{\mathrm{comp}}
\right\}.
\end{aligned}
\]

The semicolon in \(S^{\mathrm{anc}}_{K_E;K_C}\) is useful: it visually separates the empirical challenge order from the computational challenge order.

For unequal \(J_m\), replace the common factor \(\binom{J}{K_C}^{K_E}\) by

\[
\prod_{m\in I}\binom{J_m}{K_C}
\]

inside the average over empirical subsets, and let each \(A_m\) range over the \(K_C\)-subsets of \(\{1,\ldots,J_m\}\).

For the oriented paired comparison, define

\[
\Theta^{\mathrm{anc}}_{A:B;K_E,K_C}
=
S^{\mathrm{anc}}_{K_E;K_C}
\left(
\mathbf z^{A:B,\mathrm{obs}};
\mathbf Z^{A:B,\mathrm{comp}}
\right).
\]

### 3.4 Why row summaries cannot be composed naively

The desired functional is not

\[
S_{K_E}\left(
S_{K_C}^{\mathrm{anc}}(z_1^{\mathrm{obs}};\mathbf z_1),
\ldots,
S_{K_C}^{\mathrm{anc}}(z_M^{\mathrm{obs}};\mathbf z_M)
\right).
\]

That expression averages within rows before applying cross-row minima. The proposed support instead selects complete nested challenges, takes the minimum across every observed and computational effect in each challenge, and only then averages. Averaging and cross-row minimization do not commute.

This point parallels the manuscript's existing argument that marginal model summaries do not compose into a valid paired advantage. Version 17 should make noncomposition a recurring design principle:

> Apply all operations that define one complete challenge before averaging across challenges.

### 3.5 A probabilistic representation of the computational selection

For exposition, let \(A_m\) be uniform over the distinct \(K_C\)-subsets within row \(m\), independently across selected empirical evaluations, and define

\[
W_m
=
\min\left\{
z_m^{\mathrm{obs}},
\min_{j\in A_m}z_{mj}^{\mathrm{comp}}
\right\}.
\]

Then

\[
S^{\mathrm{anc}}_{K_E;K_C}
=
\binom{M}{K_E}^{-1}
\sum_{\substack{I\subseteq\{1,\ldots,M\}\\|I|=K_E}}
\mathbb E_{\mathrm{comp}}
\left[
\min_{m\in I}W_m
\right].
\]

This representation shows that the outer construction retains the original \(S_K\) logic. The difference is that each empirical index now supplies an anchored computational challenge rather than a single scalar.

The expectation here is induced by the deliberately constructed finite computational-subset selection. It is not a probability law for unseen empirical evaluations.

---

## 4. Anchored literal survival and the magnitude–survival identity

### 4.1 Within-evaluation anchored survival

At level \(s\), let

\[
R_{m,s}
=
\sum_{j=1}^{J}
\mathbf 1\{z_{mj}^{\mathrm{comp}}>s\}.
\]

The fraction of order-\(K_C\) computational subsets in evaluation \(m\) whose entries all clear \(s\) is

\[
\frac{\binom{R_{m,s}}{K_C}}
{\binom{J}{K_C}}.
\]

Observed anchoring requires the empirical retained effect to clear \(s\) as well. Define the row-level anchored survival

\[
q^{\mathrm{anc}}_{m,K_C,s}
=
\mathbf 1\{z_m^{\mathrm{obs}}>s\}
\frac{\binom{R_{m,s}}{K_C}}
{\binom{J}{K_C}}.
\]

If the observed anchor fails the floor, no computational subset can rescue that empirical evaluation.

### 4.2 Across-evaluation anchored survival

Define

\[
\Psi^{\mathrm{anc}}_{K_E;K_C,s}
=
\binom{M}{K_E}^{-1}
\sum_{\substack{I\subseteq\{1,\ldots,M\}\\|I|=K_E}}
\prod_{m\in I}
q^{\mathrm{anc}}_{m,K_C,s}.
\]

At a declared decision floor \(d\),

\[
\Psi^{\mathrm{anc}}_{K_E;K_C,d}
\]

is the fraction of complete nested challenges in which every selected observed anchor and every selected computational effect clears \(d\).

The product over \(m\in I\) is essential. A nested challenge survives only if its within-evaluation computational selection survives in every selected empirical evaluation.

### 4.3 Exact survival representation

Choose finite \(a\) and \(b\) containing every observed and computational effect. For each complete nested challenge, the layer-cake identity gives

\[
T
=
a+\int_a^b\mathbf 1\{T>s\}\,ds.
\]

Averaging over all nested selections yields

\[
\boxed{
S^{\mathrm{anc}}_{K_E;K_C}
\left(
\mathbf z^{\mathrm{obs}};
\mathbf Z^{\mathrm{comp}}
\right)
=
a+
\int_a^b
\Psi^{\mathrm{anc}}_{K_E;K_C,s}\,ds.
}
\]

This identity should be a central Version 17 result. It shows that anchored supported magnitude and anchored literal survival remain two summaries of exactly the same nested challenges:

- supported magnitude integrates survival across all effect levels;
- literal survival evaluates the same survival curve at the policy floor \(d\).

The identity would fail if magnitude were anchored but literal survival ignored the observed anchors. Both quantities must be defined on the same challenge objects.

### 4.4 Elementary-symmetric representation

Let \(e_{K_E}\) denote the elementary symmetric polynomial of degree \(K_E\). Then

\[
\Psi^{\mathrm{anc}}_{K_E;K_C,s}
=
\frac{
e_{K_E}\left(
q^{\mathrm{anc}}_{1,K_C,s},\ldots,
q^{\mathrm{anc}}_{M,K_C,s}
\right)}
{\binom{M}{K_E}}.
\]

This representation is useful both conceptually and computationally. It shows that the empirical-level survival calculation averages products of row-level survival fractions over distinct empirical subsets. It also avoids enumerating every nested selection.

---

## 5. Primary assessment, reduced assessment, and continuity with Version 16

### 5.1 Observed-only support as an empirical gate

It is useful to permit \(K_C=0\) as a formal convention for “no computational challenge,” with

\[
A_m=\varnothing,
\qquad
\min_{j\in\varnothing}z_{mj}^{\mathrm{comp}}=+\infty,
\qquad
\binom{J}{0}=1.
\]

Then

\[
S^{\mathrm{anc}}_{K_E;0}
\left(
\mathbf z^{\mathrm{obs}};
\mathbf Z^{\mathrm{comp}}
\right)
=
S_{K_E}(\mathbf z^{\mathrm{obs}}),
\]

and

\[
\Psi^{\mathrm{anc}}_{K_E;0,d}
=
\psi_{K_E,d}(\mathbf z^{\mathrm{obs}}).
\]

Thus the existing multiple-empirical-evaluation formalism is recovered exactly.

Under the Version 17 hierarchy, this observed-only quantity has two roles. First, it is the direct description of what the empirical evaluations themselves support. Second, it is a necessary gate for the full anchored assessment. Anchor domination implies

\[
S^{\mathrm{anc}}_{K_E;K_C}
\leq
S_{K_E}(\mathbf z^{\mathrm{obs}})
\]

and

\[
\Psi^{\mathrm{anc}}_{K_E;K_C,d}
\leq
\psi_{K_E,d}(\mathbf z^{\mathrm{obs}}).
\]

If the observed-only analysis fails either policy threshold, the full anchored analysis must also return no verdict. If the observed-only analysis passes, it has not yet passed the primary full assessment; it has qualified for the additional computational challenge.

The manuscript should explain \(K_C=0\) as a mathematical convention, not as a substantive “zero-order replication.” In prose, “observed-only analysis” or “observed empirical gate” is clearer. When a scientifically defensible computational procedure is part of the declared assessment, \(K_C=0\) is a reduced, less severe analysis rather than the primary rule.

### 5.2 One empirical evaluation

When \(M=1\), the only empirical challenge order is \(K_E=1\). The anchored one-evaluation support becomes

\[
S^{\mathrm{anc}}_{1;K_C}
=
\binom{J}{K_C}^{-1}
\sum_{\substack{A\subseteq\{1,\ldots,J\}\\|A|=K_C}}
\min\left\{
z_1^{\mathrm{obs}},
\min_{j\in A}z_{1j}^{\mathrm{comp}}
\right\}.
\]

This is a substantive revision of the Version 16 one-evaluation projection, which omits the observed anchor. The Version 16 quantity is recovered by setting the anchor to \(+\infty\):

\[
S^{\mathrm{anc}}_{1;K_C}
\left(+\infty;\mathbf z_1^{\mathrm{comp}}\right)
=
S_{K_C}(\mathbf z_1^{\mathrm{comp}}).
\]

Version 17 should present the unanchored quantity as a computational projection that motivated the development, while adopting the anchored quantity for the primary finite-evidence replacement rule.

### 5.3 Multiple empirical evaluations with computational challenge

When \(M>1\) and \(K_C\geq1\), the new interior case is obtained. This is the setting that Version 16 does not formally develop:

\[
S^{\mathrm{anc}}_{K_E;K_C}
\left(
\mathbf z^{\mathrm{obs}};
\mathbf Z^{\mathrm{comp}}
\right).
\]

The manuscript should emphasize that this is not the same as increasing the empirical list length from \(M\) to \(MJ\). The \(M\) empirical identities remain intact throughout the calculation.

When the within-evaluation resampling procedure is scientifically defensible, this interior construction should be the primary Version 17 assessment. The observed-only result should still be calculated and reported, because it supplies the empirical gate and reveals whether any subsequent failure was introduced by the computational level.

### 5.4 Boundary-case table

An early table would orient readers:

| Evidence and challenge | Operator | Role in Version 17 |
|---|---|---|
| \(M>1, K_C\geq1\) | \(S^{\mathrm{anc}}_{K_E;K_C}\) | Primary full assessment when resampling is defensible |
| \(M=1, K_E=1, K_C\geq1\) | \(S^{\mathrm{anc}}_{1;K_C}\) | Primary one-evaluation assessment when resampling is defensible |
| \(K_C=0\) | \(S_{K_E}(\mathbf z^{\mathrm{obs}})\) | Necessary empirical gate; also a reduced assessment if the computational level is omitted |
| Anchor \(=+\infty\) | Unanchored special case | Version 16-style computational projection |

---

## 6. Core properties to establish in Version 17

### 6.1 Anchor domination

For every nested selection,

\[
\min_{m\in I}
\left\{
z_m^{\mathrm{obs}},
\min_{j\in A_m}z_{mj}^{\mathrm{comp}}
\right\}
\leq
\min_{m\in I}z_m^{\mathrm{obs}}.
\]

Therefore,

\[
S^{\mathrm{anc}}_{K_E;K_C}
\left(
\mathbf z^{\mathrm{obs}};
\mathbf Z^{\mathrm{comp}}
\right)
\leq
S_{K_E}(\mathbf z^{\mathrm{obs}}).
\]

Likewise,

\[
\Psi^{\mathrm{anc}}_{K_E;K_C,d}
\leq
\psi_{K_E,d}(\mathbf z^{\mathrm{obs}}).
\]

These inequalities are central to the interpretation of computational replication as an additional challenge.

### 6.2 Domination of the unanchored nested projection

Because inserting an observed anchor can only lower a selection minimum,

\[
S^{\mathrm{anc}}_{K_E;K_C}
\leq
S^{\mathrm{proj}}_{K_E;K_C},
\]

where the right side denotes the otherwise identical nested construction without mandatory observed anchors. The analogous inequality holds for literal survival.

This does not make the unanchored construction invalid. It clarifies that the two quantities answer different questions.

### 6.3 Challenge monotonicity

Version 17 should extend the existing monotonicity proposition. Subject to the necessary list-length constraints:

- enlarging \(\mathcal C\) cannot increase support;
- increasing \(K_C\) cannot increase support;
- increasing \(K_E\) cannot increase support.

The same monotonicity holds for literal survival at a fixed floor.

The proof can proceed by the same subset-counting logic used in Version 16, applied within rows for \(K_C\) and across rows for \(K_E\). The observed anchors do not interfere because they are included in every relevant selection.

### 6.4 Orientation

The reversed comparison must still be constructed by negating the condition-specific paired profile before taking condition infima, observed anchors, computational effects, and nested minima.

The Version 16 orientation result should have a nested analogue:

\[
\Theta^{\mathrm{anc}}_{A:B;K_E,K_C}
+
\Theta^{\mathrm{anc}}_{B:A;K_E,K_C}
\leq0.
\]

The proof should pair the two orientations on the same observed evaluations, computational replications, empirical subsets, and computational subsets. The construction cannot support positive challenged advantages in both directions.

### 6.5 Noncomposition of marginal model summaries

The existing result that separately challenged marginal model summaries overstate the supported paired advantage should remain. It should now be applied cellwise to:

- each observed empirical evaluation; and
- each computational replication within each evaluation.

Monotonicity of the hierarchical operator then propagates the discrepancy. Pairing must be preserved at every level.

### 6.6 Reference behavior under exchangeable labels

The existing exchangeable-label reference proposition requires revision. A Version 17 statement must specify:

- whether labels are randomized in each observed empirical evaluation before its computational replications are generated;
- whether the resampling procedure is applied conditionally on each randomized evaluation;
- whether computational randomizations are independent across empirical evaluations; and
- which quantities are held fixed by the reference design.

The expected condition-specific paired effect remains zero under suitable full exchangeability and outcome-blind tie handling. Condition infima, observed anchoring, computational minima, and empirical minima are all nonlinear and should again make the expected supported value nonpositive rather than necessarily zero.

The proof should be rewritten for the full hierarchy instead of asserting that the Version 16 proposition transfers automatically.

---

## 7. The revised replacement rule

When a scientifically defensible computational procedure belongs to the declared assessment, the primary two-part policy should apply to the anchored nested quantities:

\[
\Theta^{\mathrm{anc}}_{A:B;K_E,K_C}
>\delta
\qquad\text{and}\qquad
\Psi^{\mathrm{anc}}_{A:B;K_E,K_C,d}
>\gamma.
\]

The interpretation remains:

- \(\delta\) is the smallest supported magnitude sufficient to justify replacement;
- \(d\) is the floor used to define literal survival; and
- \(\gamma\) is the minimum acceptable fraction of complete nested challenges that clear \(d\).

Failure of either requirement returns no verdict. The reversed orientation should be assessed through the same rule, with orientation-specific thresholds if consequences are asymmetric.

### 7.1 Stage 1: the observed empirical gate

First calculate

\[
\theta^{\mathrm{obs}}_{A:B;K_E}
=
S_{K_E}(\mathbf z^{A:B,\mathrm{obs}})
\]

and

\[
\psi^{\mathrm{obs}}_{A:B;K_E,d}
=
\psi_{K_E,d}(\mathbf z^{A:B,\mathrm{obs}}).
\]

The claim passes the empirical gate only if

\[
\theta^{\mathrm{obs}}_{A:B;K_E}>\delta
\qquad\text{and}\qquad
\psi^{\mathrm{obs}}_{A:B;K_E,d}>\gamma.
\]

Because the anchored nested quantities cannot exceed their observed-only counterparts, failure at this stage implies failure of the primary nested rule. The analysis may stop with no verdict; computational replication cannot alter that decision.

### 7.2 Stage 2: the within-evaluation computational challenge

If the observed empirical gate passes, calculate the anchored nested quantities and apply the primary rule

\[
\Theta^{\mathrm{anc}}_{A:B;K_E,K_C}>\delta
\qquad\text{and}\qquad
\Psi^{\mathrm{anc}}_{A:B;K_E,K_C,d}>\gamma.
\]

Passing Stage 1 but failing Stage 2 means that the observed empirical advantage did not survive the declared computational perturbations of the represented units. Passing both stages supports replacement under the full anchored assessment.

The two stages should be interpreted as a falsification sequence:

\[
\text{observed empirical challenge}
\longrightarrow
\text{within-evaluation computational challenge}
\longrightarrow
\text{supported replacement under the full assessment}.
\]

Observed anchoring also has the direct policy consequence:

> If too many observed empirical anchors fail \(d\), no number of favorable computational replications can cause the anchored literal-survival requirement to pass.

This consequence is consistent with the finite-evidence interpretation, but it should be stated explicitly because it makes the anchored rule more demanding than the Version 16 projected rule.

### 7.3 Reduced observed-only assessment

If no scientifically defensible computational procedure is available, or if the computational layer is omitted for a prespecified practical reason, the analysis may stop at the observed-only rule. The resulting verdict must be described as support under the reduced observed-only assessment, not as passage of the full nested rule.

Computational infeasibility can justify this reduced assessment, but the choice should be made before the comparison is examined. Omitting the computational layer after it defeats replacement would weaken the declared challenge in response to the result.

The notation should not describe this primarily as \(J=0\). The number \(J\) controls Monte Carlo approximation after a computational procedure has been declared; \(K_C\) controls the severity of that computational challenge. “No computational challenge” is represented by the formal convention \(K_C=0\), with \(J\) absent or irrelevant. A smaller positive \(J\) yields a less accurate approximation of the same target rather than a substantively weaker target.

---

## 8. Evidential interpretation and terminology

### 8.1 Reserve “observed support” for the observed-only quantity

The term **observed support** should continue to mean

\[
S_{K_E}(\mathbf z^{\mathrm{obs}}),
\]

calculated directly from separately conducted empirical evaluations without a computational layer.

Observed support should always be reported, but when the declared assessment includes a defensible computational procedure it is the empirical gate rather than the final replacement criterion.

### 8.2 Primary and reduced assessments

The manuscript should distinguish decision authority from diagnostic importance:

- **Primary full assessment:** observed-anchored nested support and survival when a scientifically defensible computational procedure has been declared.
- **Necessary empirical gate:** direct observed support and survival, which must pass before the full assessment can pass.
- **Reduced assessment:** the observed-only rule when the computational level is deliberately omitted.

The observed-only quantities remain scientifically prominent because they show what the empirical evaluations themselves establish and identify whether a no-verdict result arises before or after computational challenge. Nevertheless, passing the observed gate is not a supported-replacement verdict under a policy that prespecifies the full nested assessment.

The reduced assessment may be appropriate when no defensible resampling unit exists, dependence cannot be preserved, outcomes are too sparse, the resampling law would generate implausible evaluations, or computational cost is disproportionate to the decision. It should not be selected after seeing that the nested assessment fails.

### 8.3 Name the new quantity explicitly

Recommended descriptions include:

- **observed-anchored nested support**;
- **anchored empirical–computational support**; or
- **observed-anchored computationally challenged support**.

The first is mathematically precise; the second may read more naturally in prose. Whichever term is chosen should be used consistently.

The new quantity should not be called simply “observed support,” because its value depends on a constructed resampling procedure. It should not be called simply “projected support,” because the observed empirical effects remain mandatory anchors.

### 8.4 A concise interpretive statement

A suitable headline interpretation is:

> Anchored empirical–computational support summarizes the weakest paired advantage retained across selected groups of separately conducted empirical evaluations after each selected evaluation is required to retain both its observed effect and selected effects produced under its declared computational resampling procedure.

### 8.5 What the computational layer does and does not add

The manuscript should repeat the following distinction at several strategic points:

- increasing \(M\) adds empirical evidence;
- computational resampling adds no empirical evidence;
- increasing \(J\) improves Monte Carlo approximation;
- increasing \(K_C\) strengthens the declared within-evaluation computational challenge;
- increasing \(K_E\) strengthens the declared across-evaluation challenge.

Even after passing the nested rule, the result remains silent about case types, score values, environments, measurement processes, and failure mechanisms absent from the empirical evaluations.

The manuscript should also distinguish computational feasibility from computational severity. Reducing \(J\) increases Monte Carlo error; it does not redefine the exact computational target. Omitting the layer through \(K_C=0\), or lowering a positive \(K_C\), changes the assessment itself.

### 8.6 The status of the observed anchor

Anchoring deliberately privileges the effect calculated from the full observed evaluation. This is not a claim that the observed realization is a population truth. It is a policy choice about finite evidence: the empirical result actually obtained should remain part of every challenge applied to that evaluation.

The manuscript should acknowledge the tradeoff. An unusually adverse observed evaluation will remain binding even when most resamples are favorable. Under the proposed logic, this is intended rather than a bias to be corrected: the resampling distribution may diagnose fragility, but it does not erase the empirical result.

---

## 9. Recommended restructuring of the main manuscript

### 9.1 Replace the two-path narrative with a hierarchical narrative

Version 16's observed-versus-projected distinction should remain, but it should no longer organize the method as two principally alternative paths. Version 17 should begin with the full three-level hierarchy and make observed-anchored nested support the primary rule whenever its computational procedure is defensible. The observed-only construction should then enter as the necessary empirical gate and, when the computational layer is omitted, as a reduced assessment.

### 9.2 Proposed main-text outline

#### 1. Performance claims and levels of challenge

- Motivate prevalence-sensitive ranking claims.
- Introduce \(\mathcal A=(\mathcal C,\mathcal R,K)\) only after deciding how to revise the assessment object for two orders. A likely update is
  \[
  \mathcal A=(\mathcal C,\mathcal R,\mathcal B,K_E,K_C),
  \]
  where \(\mathcal B\) denotes the declared within-evaluation computational procedure. Another option is to keep \(\mathcal B\) as part of \(\mathcal R\), but separating it may improve clarity.
- Introduce \(N_m,M,J_m,K_E,K_C\) immediately.
- State that the full anchored hierarchy is primary when its computational procedure is defensible.
- Introduce observed-only support as the necessary empirical gate and as a reduced assessment when the computational level is omitted.
- Include the boundary-case table.

#### 2. Constructing a retained paired AP effect

- Prior-standardized AP.
- CNAP and the common opportunity scale.
- Paired differences at each target prevalence.
- Why marginal summaries do not compose.
- Tie-averaged AP and refinement neutrality.
- The observed retained effect \(z_m^{\mathrm{obs}}\).
- The computational retained effect \(z_{mj}^{\mathrm{comp}}\).

This order lets the reader understand what one array entry means before meeting the hierarchical support operator.

#### 3. Support across empirical and computational challenges

- Retain the original \(S_K\) definition.
- Introduce within-evaluation anchored support.
- Introduce \(S^{\mathrm{anc}}_{K_E;K_C}\).
- Explain why the array cannot be flattened.
- Explain why row summaries cannot be composed through an outer \(S_{K_E}\).
- Define anchored literal survival.
- Give the survival-integral representation.
- State reduction, monotonicity, domination, and orientation results.

#### 4. Evidence and computational implementation

- Define admissible empirical evaluations under \(\mathcal R\).
- Define the resampling procedure separately within each evaluation.
- Explain observed-only, anchored nested, and unanchored projected quantities.
- Give the staged stopping rule: do not generate computational replications for the primary verdict when the observed empirical gate already fails.
- Distinguish \(J_m\) from \(K_C\).
- Discuss clustering, matching, nonoverlap, adaptation, and independence across empirical rows.
- Present computation and Monte Carlo diagnostics.

#### 5. Finite-evidence replacement rule

- Present the observed empirical gate.
- Apply the primary two-part rule to the anchored quantities only after that gate passes.
- Define the reduced observed-only verdict when the computational level was not part of the declared assessment.
- Assess the reverse orientation.
- Explain no verdict.
- State the external deployment premise.

This section could alternatively remain attached to the support section. Separating it may improve the transition from mathematical construction to decision policy.

#### 6. Empirical illustrations

- Bank Marketing as the \(M=1\) anchored boundary.
- ProteinGym as an observed-gate failure for which anchored nesting cannot change the verdict.
- If possible, add an \(M>1,K_C>0\) example that passes the empirical gate and therefore proceeds to the computational stage.

#### 7. Interpretation, reporting, and scope

- Explain the three challenge levels.
- Discuss related population-robustness and inferential frameworks.
- Provide revised minimum reporting content.
- State limitations and transport commitments.

#### 8. Conclusion

- Present the hierarchy as the unifying contribution.
- Reiterate that computational challenge deepens but does not broaden empirical evidence.

### 9.3 Material likely to move from Version 16

- Move the \(N/M/J/K\) distinction from later sections to the Introduction, expanding it to \(K_E/K_C\).
- Move the effect construction before the generic support development, so readers encounter \(z_m^{\mathrm{obs}}\) and \(z_{mj}^{\mathrm{comp}}\) concretely.
- Merge the current “Observed and projected effect lists” and “Evidence and computational projection” discussions into the new hierarchical evidence section.
- Retain most tie-averaging material, because it remains necessary at every observed and computational cell.
- Move detailed sorted computations and Monte Carlo formulas to appendices.
- Rewrite the conclusion's displayed formula to include observed anchors and both orders.

---

## 10. Implications for the empirical illustrations

### 10.1 Bank Marketing

The Bank Marketing illustration currently reports an unanchored computational projection from one temporal evaluation. Under Version 17, the primary calculation should include the temporal holdout's observed retained effect as a mandatory anchor:

\[
\Theta^{\mathrm{anc}}_{1;K_C}
=
\binom{J}{K_C}^{-1}
\sum_A
\min\left\{
z_1^{\mathrm{obs}},
\min_{j\in A}z_{1j}^{\mathrm{comp}}
\right\}.
\]

The numerical results must be recomputed. If the observed anchor lies above most computational subset minima, the anchored and unanchored magnitudes may be similar. If it lies below them, it will cap every challenge and can materially change the result.

The literal-survival result also changes. It becomes zero whenever the temporal holdout anchor fails the declared floor, regardless of the computational exceedance count.

The existing unanchored values may be retained as secondary diagnostics labeled “unanchored computational projection,” but they should not be mixed with the primary anchored rule.

The staged presentation should begin with the observed temporal-holdout effect. If that empirical gate fails for an orientation, the anchored rule must return no verdict and no computational calculation is required for the decision. If it passes, the existing computational replications can be used to determine whether the claim survives Stage 2. The positive-control comparison is therefore a natural candidate for demonstrating the complete \(M=1\) sequence.

### 10.2 ProteinGym

The current ProteinGym analysis uses \(M=91\) observed assay effects. Its existing quantities are the observed empirical gate:

\[
S^{\mathrm{anc}}_{K_E;0}
=
S_{K_E}(\mathbf z^{\mathrm{obs}}).
\]

All three forward and reverse comparisons fail that gate under the declared policy. Anchor domination therefore implies that adding within-assay computational challenges cannot produce a supported-replacement verdict under the same thresholds. It could only preserve the no-verdict result or reveal further fragility.

This gives the absence of a nested ProteinGym computation a principled justification: the empirical claim was already defeated before the computational level, so the more expensive nested calculation was unnecessary for the primary verdict. ProteinGym remains a demonstration of the observed component of the hierarchy, but it should be described as a Stage 1 failure rather than simply as an equal-status \(K_C=0\) boundary analysis.

### 10.3 Need for an interior example

The manuscript would still be stronger with at least one \(M>1,K_C>0\) example that passes the observed empirical gate and therefore reaches Stage 2. Such an example would show how a claim that is supported across observed empirical evaluations can either survive or fail the additional within-evaluation computational challenge.

One possibility is to add within-assay computational replication to ProteinGym. Before doing so, the manuscript would need to justify:

- the resampling unit within each assay;
- whether substitutions can reasonably be resampled as independent units;
- whether protein-position or other clustering should be preserved;
- how class-stratified resampling interacts with sparse outcomes in particular assays;
- whether every assay has enough positive and negative units for the declared \(K_C\) challenge; and
- the computational cost of tie-averaged AP across \(91\times J\) replicated profiles.

If those assumptions are not defensible, Version 17 should not add nested ProteinGym resampling merely to display the interior construction. Because ProteinGym already fails the observed gate, that computation is unnecessary for its verdict in any event. A different dataset or comparison that passes Stage 1 would be a more informative interior illustration.

### 10.4 Figures

At least one schematic figure should show the hierarchy:

1. rows as empirical evaluations;
2. observed anchors attached to each row;
3. computational effects within rows;
4. selection of \(K_E\) rows and \(K_C\) cells per row; and
5. the minimum across the complete selected structure.

This figure would likely teach the construction more effectively than beginning with the full combinatorial formula.

---

## 11. Computational consequences

### 11.1 The empirical gate as a computational stopping rule

The observed anchors and their direct order-\(K_E\) support should be calculated before any computational replications are generated for the primary decision. If either observed policy requirement fails, anchor domination proves that the nested policy must fail. The algorithm can stop with no verdict.

This stopping rule can substantially reduce computation when \(M\) is large. It is not an approximation: it follows exactly from the anchored inequalities. Computational replications may still be generated as secondary fragility diagnostics, but they are unnecessary for the replacement verdict.

If the empirical gate passes, the analysis proceeds to the computational stage. Computational expense can then be addressed by efficient evaluation, convergence studies, or—if prespecified—use of the reduced observed-only assessment. The manuscript should not describe a smaller \(J\) as weakening the intended challenge; it increases approximation error for the same computational target.

### 11.2 Direct enumeration is infeasible

The number of complete nested selections is

\[
\binom{M}{K_E}
\binom{J}{K_C}^{K_E}
\]

when every row has \(J\) computational effects. Direct enumeration becomes infeasible quickly.

Version 17 should therefore lead computation through survival functions rather than nested subsets.

### 11.3 Efficient finite-array computation

For each level \(s\), compute

\[
q^{\mathrm{anc}}_{m,K_C,s}
=
\mathbf 1\{z_m^{\mathrm{obs}}>s\}
\frac{\binom{R_{m,s}}{K_C}}
{\binom{J}{K_C}}.
\]

Then compute

\[
\Psi^{\mathrm{anc}}_{K_E;K_C,s}
=
\frac{e_{K_E}(q_1(s),\ldots,q_M(s))}
{\binom{M}{K_E}}.
\]

Because this survival curve changes only when \(s\) crosses an observed or computational effect, the magnitude integral is an exact finite sum across consecutive ordered effect levels. This avoids enumerating empirical subsets or Cartesian products of computational subsets.

The elementary symmetric polynomial can be computed by the recursion

\[
e_0=1,
\qquad
e_k^{(m)}
=
e_k^{(m-1)}+q_m e_{k-1}^{(m-1)},
\]

for \(k=1,\ldots,K_E\). Thus each survival evaluation costs \(O(MK_E)\), with further optimization possible when processing sorted effect thresholds incrementally.

### 11.4 Monte Carlo target versus Monte Carlo approximation

Version 17 should distinguish:

- the exact computational functional under the declared resampling law; and
- its approximation using \(J_m\) generated replications in each row.

For an exact within-row resampling law, if \(p_m(s)\) is the probability that one computational retained effect exceeds \(s\), then the idealized iid order-\(K_C\) row survival is

\[
q^{\mathrm{anc},\infty}_{m,K_C,s}
=
\mathbf 1\{z_m^{\mathrm{obs}}>s\}
p_m(s)^{K_C}.
\]

The finite-\(J_m\) distinct-subset fraction

\[
\frac{\binom{R_{m,s}}{K_C}}
{\binom{J_m}{K_C}}
\]

is a U-statistic approximation to that target. The observed anchor remains fixed.

This formulation clarifies why \(K_C\) is a substantive order while \(J_m\) is an approximation count.

### 11.5 Monte Carlo error

The Version 16 \(K=2\) leave-one-replication standard-error formula does not transfer unchanged to the full anchored hierarchy. Version 17 should either:

- derive an appropriate hierarchical U-statistic or influence-function approximation; or
- use repeated independent seeds and increasing \(J_m\) as the primary numerical stability assessment, with a carefully labeled empirical Monte Carlo standard error.

At minimum, reports should show stability of:

- anchored supported magnitude;
- anchored literal survival at \(d\);
- the identity of observed anchors that bind;
- row-level computational survival fractions near the policy threshold; and
- the final verdict.

### 11.6 Independence and coordinated resampling

The product form for nested literal survival assumes that computational subsets are selected independently across empirical evaluations. This is natural when the empirical evaluations contain nonoverlapping units and are resampled separately.

If evaluations share units or if a scientifically meaningful perturbation must be coordinated across contexts, the computational selection law must preserve that dependence. In such cases, the product representation may no longer apply, although the direct challenge-level definition remains valid.

Version 17 should state this boundary rather than implying that independent rowwise resampling is universally appropriate.

### 11.7 Numerical prevalence search

Every observed anchor and computational effect requires a prevalence infimum. The existing warnings about finite grids, interior minima, nonsmooth changes in minimizer location, and optimizer tolerance become more important because numerical error can affect many cells and can alter exceedance indicators near \(d\).

Reports should distinguish:

- accuracy of each retained-effect value;
- stability of its minimizing prevalence;
- Monte Carlo stability across computational draws; and
- policy sensitivity when anchors or computational effects lie near \(d\).

---

## 12. Reporting implications

### 12.1 Minimum headline quantities

A Version 17 analysis should report the Stage 1 observed quantities first:

\[
S_{K_E}(\mathbf z^{\mathrm{obs}}),
\qquad
\psi_{K_E,d}(\mathbf z^{\mathrm{obs}}).
\]

If that empirical gate passes and a computational procedure belongs to the declared assessment, the report should then give the primary Stage 2 quantities:

\[
\Theta^{\mathrm{anc}}_{K_E,K_C},
\qquad
\Psi^{\mathrm{anc}}_{K_E,K_C,d}.
\]

If the empirical gate fails, the report should state that anchor domination makes the primary nested rule incapable of passing and that computational replication was therefore unnecessary for the verdict. If the computational layer was omitted altogether, the report should label any verdict as arising from the reduced observed-only assessment.

The direct observed values show what the empirical evaluations themselves support. The nested values, when required, show what remains after the declared computational challenge is added.

### 12.2 Required design declarations

Reports should identify:

- candidate and incumbent orientation;
- condition set \(\mathcal C\), with \(\Pi_\star\) for AP;
- evaluation regime \(\mathcal R\);
- independent empirical evaluation unit;
- \(M\) and the evaluation-specific \(N_m\);
- \(K_E\);
- the computational resampling procedure;
- \(J_m\) for every evaluation;
- \(K_C\);
- \(\delta,d,\gamma\); and
- whether the computational selections are independent across evaluations.

The report should also state whether the full nested assessment or the reduced observed-only assessment was prespecified, and why any computational layer was omitted.

### 12.3 Anchor-specific diagnostics

Reports should additionally provide:

- the number and fraction of observed anchors clearing \(d\);
- the row-level anchored survival values
  \(q^{\mathrm{anc}}_{m,K_C,d}\);
- which observed anchors bind the supported-magnitude calculation over relevant effect ranges;
- how often computational effects fall below their corresponding anchors; and
- whether the nested verdict differs from the direct observed verdict.

### 12.4 Suggested headline language

An example is:

> Across \(M\) separately conducted empirical evaluations satisfying the declared regime, direct order-\(K_E\) observed support was [value] and observed literal survival above \(d=[value]\) was [value], so the comparison [passed/failed] the empirical gate. [If passed:] Each evaluation was then additionally challenged using \(J_m\) paired computational replications under the declared resampling procedure. With computational order \(K_C\), observed-anchored nested support was [value] and nested literal survival was [value]. The full assessment therefore returned [supported replacement/no verdict] under thresholds \(\delta=[value]\) and \(\gamma=[value]\). [If failed:] Because anchored nested support and survival cannot exceed their observed counterparts, the full assessment necessarily returned no verdict and computational replication was not required. The computational layer adds a challenge conditional on represented units; it does not add empirical evaluations or support claims about unrepresented cases or contexts.

For \(M=1\), the opening should make clear that only one empirical evaluation was observed and that \(K_E=1\) is forced.

For a reduced analysis, a suitable final sentence is:

> No within-evaluation computational challenge was included; the verdict therefore applies to the reduced observed-only assessment rather than the full anchored assessment.

---

## 13. Generalization beyond AP

The hierarchical operator is metric-agnostic. It accepts any observed and computational retained effects of the form

\[
z_m^{\mathrm{obs}}
=
\inf_{c\in\mathcal C}\Delta_c(D_m),
\qquad
z_{mj}^{\mathrm{comp}}
=
\inf_{c\in\mathcal C}\Delta_c(D_m^{(j)}).
\]

Therefore, the conceptual addition generalizes beyond target prevalence and AP.

For the AUROC extension, the within-evaluation condition may be a case-mix concentration allowance or a pair of admissible within-class weights. The observed AUROC retained effect would remain an anchor, and computational resampling within each empirical evaluation could add another challenge before the across-evaluation order is imposed.

Care is needed because the AUROC appendix already contains two forms of computational construction:

- optimization over empirical case-mix weights; and
- possible empirical resampling when only one evaluation is observed.

Version 17 should distinguish the condition-level case-mix optimization from the computational-replication level. A possible hierarchy is:

1. optimize over the declared AUROC case-mix condition within each observed or resampled evaluation;
2. anchor and challenge across computational replications within each empirical evaluation;
3. challenge across empirical evaluations.

This demonstrates the general value of separating condition, computational, and empirical indices.

---

## 14. Decisions that should be settled before drafting Version 17

The following choices should be resolved explicitly before the full rewrite.

### 14.1 Name of the primary quantity

Recommended: **observed-anchored nested support**, with **anchored empirical–computational support** as a prose synonym.

### 14.2 Notation

Recommended:

\[
S^{\mathrm{anc}}_{K_E;K_C}
\]

for the generic operator, and

\[
\Theta^{\mathrm{anc}}_{A:B;K_E,K_C}
\]

for the paired replacement quantity. The semicolon should distinguish the two levels in the operator notation.

### 14.3 Treatment of \(K_C=0\)

Recommended: allow \(K_C=0\) as an explicit mathematical convention so the observed-only operator is an exact boundary case, while using “no computational challenge” in prose. Treat this quantity as the necessary empirical gate and, when the computational layer is omitted, as a reduced assessment. Do not use \(J=0\) as the main notation for this choice, because \(J\) concerns Monte Carlo approximation rather than challenge severity.

### 14.4 Status of the Version 16 unanchored projection

Recommended: retain it as a clearly labeled secondary computational projection and as historical motivation, but make the observed-anchored quantity primary in Version 17.

### 14.5 Assessment tuple

The current

\[
\mathcal A=(\mathcal C,\mathcal R,K)
\]

must be reconsidered. One option is

\[
\mathcal A^{\mathrm{anc}}
=
(\mathcal C,\mathcal R,\mathcal B,K_E,K_C),
\]

where \(\mathcal B\) is the declared computational procedure. This makes the computational challenge an explicit component of the claim. Another option is to define \(\mathcal B\) separately because it characterizes computation rather than empirical admissibility. The first option is narratively cleaner, but the manuscript should avoid suggesting that \(\mathcal B\) is an empirical data-generating law.

### 14.6 Interior empirical illustration

Recommended: investigate whether a scientifically defensible \(M>1,K_C>0\) analysis that passes the observed empirical gate can be added. Do not force nested resampling in ProteinGym, which already fails the gate, or in any setting where the within-evaluation resampling assumptions are weak.

### 14.7 Monte Carlo inference

Recommended: decide whether Version 17 will derive a hierarchical Monte Carlo standard-error estimator or rely on replicated seeds and convergence diagnostics. The manuscript should not retain the Version 16 \(K=2\) formula without proving its applicability to the new hierarchy.

---

## 15. Recommended propositions and appendices

### Main-text propositions

1. **Boundary reduction:** observed-only and one-evaluation constructions arise as special cases.
2. **Anchored challenge domination:** nested anchored support and survival do not exceed direct observed support and survival.
3. **Empirical-gate implication:** failure of either observed policy requirement implies failure of the corresponding full anchored rule.
4. **Hierarchical challenge monotonicity:** support is nonincreasing as \(\mathcal C\), \(K_C\), or \(K_E\) is strengthened.
5. **Magnitude–survival identity:** anchored support integrates anchored nested survival.
6. **Orientation bound:** positive supported advantages cannot occur in both directions.

### Appendix material

1. Proofs of reduction, domination, monotonicity, and orientation.
2. Elementary-symmetric and survival representations.
3. Exact finite-array computation.
4. Monte Carlo approximation under row-specific resampling laws.
5. Extension of exchangeable-label reference behavior.
6. Counterexample showing why row-summary composition fails.
7. Counterexample showing why flattening \(MJ\) cells changes the empirical meaning and can make results depend improperly on \(J_m\).
8. Adaptation of the AUROC case-mix breakdown construction.

---

## 16. Recommended narrative emphasis

The strongest Version 17 narrative is not “bootstrap uncertainty” and not “more replications.” It is **preservation of scientific identity under layered challenge**.

Each operation answers a different question:

1. **Condition infimum:** What advantage survives every declared target condition within this complete evaluation?
2. **Observed anchor:** What effect was actually retained in this empirical test?
3. **Empirical gate:** Does the claim clear the replacement thresholds across the observed empirical tests themselves?
4. **Computational order:** If so, does that empirically supported claim remain supported when the represented units are computationally recombined?
5. **Full nested rule:** Does the claim clear the replacement thresholds after both empirical and computational challenges are imposed?

The order of operations should be stated repeatedly:

\[
\text{pair models}
\rightarrow
\text{challenge conditions}
\rightarrow
\text{retain observed anchors}
\rightarrow
\text{test the observed empirical gate}
\rightarrow
\text{add computational challenges within rows}
\rightarrow
\text{apply the full anchored rule}.
\]

This is the conceptual core of Version 17.

---

## 17. Summary recommendation

Version 17 should adopt observed-anchored nested support as the primary general construction whenever a scientifically defensible within-evaluation computational procedure is available. It should preserve the original \(S_K\) operator and define

\[
S^{\mathrm{anc}}_{K_E;K_C}
\left(
\mathbf z^{\mathrm{obs}};
\mathbf Z^{\mathrm{comp}}
\right)
\]

as its hierarchical extension. The original multiple-evaluation observed analysis should be recovered by the formal boundary \(K_C=0\), used as the necessary empirical gate, and retained as a reduced assessment when no computational challenge is declared. The Version 16 one-evaluation projected analysis should remain identifiable as the unanchored \(M=1\) special case, while the new primary one-evaluation rule retains the observed empirical effect as a mandatory anchor.

The replacement procedure should be staged. First, direct observed support and survival must clear their thresholds. Failure at that empirical gate proves that the full anchored rule cannot pass, so the analysis returns no verdict without requiring computational replication. Only a comparison that passes the observed gate proceeds to the within-evaluation computational challenge. Passing both stages supports replacement under the primary full assessment.

The central interpretive claim should be:

> Computational replication can deepen the declared challenge applied to each empirical evaluation, but it cannot increase the number of empirical evaluations or erase the effect actually observed in any of them.

When the computational level is scientifically unjustified or computationally infeasible, an observed-only rule may be prespecified. It is a legitimate but less severe assessment and should be reported as such. The distinction should be represented by \(K_C=0\), not by treating a smaller \(J\) as a weaker challenge: \(J\) controls approximation accuracy, whereas \(K_C\) controls computational severity.

The central mathematical relationship remains:

\[
S^{\mathrm{anc}}_{K_E;K_C}
=
a+
\int_a^b
\Psi^{\mathrm{anc}}_{K_E;K_C,s}\,ds,
\]

so supported magnitude and literal survival continue to summarize the same challenged effects.

This revision strengthens the manuscript in four ways. It makes the Popperian motivation operational at every declared level, prevents computational projection from displacing finite empirical evidence, makes the direct observed analysis an exact and computationally useful gate, and unifies the existing observed and projected analyses within a single scientifically interpretable hierarchy.
