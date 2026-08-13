#[cfg(not(feature = "parallel"))]
use supported_ap::SupportedApError;
use supported_ap::{
    BootstrapOptions, Evaluation, Parallelism, ReferencePrevalence, ReplicateCount,
};
#[cfg(feature = "parallel")]
use supported_ap::{
    ConditionalNullCalibrationOptions, NullDistributionStorage, PermutationCount,
    PermutationOptions,
};

fn evaluation() -> Evaluation {
    Evaluation::new(
        &[0.95, 0.82, 0.78, 0.61, 0.61, 0.44, 0.31, 0.12],
        &[true, false, true, false, true, false, true, false],
    )
    .unwrap()
}

#[cfg(feature = "parallel")]
#[test]
fn bootstrap_is_identical_across_execution_policies() {
    let evaluation = evaluation();
    let prevalence = ReferencePrevalence::new(0.1).unwrap();
    let base = BootstrapOptions {
        replicates: ReplicateCount::new(400).unwrap(),
        seed: 42,
        parallelism: Parallelism::Sequential,
        ..BootstrapOptions::default()
    };

    let sequential = evaluation.supported_cnap(prevalence, base).unwrap();
    let automatic = evaluation
        .supported_cnap(
            prevalence,
            BootstrapOptions {
                parallelism: Parallelism::Auto,
                ..base
            },
        )
        .unwrap();
    let explicit = evaluation
        .supported_cnap(
            prevalence,
            BootstrapOptions {
                parallelism: Parallelism::threads(2).unwrap(),
                ..base
            },
        )
        .unwrap();

    assert_eq!(sequential, automatic);
    assert_eq!(sequential, explicit);
}

#[cfg(feature = "parallel")]
#[test]
fn conditional_null_calibration_is_identical_across_execution_policies() {
    let evaluation = evaluation();
    let prevalence = ReferencePrevalence::new(0.1).unwrap();
    let base = ConditionalNullCalibrationOptions {
        bootstrap: BootstrapOptions {
            replicates: ReplicateCount::new(80).unwrap(),
            seed: 7,
            parallelism: Parallelism::Sequential,
            ..BootstrapOptions::default()
        },
        permutation: PermutationOptions {
            permutations: PermutationCount::new(50).unwrap(),
            seed: 11,
            parallelism: Parallelism::Sequential,
            storage: NullDistributionStorage::Full,
        },
    };
    let sequential = evaluation
        .conditionally_calibrated_supported_cnap(prevalence, base)
        .unwrap();
    let parallel = evaluation
        .conditionally_calibrated_supported_cnap(
            prevalence,
            ConditionalNullCalibrationOptions {
                bootstrap: BootstrapOptions {
                    parallelism: Parallelism::Auto,
                    ..base.bootstrap
                },
                permutation: PermutationOptions {
                    parallelism: Parallelism::threads(2).unwrap(),
                    ..base.permutation
                },
            },
        )
        .unwrap();

    assert_eq!(sequential, parallel);
}

#[cfg(not(feature = "parallel"))]
#[test]
fn explicit_threads_require_the_parallel_feature() {
    let evaluation = evaluation();
    let prevalence = ReferencePrevalence::new(0.1).unwrap();
    let result = evaluation.supported_cnap(
        prevalence,
        BootstrapOptions {
            replicates: ReplicateCount::new(10).unwrap(),
            seed: 1,
            parallelism: Parallelism::threads(2).unwrap(),
            ..BootstrapOptions::default()
        },
    );

    assert!(matches!(
        result,
        Err(SupportedApError::ParallelFeatureDisabled)
    ));
}
