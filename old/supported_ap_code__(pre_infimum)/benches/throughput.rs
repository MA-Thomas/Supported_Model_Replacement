//! Throughput for the two paths that dominate real cost.
//!
//! The tie-block precomputation is the crate's main structural optimization,
//! so a tie-free input measures the worst case rather than the common one.
//! Both are reported. Conditional-null calibration is reported separately
//! because its `M * B` resampling sweeps, not the `O(B log B)` sort, set the
//! wall time.

use std::hint::black_box;
use std::time::Instant;

use supported_ap::{
    BootstrapOptions, ConditionalNullCalibrationOptions, Evaluation, NullDistributionStorage,
    Parallelism, PermutationCount, PermutationOptions, ReferencePrevalence, ReplicateCount,
};

const WARMUP: usize = 2;
const SAMPLES: usize = 5;

fn main() {
    let prevalence = ReferencePrevalence::new(0.01).unwrap();
    let sample_size = 10_000;
    let labels: Vec<bool> = (0..sample_size).map(|index| index % 20 == 0).collect();

    // 7919 is prime and coprime with the sample size, so this is a bijection
    // and every score is distinct: the maximum number of threshold blocks.
    let distinct: Vec<f64> = (0..sample_size)
        .map(|index| ((index * 7_919) % sample_size) as f64)
        .collect();
    // A coarse scale collapses the same data into 50 blocks.
    let tied: Vec<f64> = distinct
        .iter()
        .map(|score| (score / 200.0).floor())
        .collect();

    let base = BootstrapOptions {
        replicates: ReplicateCount::new(1_000).unwrap(),
        seed: 42,
        parallelism: Parallelism::Sequential,
        ..BootstrapOptions::default()
    };

    for (label, scores) in [("distinct scores", &distinct), ("50 tie blocks", &tied)] {
        let evaluation = Evaluation::new(scores, &labels).unwrap();
        bench(
            &format!("bootstrap, {label}, sequential"),
            &evaluation,
            || {
                let _ = black_box(evaluation.supported_cnap(prevalence, base).unwrap());
            },
        );

        #[cfg(feature = "parallel")]
        bench(
            &format!("bootstrap, {label}, parallel"),
            &evaluation,
            || {
                let _ = black_box(
                    evaluation
                        .supported_cnap(
                            prevalence,
                            BootstrapOptions {
                                parallelism: Parallelism::Auto,
                                ..base
                            },
                        )
                        .unwrap(),
                );
            },
        );
    }

    // Conditional-null calibration at deliberately small M and B: the full
    // defaults would dominate the whole benchmark run.
    let evaluation = Evaluation::new(&tied, &labels).unwrap();
    let conditional_null_calibration = ConditionalNullCalibrationOptions {
        bootstrap: BootstrapOptions {
            replicates: ReplicateCount::new(200).unwrap(),
            ..base
        },
        permutation: PermutationOptions {
            permutations: PermutationCount::new(20).unwrap(),
            seed: 11,
            parallelism: Parallelism::Sequential,
            storage: NullDistributionStorage::SummaryOnly,
        },
    };
    bench(
        "conditional-null calibration, 50 tie blocks, M=20 B=200",
        &evaluation,
        || {
            let _ = black_box(
                evaluation
                    .conditionally_calibrated_supported_cnap(
                        prevalence,
                        conditional_null_calibration,
                    )
                    .unwrap(),
            );
        },
    );
}

fn bench<F>(label: &str, evaluation: &Evaluation, mut run: F)
where
    F: FnMut(),
{
    for _ in 0..WARMUP {
        run();
    }
    let mut timings = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let started = Instant::now();
        run();
        timings.push(started.elapsed());
    }
    timings.sort();
    let median = timings[SAMPLES / 2];
    let best = timings[0];
    println!(
        "{label}: {} units, {} blocks, median {:.3?}, best {:.3?}",
        evaluation.len(),
        evaluation.threshold_count(),
        median,
        best
    );
}
