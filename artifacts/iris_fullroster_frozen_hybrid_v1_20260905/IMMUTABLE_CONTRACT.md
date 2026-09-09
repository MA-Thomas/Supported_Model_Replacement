# Immutable frozen-hybrid tournament contract

This directory is the complete downstream realization of
`iris_fullroster_frozen_hybrid_v1_20260905`. It was generated into a new root;
none of the earlier complete-F or adaptive-selection artifacts was overwritten.
The machine-readable authority is `immutable_contract.json`.

## Scientific roster

Each PR and ROC bundle contains 65 systems:

```text
5 component models x (12 fixed L2 operators + 1 frozen adaptive L2 operator)
```

The adaptive operator is
`endpoint_local_epitope_second_hla_hybrid_v1`. Its constants are
`c=-2.2`, `kappa=0.13`, `t=-6.45`, `delta=0.02`, `B=1`, and `w=0.12`.
The function and constants are applied unchanged to all five component models,
all three cohorts, and both score branches. There is no adaptive-selection
handoff and no 70-system dual-policy bundle.

With three evaluations, each metric tournament has
`C(65,2) x 3 = 6,240` matches.

## Immutable identities

| Artifact | SHA-256 or content hash |
|---|---|
| Pipeline configuration | `3ebb32b1e84bcdead425b1b9601cf6f1c7dc360b4826fe02910b85e79c09ca8f` |
| Complete-F input manifest | `b1e42d5427382f38a4df6de72820fa06fa194d3e350eba26f2c53b89b911fc38` |
| Transfer manifest | `a1a57ba38c99f99f584df50561bbde35e68f7e6e9961008ee6adc5212901f0e7` |
| PR bundle manifest | `223589ee1fa0f6ba92148dae8586a77245d771b832ec01f80ce9f22abc760df5` |
| PR bundle content | `dc74408f1e8ec406ec62b29e091ba2a669e4bd5dcb4310df79c5f6ba6d644fab` |
| ROC bundle manifest | `99674b6d7f588abe54dab37b96b7ec40adb1bd70ce05c9da65ec4fda2f9a2b79` |
| ROC bundle content | `aa9703e64ee097d9a5cd746f29d353882c563de99f572e5b3aa8a60af1037e49` |

The transfer package contains 30 primary jobs and ten non-bundle-eligible PDAC
Rojas/Sethna diagnostic jobs. Its size is about 412 MB. The final PR/ROC
bundles total about 7.1 MB and are the only data artifacts required to execute
the cluster tournament.

## Regeneration evidence

The 60 fixed score columns in each regenerated score table were compared as
serialized strings against the previous complete-F fixed bundles. Across PR,
ROC, PDAC, COVID SPIKE, and COVID NONSPIKE, the mismatch count was zero. The
five new columns are exactly the model-specific frozen-hybrid systems.

The Rust production scorer has a bitwise parity test against the exploratory
hybrid equation. The bundle validator and the cluster helper both accepted PR
and ROC, including the exact 65-system roster and frozen constants.

## Cluster handoff

Use
`IRIS_scripts/configs/tournament/config.cluster.frozen_hybrid_v1.threshold_zero.json`
with `IRIS_scripts/tournament/submit_directed_round_robin_slurm.sh`. Copy the
repository and final bundle tree without editing the bundle contents. Build the
release organizer and generate plans on IRIS; plan identity intentionally
includes the Linux executable, so local plans are not part of this package.

Before submission, verify both bundles:

```bash
ORGANIZER=supported_ap_code/target/release/directed_round_robin_organizer
HELPER=IRIS_scripts/shared/slurm_round_robin_helper.py
BUNDLES=artifacts/iris_fullroster_frozen_hybrid_v1_20260905/full_roster_65_systems

$ORGANIZER validate-bundle --bundle "$BUNDLES/pr"
$ORGANIZER validate-bundle --bundle "$BUNDLES/roc"
python3 "$HELPER" check-frozen-hybrid-bundle --bundle "$BUNDLES/pr"
python3 "$HELPER" check-frozen-hybrid-bundle --bundle "$BUNDLES/roc"
```
