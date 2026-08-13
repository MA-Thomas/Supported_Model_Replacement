//! The reporting ladder for one model.
//!
//! Each quantity is the one above it after a single adjustment, and the size
//! of that adjustment is the gap between neighbours.

use supported_ap::{
    BootstrapOptions, ConditionalNullCalibrationOptions, Evaluation, PermutationCount,
    PermutationOptions, ReferencePrevalence, ReplicateCount,
};

fn main() -> Result<(), supported_ap::SupportedApError> {
    let scores = [0.91, 0.74, 0.74, 0.52, 0.31, 0.08];
    let labels = [true, true, false, true, false, false];
    let prevalence = ReferencePrevalence::new(0.10)?;
    let evaluation = Evaluation::new(&scores, &labels)?;

    let options = ConditionalNullCalibrationOptions {
        bootstrap: BootstrapOptions {
            replicates: ReplicateCount::new(2_000)?,
            seed: 7,
            ..BootstrapOptions::default()
        },
        permutation: PermutationOptions {
            permutations: PermutationCount::new(200)?,
            seed: 11,
            ..PermutationOptions::default()
        },
    };
    let result = evaluation.conditionally_calibrated_supported_cnap(prevalence, options)?;
    let estimate = result.estimate();

    println!(
        "observed CNAP:                  {:.4}",
        estimate.observed().cnap()
    );
    println!(
        "mean CNAP across replications:  {:.4}",
        estimate.replicate_mean()
    );
    println!(
        "  support penalty (K={}): {:.4}",
        estimate.support_order().get(),
        estimate.support_penalty()
    );
    println!(
        "supported CNAP:               {:.4}  (MC se {:.4})",
        estimate.supported(),
        estimate.monte_carlo_standard_error()
    );
    println!(
        "calibrated supported CNAP:    {:.4}  (MC se {:.4})",
        result.calibrated(),
        result.monte_carlo_standard_error()
    );

    println!();
    println!("anchors that fix the scale:");
    println!(
        "  conditional null:      {:.4}  (MC se {:.4})",
        result.conditional_null(),
        result.conditional_null_monte_carlo_standard_error()
    );
    println!("  perfect separation:    1.0000");
    println!("  permutation p-value:   {:.4}", result.p_value());

    if result.is_above_conditional_null() {
        println!();
        println!("The supported value clears its conditional null, so the calibrated");
        println!("value is read between measured chance and perfect separation.");
    } else {
        println!();
        println!("The supported value is at or below its conditional null. Report the");
        println!("result as at or below measured chance and read its direction from");
        println!("the signed observed CNAP, not from the calibrated scale.");
    }

    // The lower tail, which the headline averages over and cannot report.
    println!();
    println!(
        "level retained with frequency 0.5: {:.4}",
        estimate.survival_level(0.5)?
    );
    Ok(())
}
