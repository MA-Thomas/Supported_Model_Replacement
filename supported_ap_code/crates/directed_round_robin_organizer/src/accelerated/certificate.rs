use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::plan::{
    AcceleratedPlan, ContextExecution, OutputScope, read_json, validate_accelerated_plan,
};
use crate::artifact::{
    MatchArtifact, read_artifact_with_digest, result_path, validate_bound_artifact,
};
use crate::graph::{DirectedEdge, GraphSummary, analyze_graph};
use crate::input::LoadedBundle;
use crate::judge::JudgeReport;
use crate::plan::match_for_pair;
use crate::{DirectedVerdict, Error, Result};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MatchReference {
    pub evaluation_id: String,
    pub system_low: String,
    pub system_high: String,
    pub match_id: String,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ExclusionWitness {
    SupportedIncoming {
        evaluation_id: String,
        challenger: String,
        match_id: String,
    },
    /// A complete context graph is required to check this SCC witness.
    NonSourceComponent {
        evaluation_id: String,
        component: Vec<String>,
        incoming: DirectedEdge,
    },
}

/// Untrusted wire representation. Only `audit_selection` produces a selection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SelectionCertificate {
    pub schema_version: u32,
    pub plan_id: String,
    pub output_scope: OutputScope,
    pub matches: Vec<MatchReference>,
    pub exclusions: BTreeMap<String, ExclusionWitness>,
    pub survivors: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ComparisonEvidence {
    Assessed {
        match_id: String,
        verdict: DirectedVerdict,
    },
    ObservedGateRejected,
    NotComputed {
        exclusion: Option<ExclusionWitness>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SurvivorCertificate {
    pub system_id: String,
    pub assessed_incoming: usize,
    pub gate_rejected_incoming: usize,
    pub exhaustive_source_contexts: Vec<String>,
    pub unresolved_incoming: usize,
}

/// No Deserialize and no public constructor: the certificate has been checked
/// against the bundle, every referenced report, and all incoming obligations.
/// Graph maxima use the same asserted-supported-edge convention as Reduction;
/// scientific Unresolved directions remain explicitly qualified.
#[derive(Debug, Serialize)]
pub struct AuditedSelection {
    plan_id: String,
    tournament_id: String,
    output_scope: OutputScope,
    strategy: crate::SelectionStrategy,
    survivors: Vec<String>,
    survivor_certificates: Vec<SurvivorCertificate>,
    exclusions: BTreeMap<String, ExclusionWitness>,
    assessed_matches: usize,
    possible_matches: usize,
    complete_context_graphs: BTreeMap<String, GraphSummary>,
    assessed_unresolved_directions: Vec<(String, DirectedEdge)>,
    #[serde(skip)]
    plan: AcceleratedPlan,
    #[serde(skip)]
    delta: f64,
    #[serde(skip)]
    evidence: EvidenceIndex,
}

impl AuditedSelection {
    pub fn survivors(&self) -> &[String] {
        &self.survivors
    }
    pub fn plan_id(&self) -> &str {
        &self.plan_id
    }
    pub fn tournament_id(&self) -> &str {
        &self.tournament_id
    }
    pub fn output_scope(&self) -> OutputScope {
        self.output_scope
    }
    pub fn survivor_certificates(&self) -> &[SurvivorCertificate] {
        &self.survivor_certificates
    }
    pub fn exclusions(&self) -> &BTreeMap<String, ExclusionWitness> {
        &self.exclusions
    }
    pub fn complete_context_graphs(&self) -> &BTreeMap<String, GraphSummary> {
        &self.complete_context_graphs
    }
    pub fn assessed_unresolved_directions(&self) -> &[(String, DirectedEdge)] {
        &self.assessed_unresolved_directions
    }
    pub fn assessed_matches(&self) -> usize {
        self.assessed_matches
    }
    pub fn possible_matches(&self) -> usize {
        self.possible_matches
    }
    pub fn match_references(&self) -> impl Iterator<Item = &MatchReference> {
        self.evidence.values().map(|e| &e.reference)
    }
    /// Returns complete S0 pair reports only after the operational scope audit.
    pub fn operational_match_references(&self) -> Result<Vec<&MatchReference>> {
        if self.output_scope != OutputScope::SurvivorSetAndOperationalInputs {
            return Err(Error::Incomplete(
                "operational inputs were not requested".into(),
            ));
        }
        Ok(self
            .match_references()
            .filter(|r| {
                self.survivors.binary_search(&r.system_low).is_ok()
                    && self.survivors.binary_search(&r.system_high).is_ok()
            })
            .collect())
    }
    /// Rechecks the bytes and original run binding when a consumer reads a
    /// numerical report. Auditing never authorizes subsequently changed files.
    pub fn read_match(
        &self,
        bundle: &LoadedBundle,
        root: &Path,
        reference: &MatchReference,
    ) -> Result<MatchArtifact> {
        let e = lookup(
            &self.evidence,
            &reference.evaluation_id,
            &reference.system_low,
            &reference.system_high,
        )
        .ok_or_else(|| invalid("match is outside the audited evidence"))?;
        if e.reference != *reference
            || bundle.manifest.bundle_content_hash != self.plan.bundle_content_hash
        {
            return Err(invalid(
                "report reference or bundle differs from audited selection",
            ));
        }
        let (artifact, digest) =
            read_artifact_with_digest(&result_path(root, &reference.match_id))?;
        let item = match_for_pair(
            bundle,
            &reference.evaluation_id,
            &reference.system_low,
            &reference.system_high,
        )?;
        validate_bound_artifact(&artifact, bundle, &self.plan.binding(), &item)?;
        if digest != reference.sha256 {
            return Err(invalid("match changed after selection audit"));
        }
        Ok(artifact)
    }
    pub fn comparison(
        &self,
        evaluation: &str,
        winner: &str,
        loser: &str,
    ) -> Result<ComparisonEvidence> {
        let context = self
            .plan
            .contexts
            .get(evaluation)
            .ok_or_else(|| invalid("unknown context"))?;
        if winner == loser
            || [winner, loser]
                .iter()
                .any(|s| self.plan.systems.binary_search(&(*s).to_owned()).is_err())
        {
            return Err(invalid("unknown or identical comparison systems"));
        }
        if let Some(e) = lookup(&self.evidence, evaluation, winner, loser) {
            return Ok(ComparisonEvidence::Assessed {
                match_id: e.reference.match_id.clone(),
                verdict: e.verdict(winner),
            });
        }
        if context.rejects(winner, loser, self.delta) {
            return Ok(ComparisonEvidence::ObservedGateRejected);
        }
        Ok(ComparisonEvidence::NotComputed {
            exclusion: self.exclusions.get(loser).cloned(),
        })
    }
}

#[derive(Debug, Clone)]
pub(crate) struct MatchEvidence {
    pub reference: MatchReference,
    pub low_over_high: DirectedVerdict,
    pub high_over_low: DirectedVerdict,
}

impl MatchEvidence {
    pub(crate) fn verdict(&self, winner: &str) -> DirectedVerdict {
        if winner == self.reference.system_low {
            self.low_over_high
        } else {
            self.high_over_low
        }
    }
}

pub(crate) type EvidenceIndex = BTreeMap<(String, String, String), MatchEvidence>;
pub(crate) fn key(e: &str, a: &str, b: &str) -> (String, String, String) {
    let (low, high) = if a < b { (a, b) } else { (b, a) };
    (e.into(), low.into(), high.into())
}
pub(crate) fn lookup<'a>(
    index: &'a EvidenceIndex,
    e: &str,
    a: &str,
    b: &str,
) -> Option<&'a MatchEvidence> {
    index.get(&key(e, a, b))
}
fn invalid(message: impl Into<String>) -> Error {
    Error::InvalidReduction(message.into())
}

/// Validates report orientation, normalized verdicts, and the numerical
/// necessary condition for BOTH directions. A contradiction invalidates this
/// run; it cannot silently survive as a pruning decision. Replan the named
/// context with exhaustive execution to investigate it.
pub(crate) fn check_order(
    plan: &AcceleratedPlan,
    bundle: &LoadedBundle,
    artifact: &MatchArtifact,
) -> Result<()> {
    let judged = &artifact.judged;
    let (low_verdict, high_verdict, low_observed, high_observed) = match &judged.report {
        JudgeReport::PrCnap { result, .. } if plan.metric == crate::MetricKind::PrCnap => (
            crate::judge::normalize_pr(&result.forward),
            crate::judge::normalize_pr(&result.reverse),
            result.observed_evaluation.forward.value,
            result.observed_evaluation.reverse.value,
        ),
        JudgeReport::Auroc {
            low_over_high_result: low,
            high_over_low_result: high,
            ..
        } if plan.metric == crate::MetricKind::Auroc => (
            crate::judge::normalize_auroc(low.baseline.verdict),
            crate::judge::normalize_auroc(high.baseline.verdict),
            judged.descriptive.observed_difference_low_over_high,
            judged.descriptive.observed_difference_high_over_low,
        ),
        _ => return Err(invalid("match report uses the wrong judge")),
    };
    if low_verdict != judged.low_over_high.verdict
        || high_verdict != judged.high_over_low.verdict
        || !low_observed.is_finite()
        || !high_observed.is_finite()
    {
        return Err(invalid("inconsistent or nonfinite match report"));
    }
    let context = &plan.contexts[&artifact.evaluation_id];
    let a = &artifact.system_low;
    let b = &artifact.system_high;
    let consistent = match context {
        ContextExecution::OrderedCnap { values, .. } => {
            low_observed <= values[a] - values[b] && high_observed <= values[b] - values[a]
        }
        ContextExecution::OrderedAuroc { doubled_credits } => {
            let e = &bundle.evaluations[&artifact.evaluation_id];
            let difference = ((doubled_credits[a] as i64 - doubled_credits[b] as i64) as f64 / 2.0)
                / (e.positive_count as f64 * e.negative_count as f64);
            low_observed == difference && high_observed == -difference
        }
        ContextExecution::Exhaustive { .. } => true,
    };
    let delta = bundle.spec.evidence_policy.magnitude_threshold;
    if !consistent
        || [(a, b, low_verdict), (b, a, high_verdict)]
            .iter()
            .any(|(w, l, verdict)| {
                *verdict == DirectedVerdict::Supported && context.rejects(w, l, delta)
            })
    {
        return Err(invalid(format!(
            "numerical ordering contradiction in context {} for {a}, {b}: observed ({low_observed:?}, {high_observed:?}); no accelerated certificate is valid; replan this context as exhaustive",
            artifact.evaluation_id
        )));
    }
    Ok(())
}

pub(crate) fn load_evidence(
    bundle: &LoadedBundle,
    plan: &AcceleratedPlan,
    root: &Path,
    evaluation: &str,
    a: &str,
    b: &str,
) -> Result<MatchEvidence> {
    let item = match_for_pair(bundle, evaluation, a, b)?;
    let path = result_path(root, &item.match_id);
    let (artifact, digest) = read_artifact_with_digest(&path)?;
    validate_bound_artifact(&artifact, bundle, &plan.binding(), &item)?;
    check_order(plan, bundle, &artifact)?;
    Ok(MatchEvidence {
        reference: MatchReference {
            evaluation_id: item.evaluation_id,
            system_low: item.system_low,
            system_high: item.system_high,
            match_id: item.match_id,
            sha256: digest,
        },
        low_over_high: artifact.judged.low_over_high.verdict,
        high_over_low: artifact.judged.high_over_low.verdict,
    })
}

pub(crate) fn complete_graph(
    systems: &[String],
    evaluation: &str,
    index: &EvidenceIndex,
) -> Result<GraphSummary> {
    let mut supported = Vec::new();
    let mut unresolved = Vec::new();
    for (i, a) in systems.iter().enumerate() {
        for b in &systems[i + 1..] {
            let e = lookup(index, evaluation, a, b).ok_or_else(|| {
                invalid(format!("missing exhaustive pair {evaluation}: {a}, {b}"))
            })?;
            for (winner, loser) in [(a, b), (b, a)] {
                let edge = DirectedEdge {
                    from: winner.clone(),
                    to: loser.clone(),
                };
                match e.verdict(winner) {
                    DirectedVerdict::Supported => supported.push(edge),
                    DirectedVerdict::Unresolved => unresolved.push(edge),
                    DirectedVerdict::NotSupported => (),
                }
            }
        }
    }
    analyze_graph(systems.iter().cloned(), supported, unresolved)
}

pub(crate) fn component_witness(
    evaluation: &str,
    graph: &GraphSummary,
    target: &str,
) -> Option<ExclusionWitness> {
    let component = graph
        .strongly_connected_components
        .iter()
        .find(|c| c.iter().any(|s| s == target))?;
    let incoming = graph
        .edges
        .iter()
        .find(|edge| component.contains(&edge.to) && !component.contains(&edge.from))?;
    Some(ExclusionWitness::NonSourceComponent {
        evaluation_id: evaluation.into(),
        component: component.clone(),
        incoming: incoming.clone(),
    })
}

pub fn read_certificate(directory: &Path) -> Result<SelectionCertificate> {
    read_json(&directory.join("selection_certificate.json"))
}

pub fn audit_selection(
    bundle: &LoadedBundle,
    plan: &AcceleratedPlan,
    results_root: &Path,
    certificate: &SelectionCertificate,
) -> Result<AuditedSelection> {
    audit_scope(
        bundle,
        plan,
        results_root,
        certificate,
        plan.options.output_scope,
    )
}

/// Internal intermediate proof: retains the original plan binding, but cannot
/// provide operational references until the separate completion stage passes.
pub(super) fn audit_survivors(
    bundle: &LoadedBundle,
    plan: &AcceleratedPlan,
    results_root: &Path,
    certificate: &SelectionCertificate,
) -> Result<AuditedSelection> {
    audit_scope(
        bundle,
        plan,
        results_root,
        certificate,
        OutputScope::SurvivorSet,
    )
}

fn audit_scope(
    bundle: &LoadedBundle,
    plan: &AcceleratedPlan,
    results_root: &Path,
    certificate: &SelectionCertificate,
    required_scope: OutputScope,
) -> Result<AuditedSelection> {
    validate_accelerated_plan(plan, bundle)?;
    if certificate.schema_version != 1
        || certificate.plan_id != plan.plan_id
        || certificate.output_scope != required_scope
    {
        return Err(invalid("selection certificate identity or scope mismatch"));
    }
    let mut index = EvidenceIndex::new();
    for r in &certificate.matches {
        let e = load_evidence(
            bundle,
            plan,
            results_root,
            &r.evaluation_id,
            &r.system_low,
            &r.system_high,
        )?;
        if e.reference != *r
            || index
                .insert(key(&r.evaluation_id, &r.system_low, &r.system_high), e)
                .is_some()
        {
            return Err(invalid("duplicated or mismatched match reference"));
        }
    }
    let mut graphs = BTreeMap::new();
    for (id, context) in &plan.contexts {
        if matches!(context, ContextExecution::Exhaustive { .. }) {
            graphs.insert(id.clone(), complete_graph(&plan.systems, id, &index)?);
        }
    }
    let survivors: BTreeSet<_> = certificate.survivors.iter().cloned().collect();
    let classified: BTreeSet<_> = survivors
        .iter()
        .chain(certificate.exclusions.keys())
        .cloned()
        .collect();
    if certificate.survivors != survivors.iter().cloned().collect::<Vec<_>>()
        || survivors
            .iter()
            .any(|s| certificate.exclusions.contains_key(s))
        || classified != plan.systems.iter().cloned().collect()
    {
        return Err(invalid(
            "certificate must partition the complete roster exactly once",
        ));
    }
    for (target, witness) in &certificate.exclusions {
        match witness {
            ExclusionWitness::SupportedIncoming {
                evaluation_id,
                challenger,
                match_id,
            } => {
                let context = plan
                    .contexts
                    .get(evaluation_id)
                    .ok_or_else(|| invalid("unknown witness context"))?;
                if matches!(context, ContextExecution::Exhaustive { .. }) {
                    return Err(invalid("a single edge is not an SCC exclusion certificate"));
                }
                let e = lookup(&index, evaluation_id, challenger, target)
                    .ok_or_else(|| invalid("missing exclusion witness report"))?;
                if &e.reference.match_id != match_id
                    || e.verdict(challenger) != DirectedVerdict::Supported
                {
                    return Err(invalid(
                        "exclusion witness is not a supported incoming direction",
                    ));
                }
            }
            ExclusionWitness::NonSourceComponent { evaluation_id, .. } => {
                let graph = graphs
                    .get(evaluation_id)
                    .ok_or_else(|| invalid("missing complete SCC context"))?;
                if component_witness(evaluation_id, graph, target).as_ref() != Some(witness) {
                    return Err(invalid("invalid non-source SCC witness"));
                }
            }
        }
    }
    let delta = bundle.spec.evidence_policy.magnitude_threshold;
    let mut survivor_certificates = Vec::new();
    for target in &certificate.survivors {
        let mut proof = SurvivorCertificate {
            system_id: target.clone(),
            assessed_incoming: 0,
            gate_rejected_incoming: 0,
            exhaustive_source_contexts: Vec::new(),
            unresolved_incoming: 0,
        };
        for (evaluation, context) in &plan.contexts {
            if let Some(graph) = graphs.get(evaluation) {
                if !graph.maximal_vertices.contains(target) {
                    return Err(invalid("survivor is outside a source SCC"));
                }
                proof.exhaustive_source_contexts.push(evaluation.clone());
            }
            for challenger in &plan.systems {
                if challenger == target {
                    continue;
                }
                if let Some(e) = lookup(&index, evaluation, challenger, target) {
                    proof.assessed_incoming += 1;
                    match e.verdict(challenger) {
                        DirectedVerdict::Supported if !graphs.contains_key(evaluation) => {
                            return Err(invalid(
                                "survivor has a supported incoming edge in an ordered context",
                            ));
                        }
                        DirectedVerdict::Unresolved => proof.unresolved_incoming += 1,
                        _ => (),
                    }
                } else if context.rejects(challenger, target, delta) {
                    proof.gate_rejected_incoming += 1;
                } else {
                    return Err(invalid(format!(
                        "uncomputed survivor obligation: {evaluation}, {challenger} -> {target}"
                    )));
                }
            }
        }
        survivor_certificates.push(proof);
    }
    if required_scope == OutputScope::SurvivorSetAndOperationalInputs {
        for evaluation in plan.contexts.keys() {
            for (i, a) in certificate.survivors.iter().enumerate() {
                for b in &certificate.survivors[i + 1..] {
                    if lookup(&index, evaluation, a, b).is_none() {
                        return Err(invalid("missing S0 operational match report"));
                    }
                }
            }
        }
    }
    let mut unresolved = Vec::new();
    for e in index.values() {
        let r = &e.reference;
        for (a, b) in [
            (&r.system_low, &r.system_high),
            (&r.system_high, &r.system_low),
        ] {
            if e.verdict(a) == DirectedVerdict::Unresolved {
                unresolved.push((
                    r.evaluation_id.clone(),
                    DirectedEdge {
                        from: a.clone(),
                        to: b.clone(),
                    },
                ));
            }
        }
    }
    Ok(AuditedSelection {
        plan_id: plan.plan_id.clone(),
        tournament_id: plan.tournament_id.clone(),
        output_scope: certificate.output_scope,
        strategy: crate::SelectionStrategy::CandidateConservative,
        survivors: certificate.survivors.clone(),
        survivor_certificates,
        exclusions: certificate.exclusions.clone(),
        assessed_matches: index.len(),
        possible_matches: plan.systems.len() * (plan.systems.len() - 1) / 2 * plan.contexts.len(),
        complete_context_graphs: graphs,
        assessed_unresolved_directions: unresolved,
        plan: plan.clone(),
        delta,
        evidence: index,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exhaustive_context_certifies_whole_cycles_and_keeps_unresolved_separate() {
        let systems: Vec<_> = ["a", "b", "c", "d"]
            .into_iter()
            .map(str::to_owned)
            .collect();
        let mut index = EvidenceIndex::new();
        for (i, a) in systems.iter().enumerate() {
            for b in &systems[i + 1..] {
                index.insert(
                    key("context", a, b),
                    MatchEvidence {
                        reference: MatchReference {
                            evaluation_id: "context".into(),
                            system_low: a.clone(),
                            system_high: b.clone(),
                            match_id: format!("{a}-{b}"),
                            sha256: "test".into(),
                        },
                        low_over_high: DirectedVerdict::NotSupported,
                        high_over_low: DirectedVerdict::NotSupported,
                    },
                );
            }
        }
        for (a, b) in [("b", "c"), ("c", "d"), ("d", "b")] {
            let e = index.get_mut(&key("context", a, b)).unwrap();
            if a < b {
                e.low_over_high = DirectedVerdict::Supported;
            } else {
                e.high_over_low = DirectedVerdict::Supported;
            }
        }
        let graph = complete_graph(&systems, "context", &index).unwrap();
        assert_eq!(graph.maximal_vertices, systems);
        assert!(
            component_witness("context", &graph, "c").is_none(),
            "an internal loss cannot exclude a source SCC member"
        );
        index
            .get_mut(&key("context", "a", "b"))
            .unwrap()
            .low_over_high = DirectedVerdict::Supported;
        index
            .get_mut(&key("context", "a", "d"))
            .unwrap()
            .high_over_low = DirectedVerdict::Unresolved;
        let graph = complete_graph(&systems, "context", &index).unwrap();
        assert_eq!(graph.maximal_vertices, ["a"]);
        for target in ["b", "c", "d"] {
            assert_eq!(
                component_witness("context", &graph, target),
                Some(ExclusionWitness::NonSourceComponent {
                    evaluation_id: "context".into(),
                    component: vec!["b".into(), "c".into(), "d".into()],
                    incoming: DirectedEdge {
                        from: "a".into(),
                        to: "b".into()
                    },
                })
            );
        }
        assert_eq!(
            graph.unresolved_directions,
            [DirectedEdge {
                from: "d".into(),
                to: "a".into()
            }]
        );
        index.remove(&key("context", "a", "c"));
        assert!(
            complete_graph(&systems, "context", &index).is_err(),
            "a missing pair is not NotSupported"
        );
    }
}
