import argparse
import csv
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

import stage2_run_contract as contract


class Stage2RunContractTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.query = self.root / "query.csv"
        self.query.write_text(
            "peptide,HLA-RE,PatientID,TCGA_EXPR_TYPE,env_id,gene,count\n"
            "PEPTIDEA,A0101,P1,PDAC,0,G1,1\n"
            "PEPTIDEB,A0101,P1,PDAC,0,G2,1\n",
            encoding="utf-8",
        )
        self.param_file = self.root / "param_sets.csv"
        self.param_file.write_text(
            "d_pos,d_neg,steepness_pos,steepness_neg,tau_thymus\n"
            "3,2,1,1,1000\n",
            encoding="utf-8",
        )
        self.mn_file = self.root / "mn_tuples.csv"
        self.mn_file.write_text(
            "at_least_M,at_most_N\n1,1\n1,2\n", encoding="utf-8"
        )

    def tearDown(self):
        self.tmp.cleanup()

    def test_active_chunks_exclude_zero_input_environments(self):
        rows = contract.load_query(self.query)
        self.assertEqual(contract.active_chunks(rows, 3, 3), [0])

    def test_representation_validation_accepts_full_and_mono(self):
        rows = contract.load_query(self.query)
        full = self.root / "full_env.csv"
        full.write_text(
            'env_id,allele_environment\n0,"A0101,B0702"\n',
            encoding="utf-8",
        )
        mono = self.root / "mono_env.csv"
        mono.write_text(
            "env_id,allele_environment\n0,A0101\n",
            encoding="utf-8",
        )
        full_summary = contract.validate_environment_representation(
            rows, full, "full", 1
        )
        mono_summary = contract.validate_environment_representation(
            rows, mono, "mono", 1
        )
        self.assertEqual((full_summary.min_alleles, full_summary.max_alleles), (2, 2))
        self.assertEqual((mono_summary.min_alleles, mono_summary.max_alleles), (1, 1))

    def test_mono_representation_rejects_multi_allele_environment(self):
        rows = contract.load_query(self.query)
        env_dict = self.root / "not_mono.csv"
        env_dict.write_text(
            'env_id,allele_environment\n0,"A0101,B0702"\n',
            encoding="utf-8",
        )
        with self.assertRaises(contract.ContractError):
            contract.validate_environment_representation(rows, env_dict, "mono", 1)

    def test_representation_rejects_query_hla_absent_from_environment(self):
        rows = contract.load_query(self.query)
        env_dict = self.root / "wrong_hla.csv"
        env_dict.write_text(
            "env_id,allele_environment\n0,B0702\n",
            encoding="utf-8",
        )
        with self.assertRaises(contract.ContractError):
            contract.validate_environment_representation(rows, env_dict, "mono", 1)

    def test_unique_compute_keys_accepts_deduplicated_roster(self):
        rows = contract.load_query(self.query)
        # self.query has two distinct peptides on the same HLA/env — no dup keys.
        contract.assert_unique_compute_keys(rows, self.query)

    def test_unique_compute_keys_rejects_duplicate_rows(self):
        dup = self.root / "dup_query.csv"
        # PEPTIDEA/A0101/env0/PDAC appears twice (e.g. a homozygous allele copy
        # or a repeated parent peptide leaking into the roster).
        dup.write_text(
            "peptide,HLA-RE,PatientID,TCGA_EXPR_TYPE,env_id,gene,count\n"
            "PEPTIDEA,A0101,P1,PDAC,0,G1,1\n"
            "PEPTIDEA,A0101,P1,PDAC,0,G1,1\n"
            "PEPTIDEB,A0101,P1,PDAC,0,G2,1\n",
            encoding="utf-8",
        )
        rows = contract.load_query(dup)
        with self.assertRaises(contract.ContractError):
            contract.assert_unique_compute_keys(rows, dup)

    def test_unique_compute_keys_normalizes_hla_and_case(self):
        dup = self.root / "dup_norm_query.csv"
        # Same compute key after normalization: 'HLA-A0101' vs 'A0101',
        # lower-case vs upper-case peptide.
        dup.write_text(
            "peptide,HLA-RE,PatientID,TCGA_EXPR_TYPE,env_id,gene,count\n"
            "peptidea,HLA-A0101,P1,PDAC,0,G1,1\n"
            "PEPTIDEA,A0101,P1,PDAC,0,G1,1\n",
            encoding="utf-8",
        )
        rows = contract.load_query(dup)
        with self.assertRaises(contract.ContractError):
            contract.assert_unique_compute_keys(rows, dup)

    def test_prepare_run_keeps_common_identity_across_qpi_and_pn(self):
        runner_root = self.root / "runner_root"
        runner_root.mkdir()
        (runner_root / "data" / "matrices").mkdir(parents=True)
        (runner_root / "data" / "matrices" / "matrix.bin").write_bytes(b"matrix")
        env_dict = self.root / "hla_env_dict.csv"
        env_dict.write_text(
            "env_id,allele_environment\n0,A0101\n1,A0101\n2,A0101\n",
            encoding="utf-8",
        )
        config = self.root / "model.toml"
        config.write_text(
            f'query_peptide_input_tuples_file = "{self.query}"\n'
            f'hla_env_dict = "{env_dict}"\n',
            encoding="utf-8",
        )
        runner = self.root / "Runner"
        submit = self.root / "submit.sh"
        worker = self.root / "run.sh"
        helper = self.root / "stage2_run_contract.py"
        pmhc = self.root / "pmhc.toml"
        pmhc_parameters = self.root / "pmhc_parameters.csv"
        for path in (runner, submit, worker, helper, pmhc):
            path.write_text(path.name, encoding="utf-8")
        pmhc_parameters.write_text(
            "hla,beta_pos,beta_neg,lambda\nA0101,0.3,0.4,1.2\n",
            encoding="utf-8",
        )
        run_root = self.root / "outputs" / "runs" / "test"

        base = dict(
            run_root=str(run_root),
            run_id="test",
            dataset="PDAC",
            runner_root=str(runner_root),
            runner=str(runner),
            q_model_config=str(config),
            submit_script=str(submit),
            run_script=str(worker),
            contract_helper=str(helper),
            total_env_ids=3,
            env_chunks=3,
            total_params=1,
            param_chunks=1,
            param_file=str(self.param_file),
            mn_tuples=str(self.mn_file),
            pmhc_config=str(pmhc),
            pmhc_parameters_file=str(pmhc_parameters),
            in_vitro=1,
            peptide_conc=1000.0,
            pn_hla_scope="focal",
            max_num_ps_values_log2=14,
            hla_environment_representation="full",
            representation_crosswalk="",
        )
        qpi_args = argparse.Namespace(mode="qpi", **base)
        pn_args = argparse.Namespace(mode="pn", **base)
        self.assertEqual(contract.command_prepare_run(qpi_args), 0)
        common_before = json.loads((run_root / "run_manifest.json").read_text())[
            "common_fingerprint"
        ]
        common_roles = {
            item["role"]
            for item in json.loads((run_root / "run_manifest.json").read_text())["inputs"]
        }
        self.assertIn("contract_helper", common_roles)
        qpi_roles = {
            item["role"]
            for item in json.loads((run_root / "qpi_manifest.json").read_text())["inputs"]
        }
        self.assertIn("pmhc_parameters", qpi_roles)
        self.assertEqual(contract.command_prepare_run(pn_args), 0)
        pn_manifest = json.loads((run_root / "pn_manifest.json").read_text())
        self.assertEqual(pn_manifest["pn_hla_scope"], "focal")
        self.assertEqual(pn_manifest["max_num_ps_values_log2"], 14)
        self.assertEqual(pn_manifest["max_num_ps_values"], 16384)
        all_run_root = self.root / "outputs" / "runs" / "test_all"
        all_args = argparse.Namespace(
            mode="pn",
            **{
                **base,
                "run_root": str(all_run_root),
                "run_id": "test_all",
                "pn_hla_scope": "all",
            },
        )
        self.assertEqual(contract.command_prepare_run(all_args), 0)
        all_manifest = json.loads((all_run_root / "pn_manifest.json").read_text())
        self.assertNotEqual(
            pn_manifest["mode_fingerprint"], all_manifest["mode_fingerprint"]
        )
        larger_k_run_root = self.root / "outputs" / "runs" / "test_larger_k"
        larger_k_args = argparse.Namespace(
            mode="pn",
            **{
                **base,
                "run_root": str(larger_k_run_root),
                "run_id": "test_larger_k",
                "max_num_ps_values_log2": 15,
            },
        )
        self.assertEqual(contract.command_prepare_run(larger_k_args), 0)
        larger_k_manifest = json.loads(
            (larger_k_run_root / "pn_manifest.json").read_text()
        )
        self.assertNotEqual(
            pn_manifest["mode_fingerprint"],
            larger_k_manifest["mode_fingerprint"],
        )
        common_after = json.loads((run_root / "run_manifest.json").read_text())[
            "common_fingerprint"
        ]
        self.assertEqual(common_before, common_after)

    def test_qpi_fingerprint_changes_when_pmhc_parameters_change(self):
        runner_root = self.root / "runner_root"
        runner_root.mkdir()
        env_dict = self.root / "hla_env_dict.csv"
        env_dict.write_text("env_id,allele_environment\n0,A0101\n", encoding="utf-8")
        config = self.root / "model.toml"
        config.write_text(
            f'query_peptide_input_tuples_file = "{self.query}"\n'
            f'hla_env_dict = "{env_dict}"\n',
            encoding="utf-8",
        )
        files = {
            name: self.root / name
            for name in ("Runner", "submit.sh", "run.sh", "contract.py", "pmhc.toml")
        }
        for path in files.values():
            path.write_text(path.name, encoding="utf-8")
        pmhc_parameters = self.root / "pmhc_parameters.csv"

        def prepare(run_name, beta_pos):
            pmhc_parameters.write_text(
                f"hla,beta_pos,beta_neg,lambda\nA0101,{beta_pos},0.4,1.2\n",
                encoding="utf-8",
            )
            run_root = self.root / run_name
            args = argparse.Namespace(
                run_root=str(run_root),
                run_id=run_name,
                dataset="PDAC",
                mode="qpi",
                runner_root=str(runner_root),
                runner=str(files["Runner"]),
                q_model_config=str(config),
                submit_script=str(files["submit.sh"]),
                run_script=str(files["run.sh"]),
                contract_helper=str(files["contract.py"]),
                total_env_ids=1,
                env_chunks=1,
                total_params=1,
                param_chunks=1,
                param_file=str(self.param_file),
                mn_tuples=str(self.mn_file),
                pmhc_config=str(files["pmhc.toml"]),
                pmhc_parameters_file=str(pmhc_parameters),
                in_vitro=1,
                peptide_conc=1000.0,
                pn_hla_scope="all",
                max_num_ps_values_log2=14,
                hla_environment_representation="full",
                representation_crosswalk="",
            )
            self.assertEqual(contract.command_prepare_run(args), 0)
            return json.loads((run_root / "qpi_manifest.json").read_text())[
                "mode_fingerprint"
            ]

        first = prepare("run_a", "0.3")
        second = prepare("run_b", "0.5")
        self.assertNotEqual(first, second)

    def test_qpi_validation_publication_and_completion_manifest(self):
        staging = self.root / "stage"
        source = staging / "results_qpi_env_id_0_1"
        source.mkdir(parents=True)
        (source / "query_peptides_results_HLA_A0101_q_values.csv").write_text(
            "peptide,env_id,peptide_conc,q_value\n"
            "PEPTIDEA,0,1000,0.1\nPEPTIDEB,0,1000,0.2\n",
            encoding="utf-8",
        )
        (source / "query_peptides_results_HLA_A0101_pi_values.csv").write_text(
            "peptide,pi_value\nPEPTIDEA,0.3\nPEPTIDEB,0.4\n",
            encoding="utf-8",
        )
        final = self.root / "final"
        args = argparse.Namespace(
            mode="qpi",
            run_id="run1",
            fingerprint="abc",
            task_id=0,
            query=str(self.query),
            staging_root=str(staging),
            final_root=str(final),
            env_start=0,
            env_stop=1,
            parameter_start=0,
            parameter_stop=1,
            param_file=str(self.param_file),
            mn_tuples=str(self.mn_file),
        )
        self.assertEqual(contract.command_validate_publish(args), 0)
        manifest = final / "results_qpi_env_id_0_1" / "TASK_0.done.json"
        contract.verify_task_manifest(manifest, "abc", final, True)
        self.assertEqual(json.loads(manifest.read_text())["actual_files"], 2)

    def test_pn_validation_requires_exact_peptide_mn_grid(self):
        staging = self.root / "stage"
        source = staging / "results_pn_env_id_0_1"
        source.mkdir(parents=True)
        filename = (
            "query_peptides_results_dpos_3_dneg_2_steepness_pos_1_"
            "steepness_neg_1_fft_size_16384_tau_thymus_1000_"
            "HLA_A0101_expr_PDAC.csv"
        )
        with (source / filename).open("w", newline="", encoding="utf-8") as handle:
            writer = csv.writer(handle)
            writer.writerow(["peptide", "env_id", "M", "N", "p_pos", "p_neg"])
            for peptide in ("PEPTIDEA", "PEPTIDEB"):
                for n_value in (1, 2):
                    writer.writerow([peptide, 0, 1, n_value, 0.5, 0.6])

        final = self.root / "final"
        args = argparse.Namespace(
            mode="pn",
            run_id="run1",
            fingerprint="pnabc",
            task_id=0,
            query=str(self.query),
            staging_root=str(staging),
            final_root=str(final),
            env_start=0,
            env_stop=1,
            parameter_start=0,
            parameter_stop=1,
            param_file=str(self.param_file),
            mn_tuples=str(self.mn_file),
        )
        self.assertEqual(contract.command_validate_publish(args), 0)
        manifest = final / "results_pn_env_id_0_1" / "TASK_0.done.json"
        contract.verify_task_manifest(manifest, "pnabc", final, True)

    def test_missing_qpi_hla_file_is_fatal(self):
        source = self.root / "results_qpi_env_id_0_1"
        source.mkdir()
        (source / "query_peptides_results_HLA_A0101_q_values.csv").write_text(
            "peptide,env_id,q_value\nPEPTIDEA,0,0.1\nPEPTIDEB,0,0.2\n",
            encoding="utf-8",
        )
        with self.assertRaises(contract.ContractError):
            contract.validate_qpi(source, contract.load_query(self.query))

    def test_qpi_worker_passes_explicit_pmhc_paths_and_publishes(self):
        fake_runner = self.root / "Runner"
        capture = self.root / "runner_args.json"
        fake_runner.write_text(
            f"#!{sys.executable}\n"
            "import json, os, sys\n"
            "from pathlib import Path\n"
            "args = sys.argv[1:]\n"
            "Path(os.environ['FAKE_CAPTURE']).write_text(json.dumps(args))\n"
            "def value(flag): return args[args.index(flag) + 1]\n"
            "root = Path(value('--outputs-root'))\n"
            "start = value('--env-id-start')\n"
            "stop = value('--env-id-end')\n"
            "out = root / f'results_qpi_env_id_{start}_{stop}'\n"
            "out.mkdir(parents=True)\n"
            "(out / 'query_peptides_results_HLA_A0101_q_values.csv').write_text("
            "'peptide,env_id,peptide_conc,q_value\\nPEPTIDEA,0,1000,0.1\\nPEPTIDEB,0,1000,0.2\\n')\n"
            "(out / 'query_peptides_results_HLA_A0101_pi_values.csv').write_text("
            "'peptide,pi_value\\nPEPTIDEA,0.3\\nPEPTIDEB,0.4\\n')\n",
            encoding="utf-8",
        )
        fake_runner.chmod(0o755)
        q_model = self.root / "q_model.toml"
        pmhc_config = self.root / "pmhc.toml"
        pmhc_parameters = self.root / "pmhc_parameters.csv"
        q_model.write_text("model = 'fixture'\n", encoding="utf-8")
        pmhc_config.write_text("model = 'fixture'\n", encoding="utf-8")
        pmhc_parameters.write_text(
            "hla,beta_pos,beta_neg,lambda\nA0101,0.3,0.4,1.2\n",
            encoding="utf-8",
        )
        final = self.root / "final"
        environment = os.environ.copy()
        environment.update(
            {
                "DATASET": "PDAC",
                "COMPUTE_PN": "0",
                "COMPUTE_Q": "1",
                "COMPUTE_PI": "1",
                "COMPUTE_EVAC": "0",
                "EXECUTION_MODE": "qpi_only",
                "TOTAL_ENV_IDS": "1",
                "ENV_CHUNKS": "1",
                "RUNNER": str(fake_runner),
                "RUNNER_ROOT": str(self.root),
                "OUTPUTS_ROOT": str(final),
                "SLURM_ARRAY_TASK_ID": "0",
                "SLURM_JOB_ID": "123",
                "SLURM_CPUS_PER_TASK": "1",
                "STAGE2_MODE": "qpi",
                "RUN_ID": "fixture",
                "MODE_FINGERPRINT": "abc",
                "QUERY_INPUT_FILE": str(self.query),
                "CONTRACT_HELPER": str(Path(contract.__file__).resolve()),
                "PYTHON_BIN": sys.executable,
                "PARAM_FILE": str(self.param_file),
                "MN_TUPLES_FILE": str(self.mn_file),
                "PARAM_FILE_SHA256": contract.sha256_file(self.param_file),
                "MN_TUPLES_FILE_SHA256": contract.sha256_file(self.mn_file),
                "HLA_ENVIRONMENT_REPRESENTATION": "full",
                "Q_MODEL_CONFIG": str(q_model),
                "PMHC_CONFIG": str(pmhc_config),
                "PMHC_PARAMETERS_FILE": str(pmhc_parameters),
                "COMPUTE_IN_VITRO": "1",
                "IN_VITRO_PEPTIDE_CONC": "1000.0",
                "FAKE_CAPTURE": str(capture),
            }
        )
        worker = Path(__file__).resolve().parents[1] / "run_stage_2_new.sh"
        subprocess.run(["bash", str(worker)], env=environment, check=True)

        runner_args = json.loads(capture.read_text())
        self.assertEqual(
            runner_args[runner_args.index("--pmhc-config") + 1], str(pmhc_config)
        )
        self.assertEqual(
            runner_args[runner_args.index("--pmhc-parameters-file") + 1],
            str(pmhc_parameters),
        )
        self.assertNotIn("--parameter-file", runner_args)
        self.assertNotIn("--mn-tuples-file", runner_args)
        manifest = final / "results_qpi_env_id_0_1" / "TASK_0.done.json"
        contract.verify_task_manifest(manifest, "abc", final, True)

    def test_pn_worker_passes_hla_scope_and_top_k(self):
        fake_runner = self.root / "Runner"
        capture = self.root / "runner_args.json"
        fake_runner.write_text(
            f"#!{sys.executable}\n"
            "import json, os, sys\n"
            "from pathlib import Path\n"
            "args = sys.argv[1:]\n"
            "Path(os.environ['FAKE_CAPTURE']).write_text(json.dumps(args))\n"
            "def value(flag): return args[args.index(flag) + 1]\n"
            "root = Path(value('--outputs-root'))\n"
            "start = value('--env-id-start')\n"
            "stop = value('--env-id-end')\n"
            "out = root / f'results_pn_env_id_{start}_{stop}'\n"
            "out.mkdir(parents=True)\n"
            "name = ('query_peptides_results_dpos_3_dneg_2_steepness_pos_1_'"
            "        'steepness_neg_1_fft_size_16384_tau_thymus_1000_'"
            "        'HLA_A0101_expr_PDAC.csv')\n"
            "rows = ['peptide,env_id,M,N,p_pos,p_neg']\n"
            "for peptide in ('PEPTIDEA', 'PEPTIDEB'):\n"
            "    for n in (1, 2): rows.append(f'{peptide},0,1,{n},0.5,0.6')\n"
            "(out / name).write_text('\\n'.join(rows) + '\\n')\n",
            encoding="utf-8",
        )
        fake_runner.chmod(0o755)
        q_model = self.root / "q_model.toml"
        q_model.write_text("model = 'fixture'\n", encoding="utf-8")
        final = self.root / "final"
        environment = os.environ.copy()
        environment.update(
            {
                "DATASET": "PDAC",
                "COMPUTE_PN": "1",
                "COMPUTE_Q": "0",
                "COMPUTE_PI": "0",
                "COMPUTE_EVAC": "0",
                "EXECUTION_MODE": "pn",
                "TOTAL_ENV_IDS": "1",
                "ENV_CHUNKS": "1",
                "TOTAL_PARAMS": "1",
                "PARAM_CHUNKS": "1",
                "PN_HLA_SCOPE": "focal",
                "MAX_NUM_PS_VALUES_LOG2": "14",
                "RUNNER": str(fake_runner),
                "RUNNER_ROOT": str(self.root),
                "OUTPUTS_ROOT": str(final),
                "SLURM_ARRAY_TASK_ID": "0",
                "SLURM_JOB_ID": "124",
                "SLURM_CPUS_PER_TASK": "1",
                "STAGE2_MODE": "pn",
                "RUN_ID": "fixture",
                "MODE_FINGERPRINT": "pnabc",
                "QUERY_INPUT_FILE": str(self.query),
                "CONTRACT_HELPER": str(Path(contract.__file__).resolve()),
                "PYTHON_BIN": sys.executable,
                "PARAM_FILE": str(self.param_file),
                "MN_TUPLES_FILE": str(self.mn_file),
                "PARAM_FILE_SHA256": contract.sha256_file(self.param_file),
                "MN_TUPLES_FILE_SHA256": contract.sha256_file(self.mn_file),
                "HLA_ENVIRONMENT_REPRESENTATION": "full",
                "Q_MODEL_CONFIG": str(q_model),
                "FAKE_CAPTURE": str(capture),
            }
        )
        worker = Path(__file__).resolve().parents[1] / "run_stage_2_new.sh"
        subprocess.run(["bash", str(worker)], env=environment, check=True)

        runner_args = json.loads(capture.read_text())
        self.assertEqual(
            runner_args[runner_args.index("--pn-hla-scope") + 1], "focal"
        )
        self.assertEqual(
            runner_args[runner_args.index("--max-num-ps-values-log2") + 1], "14"
        )
        self.assertEqual(
            runner_args[runner_args.index("--parameter-file") + 1],
            str(self.param_file),
        )
        self.assertEqual(
            runner_args[runner_args.index("--mn-tuples-file") + 1],
            str(self.mn_file),
        )
        self.assertEqual(
            runner_args[
                runner_args.index("--hla-environment-representation") + 1
            ],
            "full",
        )
        manifest = final / "results_pn_env_id_0_1" / "TASK_0.done.json"
        contract.verify_task_manifest(manifest, "pnabc", final, True)

    def test_query_shards_are_balanced_disjoint_and_exhaustive(self):
        rows = contract.load_query(self.query)
        shards = [contract.select_query_shard(rows, index, 2) for index in range(2)]
        self.assertEqual([len(shard) for shard in shards], [1, 1])
        keys = [contract.query_compute_key(row) for shard in shards for row in shard]
        self.assertEqual(len(keys), len(set(keys)))
        self.assertEqual(set(keys), {contract.query_compute_key(row) for row in rows})

    def test_sharded_pn_validation_and_merge_publish_legacy_parent(self):
        plan = self.root / "pn_shard_plan.json"
        contract.write_pn_shard_plan(
            plan,
            self.query,
            total_env_ids=1,
            env_chunks=1,
            total_params=1,
            param_chunks=1,
            target_rows=1,
        )
        _, tasks = contract.load_pn_shard_plan(plan)
        self.assertEqual(len(tasks), 2)
        run_root = self.root / "run"
        filename = (
            "query_peptides_results_dpos_3_dneg_2_steepness_pos_1_"
            "steepness_neg_1_fft_size_16384_tau_thymus_1000_"
            "HLA_A0101_expr_PDAC.csv"
        )
        env_rows = contract.load_query(self.query)
        for task in tasks:
            staging = self.root / f"stage_{task.task_id}"
            source = staging / "results_pn_env_id_0_1"
            source.mkdir(parents=True)
            shard_rows = contract.select_query_shard(
                env_rows, task.shard_index, task.shard_count
            )
            with (source / filename).open("w", newline="", encoding="utf-8") as handle:
                writer = csv.writer(handle, lineterminator="\n")
                writer.writerow(["peptide", "env_id", "M", "N", "p_pos", "p_neg"])
                for row in shard_rows:
                    writer.writerow([row.peptide, row.env_id, 1, 1, 0.5, 0.6])
                    writer.writerow([row.peptide, row.env_id, 1, 2, 0.4, 0.7])
            args = argparse.Namespace(
                plan=str(plan),
                task_id=task.task_id,
                run_id="fixture",
                fingerprint="shard-fingerprint",
                query=str(self.query),
                staging_root=str(staging),
                run_root=str(run_root),
                param_file=str(self.param_file),
                mn_tuples=str(self.mn_file),
            )
            self.assertEqual(contract.command_validate_pn_shard(args), 0)

        merge = argparse.Namespace(
            plan=str(plan),
            parent_task_id=0,
            run_id="fixture",
            fingerprint="shard-fingerprint",
            query=str(self.query),
            run_root=str(run_root),
            param_file=str(self.param_file),
            mn_tuples=str(self.mn_file),
        )
        self.assertEqual(contract.command_merge_pn_shards(merge), 0)
        manifest = run_root / "results_pn_env_id_0_1" / "TASK_0.done.json"
        contract.verify_task_manifest(
            manifest, "shard-fingerprint", run_root, True
        )
        output = run_root / "results_pn_env_id_0_1" / filename
        with output.open(newline="", encoding="utf-8") as handle:
            records = list(csv.DictReader(handle))
        self.assertEqual(len(records), 4)
        self.assertEqual(
            {(row["peptide"], row["M"], row["N"]) for row in records},
            {
                ("PEPTIDEA", "1", "1"),
                ("PEPTIDEA", "1", "2"),
                ("PEPTIDEB", "1", "1"),
                ("PEPTIDEB", "1", "2"),
            },
        )


if __name__ == "__main__":
    unittest.main()
