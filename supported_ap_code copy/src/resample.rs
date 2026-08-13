use rand::Rng;

use crate::{ClassCounts, Error};

#[derive(Debug, Clone)]
pub(crate) struct Pools {
    pub(crate) positive: Vec<usize>,
    pub(crate) negative: Vec<usize>,
}

impl Pools {
    pub(crate) fn new(labels: &[bool]) -> Result<Self, Error> {
        let positive: Vec<_> = labels
            .iter()
            .enumerate()
            .filter_map(|(index, &label)| label.then_some(index))
            .collect();
        let negative: Vec<_> = labels
            .iter()
            .enumerate()
            .filter_map(|(index, &label)| (!label).then_some(index))
            .collect();
        if positive.is_empty() || negative.is_empty() {
            return Err(Error::MissingClass);
        }
        Ok(Self { positive, negative })
    }

    pub(crate) fn counts(&self) -> ClassCounts {
        ClassCounts::new(self.positive.len(), self.negative.len())
            .expect("validated pools contain both classes")
    }
}

pub(crate) fn stratified_multiplicities<R: Rng + ?Sized>(
    pools: &Pools,
    counts: ClassCounts,
    rng: &mut R,
    out: &mut [usize],
) {
    out.fill(0);
    for _ in 0..counts.positive() {
        let observation = pools.positive[rng.gen_range(0..pools.positive.len())];
        out[observation] += 1;
    }
    for _ in 0..counts.negative() {
        let observation = pools.negative[rng.gen_range(0..pools.negative.len())];
        out[observation] += 1;
    }
}
