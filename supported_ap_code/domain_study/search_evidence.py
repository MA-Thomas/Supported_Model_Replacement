"""Read numerical search evidence without collapsing uncertainty into failure."""
import math


def effect_bounds(effect):
    value = effect['value']
    search = effect.get('search')
    if not math.isfinite(value):
        raise ValueError('non-finite retained effect')
    if not search:
        return value, value
    lower, upper, sampled = (search[k] for k in ('lower_bound', 'upper_bound', 'sampled_value'))
    if not all(math.isfinite(x) for x in (lower, upper, sampled)) or not lower == value <= sampled <= upper:
        raise ValueError('invalid retained-effect enclosure')
    if search['stop_reason'] not in {'accuracy_reached', 'budget_exhausted', 'floating_point_limit'}:
        raise ValueError('unknown search stop reason')
    return lower, upper


def bounds_of(summary):
    return summary.get('prevalence_search') or summary.get('search')


def status(summary):
    bounds = bounds_of(summary)
    if bounds:
        return bounds['verdict']
    passed = summary.get('supported', summary.get('verdict') == 'supported_replacement')
    return 'verified_pass' if passed else 'verified_failure'


def validate_summary(summary, lower, upper, policy):
    """Check independently reduced endpoints; roundoff allowance is validation-only."""
    bounds = bounds_of(summary)
    if not bounds:
        if any(abs(summary[k] - x) > 1e-10 for k, x in zip(('magnitude', 'survival'), lower)):
            raise ValueError('summary differs from independent reduction')
        passing = lower[0] > policy['delta'] and lower[1] > policy['gamma']
        if summary['supported'] != passing:
            raise ValueError('summary verdict mismatch')
        return
    lo = [bounds['supported_magnitude_lower'], bounds['literal_survival_lower']]
    hi = [bounds['supported_magnitude_upper'], bounds['literal_survival_upper']]
    if not all(math.isfinite(x) for x in lo + hi) or any(a > b for a, b in zip(lo, hi)):
        raise ValueError('invalid decision enclosure')
    for a, b, expected_lo, expected_hi in zip(lo, hi, lower, upper):
        if abs(a - expected_lo) > 1e-10 or abs(b - expected_hi) > 1e-10:
            raise ValueError('decision enclosure differs from independent reduction')
    if summary['magnitude'] != lo[0] or summary['survival'] != lo[1]:
        raise ValueError('summary must report its conservative endpoints')
    passing = lo[0] > policy['delta'] and lo[1] > policy['gamma']
    failing = hi[0] <= policy['delta'] or hi[1] <= policy['gamma']
    expected = 'verified_pass' if passing else 'verified_failure' if failing else 'unresolved'
    if bounds['verdict'] != expected or summary['supported'] != passing:
        raise ValueError('decision status disagrees with its bounds')


def effect_columns(effect, prefix=''):
    lower, upper = effect_bounds(effect)
    search = effect.get('search') or {}
    return {prefix + key: value for key, value in dict(
        effect_lower=lower, effect_upper=upper,
        sampled_effect=search.get('sampled_value', effect['value']),
        sampled_prevalence=effect['limiting_prevalence'],
        search_stop=search.get('stop_reason', 'finite_set'),
        search_evaluations=search.get('evaluations'),
    ).items()}


def evidence_columns(summary, prefix=''):
    bounds = bounds_of(summary) or {}
    magnitude = summary.get('supported_magnitude', summary.get('magnitude'))
    survival = summary.get('literal_survival', {}).get('subset_fraction', summary.get('survival'))
    return {prefix + key: value for key, value in dict(
        numerical_status=status(summary),
        magnitude_lower=bounds.get('supported_magnitude_lower', magnitude),
        magnitude_upper=bounds.get('supported_magnitude_upper', magnitude),
        survival_lower=bounds.get('literal_survival_lower', survival),
        survival_upper=bounds.get('literal_survival_upper', survival),
    ).items()}
