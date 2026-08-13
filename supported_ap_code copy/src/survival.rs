use serde::{Deserialize, Serialize};

use crate::{Error, SupportOrder, SurvivalFloor};

/// Literal survival of a finite indexed effect list at a strict floor.
///
/// `subset_fraction` is `choose(R_d, K) / choose(L, K)`, where `R_d` effects
/// strictly exceed the floor and `L` is the finite list length. It is not a
/// probability statement about unseen evaluations.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LiteralSurvival {
    pub survivor_count: usize,
    pub list_length: usize,
    pub support_order: SupportOrder,
    pub survival_floor: SurvivalFloor,
    pub survivor_fraction: f64,
    pub subset_fraction: f64,
}

pub fn literal_survival(
    effects: &[f64],
    order: SupportOrder,
    floor: SurvivalFloor,
) -> Result<LiteralSurvival, Error> {
    validate_effects(effects)?;
    let k = order.get();
    if k > effects.len() {
        return Err(Error::SupportOrderExceedsList {
            order: k,
            list_length: effects.len(),
        });
    }
    let survivors = effects
        .iter()
        .filter(|&&effect| effect > floor.get())
        .count();
    Ok(LiteralSurvival {
        survivor_count: survivors,
        list_length: effects.len(),
        support_order: order,
        survival_floor: floor,
        survivor_fraction: survivors as f64 / effects.len() as f64,
        subset_fraction: choose_ratio(survivors, effects.len(), k),
    })
}

pub(crate) fn choose_ratio(successes: usize, trials: usize, order: usize) -> f64 {
    if successes < order {
        return 0.0;
    }
    if successes == trials {
        return 1.0;
    }
    (0..order)
        .map(|index| (successes - index) as f64 / (trials - index) as f64)
        .product()
}

fn validate_effects(effects: &[f64]) -> Result<(), Error> {
    if effects.is_empty() {
        return Err(Error::EmptyEffects);
    }
    for (index, &value) in effects.iter().enumerate() {
        if !value.is_finite() {
            return Err(Error::InvalidEffect { index, value });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_floor_and_distinct_subset_fraction_match_definition() {
        let summary = literal_survival(
            &[-0.1, 0.0, 0.2, 0.3],
            SupportOrder::new(2).unwrap(),
            SurvivalFloor::new(0.0).unwrap(),
        )
        .unwrap();
        assert_eq!(summary.survivor_count, 2);
        assert_eq!(summary.list_length, 4);
        assert_eq!(summary.survivor_fraction, 0.5);
        assert!((summary.subset_fraction - 1.0 / 6.0).abs() < 1e-14);
    }
}
