import copy
import json
import tempfile
import unittest
from pathlib import Path

from domain_study.search_evidence import effect_bounds, evidence_columns, validate_summary
from domain_study.proteingym_contexts import context_story as story


class SearchEvidenceTests(unittest.TestCase):
    def test_budget_exhaustion_is_pending_not_failure(self):
        summary = dict(magnitude=-0.01, survival=0.0, supported=False, search=dict(
            supported_magnitude_lower=-0.01, supported_magnitude_upper=0.01,
            literal_survival_lower=0.0, literal_survival_upper=1.0, verdict='unresolved'))
        validate_summary(summary, (-0.01, 0.0), (0.01, 1.0), dict(delta=0.0, gamma=0.81))
        self.assertEqual(evidence_columns(summary)['numerical_status'], 'unresolved')
        corrupted = copy.deepcopy(summary)
        corrupted['search']['verdict'] = 'verified_failure'
        with self.assertRaises(ValueError):
            validate_summary(corrupted, (-0.01, 0.0), (0.01, 1.0), dict(delta=0.0, gamma=0.81))

    def test_effect_certificate_cannot_hide_invalid_endpoints(self):
        effect = dict(value=0.1, search=dict(lower_bound=0.1, sampled_value=0.2, upper_bound=0.15,
                                            stop_reason='budget_exhausted'))
        with self.assertRaises(ValueError):
            effect_bounds(effect)

    def test_possible_cycles_do_not_turn_into_verified_edges(self):
        p = dict(replications=4, computational_order=2, delta=0.0, floor=0.0, gamma=0.81)
        def direction(a, b):
            return dict(candidate=a, incumbent=b, observed=dict(value=-0.01, limiting_prevalence=0.1,
                search=dict(lower_bound=-0.01, upper_bound=0.01, sampled_value=0.0,
                            evaluations=3, stop_reason='budget_exhausted')),
                observed_gate=dict(magnitude=-0.01, survival=0.0, supported=False, search=dict(
                    supported_magnitude_lower=-0.01, supported_magnitude_upper=0.01,
                    literal_survival_lower=0.0, literal_survival_upper=1.0, verdict='unresolved')),
                full=None, prefix=None, computational=[], final_supported=False)
        report = dict(context_id='X', forward=direction(story.MODELS[0], story.MODELS[1]),
                      reverse=direction(story.MODELS[1], story.MODELS[0]), replications=0,
                      maximum_refinement_difference=1.0, refinement_gate_disagreements=1,
                      refinement_floor_disagreements=1, refined_computational_draws=0,
                      rejected_missing_class_draws=0)
        plan = dict(schema_version=2, run_id='test', parameters=p, context_ids=['X'], interpretation='test')
        audit = [dict(assay_id='X', selection_type='synthetic', single_variant_count=4)]
        with tempfile.TemporaryDirectory() as directory:
            result, *_ = story.summarize(Path(directory), plan, audit, [report], 0.0)
        self.assertFalse(result['graph_result_is_resolved'])
        self.assertEqual(result['unresolved_decisions']['full'], 2)
        self.assertEqual(result['reductions']['full']['supported_edges'], 0)
        self.assertEqual(result['definite_reductions']['full']['candidate_conservative'], [story.MODELS[2]])

    def test_adaptive_replay_passes_regular_validator(self):
        output = story.ROOT/'domain_study/proteingym_adaptive_20260918/cnap'
        if not (output/'plan.json').exists():
            self.skipTest('saved adaptive regression run is not installed')
        plan = json.loads((output/'plan.json').read_text())
        plan.update(schema_version=2, profile='publication')
        reports, _ = story.load_and_validate(output, plan,
            story.read_csv(story.PARENT/'tables/assay_audit.csv'),
            story.read_csv(story.PARENT/'tables/observed_assay_effects.csv'))
        self.assertEqual(len(reports), 273)


if __name__ == '__main__':
    unittest.main()
