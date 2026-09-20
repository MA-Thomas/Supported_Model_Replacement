//! Hierarchical component selection using the exact winning parent score vectors.
use crate::{
    PipelineConfig, Version2Selection, audit_prepared_for_config, audit_version2, config::MODELS,
    io::*,
};
use anyhow::{Context, Result, ensure};
use directed_round_robin_organizer::{input::LoadedBundle, load_bundle, system::SystemRegistry};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

#[derive(Debug, Serialize, Deserialize)]
pub struct FinalistBranch {
    #[serde(default)]
    pub numerically_pending: bool,
    pub systems: Vec<String>,
    pub component_by_system: BTreeMap<String, String>,
    pub status: String,
}

pub fn audit_selection_for(
    output: &Path,
    config: &Path,
    bundle: &LoadedBundle,
) -> Result<Version2Selection> {
    audit_version2(output)?;
    let manifest: serde_json::Value = read_json(&output.join("manifest.json"))?;
    let selection: Version2Selection = read_json(&output.join("selection.json"))?;
    ensure!(
        manifest["bundle_content_hash"] == bundle.manifest.bundle_content_hash
            && manifest["tournament_id"] == selection.tournament_id
            && selection.tournament_id
                == directed_round_robin_organizer::plan::tournament_id(bundle)
                    .map_err(anyhow::Error::msg)?
            && selection.metric == bundle.spec.metric,
        "selection belongs to a different tournament"
    );
    let sources = manifest["source_files"]
        .as_array()
        .context("selection sources")?;
    let required = [
        config.canonicalize()?,
        bundle.root.join("bundle_manifest.json").canonicalize()?,
    ];
    for path in required {
        ensure!(
            sources
                .iter()
                .any(|s| s["path"].as_str().is_some_and(|p| output
                    .join(p)
                    .canonicalize()
                    .ok()
                    .as_ref()
                    == Some(&path))),
            "selection does not bind the requested config/bundle"
        );
    }
    ensure!(
        selection.s_op.iter().collect::<BTreeSet<_>>().len() == selection.s_op.len()
            && selection.s_op.iter().all(|id| bundle
                .registry
                .systems
                .iter()
                .any(|s| &s.system_id == id)),
        "invalid finalist IDs"
    );
    Ok(selection)
}

type Identity = Vec<String>;
fn identities(bundle: &LoadedBundle) -> Result<BTreeMap<Identity, (usize, bool)>> {
    parse_identities(
        &bundle.root.join("evaluations/nci/endpoint_identities.csv"),
        &bundle.evaluations["nci"],
    )
}

fn parse_identities(
    path: &Path,
    evaluation: &directed_round_robin_organizer::input::LoadedEvaluation,
) -> Result<BTreeMap<Identity, (usize, bool)>> {
    let indices: BTreeMap<_, _> = evaluation
        .endpoint_ids
        .iter()
        .enumerate()
        .map(|(i, id)| (id.as_str(), i))
        .collect();
    let mut reader = csv::Reader::from_path(path)?;
    let headers = reader.headers()?.clone();
    let column = |name: &str| {
        headers
            .iter()
            .position(|s| s == name)
            .with_context(|| format!("missing identity column {name}"))
    };
    let id = column("endpoint_id")?;
    let label = column("label")?;
    let fields = [
        "peptide",
        "hla",
        "patient_id",
        "gene",
        "cancer_type",
        "wt_mt_group_id",
    ]
    .map(column)
    .into_iter()
    .collect::<Result<Vec<_>>>()?;
    let mut result = BTreeMap::new();
    let mut seen = BTreeSet::new();
    for row in reader.records() {
        let row = row?;
        let index = *indices.get(&row[id]).context("unknown identity endpoint")?;
        ensure!(seen.insert(index), "duplicate endpoint identity");
        let y = match &row[label] {
            "0" => false,
            "1" => true,
            _ => anyhow::bail!("invalid identity label"),
        };
        ensure!(
            evaluation.labels[index] == y,
            "identity label differs from bundle"
        );
        let key = fields.iter().map(|&i| row[i].to_owned()).collect();
        ensure!(
            result.insert(key, (index, y)).is_none(),
            "duplicate biological identity"
        );
    }
    ensure!(
        result.len() == evaluation.endpoint_ids.len(),
        "incomplete identity coverage"
    );
    Ok(result)
}

pub fn prepare_finalists(
    config_path: &Path,
    prepared: &Path,
    version2: &Path,
    output: &Path,
) -> Result<()> {
    audit_prepared_for_config(prepared, config_path)?;
    let config = PipelineConfig::load(config_path)?;
    let stage = stage_path(output)?;
    let result = (|| {
        let mut branches = BTreeMap::new();
        let mut sources = BTreeMap::new();
        sources.insert(
            relative_path(output, config_path)?,
            sha256_file(config_path)?,
        );
        for metric in ["pr", "roc"] {
            let mut baseline: Option<BTreeMap<Identity, bool>> = None;
            let mut registry = SystemRegistry {
                schema_version: 2,
                systems: vec![],
            };
            let mut scores = BTreeMap::<String, Vec<f64>>::new();
            let mut components = BTreeMap::new();
            let mut parents = Vec::new();
            for model in MODELS {
                let parent_root = prepared.join(model).join(metric).join("bundle");
                let bundle = load_bundle(&parent_root).map_err(anyhow::Error::msg)?;
                ensure!(
                    bundle.spec.annotations.get("config_sha256")
                        == Some(&json!(sha256_file(config_path)?)),
                    "parent config mismatch"
                );
                let mut expected_spec = config.tournament_spec(metric)?;
                expected_spec
                    .annotations
                    .insert("component_model_id".into(), json!(model));
                expected_spec
                    .annotations
                    .insert("candidate_count".into(), json!(config.candidate_count));
                expected_spec
                    .annotations
                    .insert("config_sha256".into(), json!(sha256_file(config_path)?));
                ensure!(
                    bundle.policy_hash
                        == expected_spec
                            .assessment_policy_hash()
                            .map_err(anyhow::Error::msg)?,
                    "parent policy mismatch"
                );
                let selected_root = version2.join(model).join(metric);
                let selection = audit_selection_for(&selected_root, config_path, &bundle)?;
                let identities = identities(&bundle)?;
                let labels: BTreeMap<_, _> = identities
                    .iter()
                    .map(|(k, (_, y))| (k.clone(), *y))
                    .collect();
                if let Some(base) = &baseline {
                    ensure!(
                        *base == labels,
                        "component observation identities/labels differ"
                    );
                } else {
                    baseline = Some(labels);
                }
                for id in &selection.s_op {
                    let system = bundle
                        .registry
                        .systems
                        .iter()
                        .find(|s| &s.system_id == id)
                        .context("missing finalist")?
                        .clone();
                    let values = &bundle.evaluations["nci"].scores[id];
                    scores.insert(
                        id.clone(),
                        identities.values().map(|(i, _)| values[*i]).collect(),
                    );
                    components.insert(id.clone(), model.to_string());
                    registry.systems.push(system);
                }
                for root in [&parent_root, &selected_root] {
                    for relative in collect_file_hashes(root)?
                        .keys()
                        .chain(std::iter::once(&"manifest.json".to_string()))
                    {
                        let source = root.join(relative);
                        if source.is_file() {
                            sources.insert(relative_path(output, &source)?, sha256_file(&source)?);
                        }
                    }
                }
                parents.push(json!({"component": model, "tournament_id": selection.tournament_id,
                    "s_op": selection.s_op, "numerical_status": selection.numerical_status, "selection_sha256": sha256_file(&selected_root.join("selection.json"))?}));
            }
            registry
                .systems
                .sort_by(|a, b| a.system_id.cmp(&b.system_id));
            let branch = stage.join(metric);
            fs::create_dir(&branch)?;
            let ids: Vec<_> = registry
                .systems
                .iter()
                .map(|s| s.system_id.clone())
                .collect();
            let status = match ids.len() {
                0 => "empty",
                1 => "singleton",
                _ => "tournament",
            };
            write_json_new(&branch.join("parents.json"), &parents)?;
            if ids.len() >= 2 {
                let root = branch.join("bundle");
                let eval = root.join("evaluations/nci");
                fs::create_dir_all(&eval)?;
                let mut spec = config.tournament_spec(metric)?;
                spec.annotations
                    .insert("config_sha256".into(), json!(sha256_file(config_path)?));
                spec.annotations
                    .insert("selection_stage".into(), json!("component_finalists"));
                write_json(&root.join("systems.json"), &registry)?;
                write_json(&root.join("tournament_spec.json"), &spec)?;
                let mut endpoints = csv::Writer::from_path(eval.join("endpoints.csv"))?;
                endpoints.write_record(["endpoint_id", "label"])?;
                let mut identity_writer =
                    csv::Writer::from_path(eval.join("endpoint_identities.csv"))?;
                identity_writer.write_record([
                    "endpoint_id",
                    "peptide",
                    "hla",
                    "patient_id",
                    "gene",
                    "cancer_type",
                    "wt_mt_group_id",
                    "label",
                ])?;
                let mut writer = csv::Writer::from_path(eval.join("scores.csv"))?;
                let mut header = vec!["endpoint_id".to_owned()];
                header.extend(registry.systems.iter().map(|s| s.score_column.clone()));
                writer.write_record(header)?;
                for (i, (key, y)) in baseline
                    .context("no parent observations")?
                    .iter()
                    .enumerate()
                {
                    let id = format!("nci_finalist_{i:06}");
                    let label = u8::from(*y).to_string();
                    endpoints.write_record([id.clone(), label.clone()])?;
                    let mut identity = vec![id.clone()];
                    identity.extend(key.clone());
                    identity.push(label);
                    identity_writer.write_record(identity)?;
                    let mut row = vec![id];
                    row.extend(ids.iter().map(|s| scores[s][i].to_string()));
                    writer.write_record(row)?;
                }
                endpoints.flush()?;
                writer.flush()?;
                identity_writer.flush()?;
                write_json(
                    &eval.join("source_provenance.json"),
                    &json!({"stage": "component_finalists", "parents": parents,
                    "endpoint_identities_sha256": sha256_file(&eval.join("endpoint_identities.csv"))?}),
                )?;
                let evaluation = crate::bundle::evaluation_manifest(&root, "nci")?;
                crate::bundle::finalize_bundle(
                    &root,
                    config.provider_identity.clone(),
                    spec.metric,
                    vec![evaluation],
                )?;
            }
            branches.insert(
                metric,
                FinalistBranch {
                    numerically_pending: parents.iter().any(|p| {
                        p["numerical_status"]
                            .as_str()
                            .is_some_and(|s| s != "resolved")
                    }),
                    systems: ids,
                    component_by_system: components,
                    status: status.into(),
                },
            );
        }
        write_json_new(
            &stage.join("manifest.json"),
            &json!({"schema_version": 1, "kind": "nci_component_finalists",
            "branches": branches, "source_files": sources, "files": collect_file_hashes(&stage)?}),
        )?;
        fs::rename(&stage, output)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(stage);
    }
    result
}

pub fn audit_finalists(root: &Path) -> Result<BTreeMap<String, FinalistBranch>> {
    let manifest: serde_json::Value = read_json(&root.join("manifest.json"))?;
    ensure!(
        manifest["schema_version"] == 1 && manifest["kind"] == "nci_component_finalists",
        "unknown finalist package"
    );
    let files: BTreeMap<String, String> = serde_json::from_value(manifest["files"].clone())?;
    ensure!(files == collect_file_hashes(root)?, "finalist files differ");
    let sources: BTreeMap<String, String> =
        serde_json::from_value(manifest["source_files"].clone())?;
    for (path, hash) in sources {
        ensure!(
            sha256_file(&root.join(path))? == hash,
            "finalist parent source changed"
        );
    }
    let branches: BTreeMap<String, FinalistBranch> =
        serde_json::from_value(manifest["branches"].clone())?;
    ensure!(
        branches.keys().map(String::as_str).collect::<Vec<_>>() == ["pr", "roc"],
        "finalist metrics incomplete"
    );
    for (metric, branch) in &branches {
        let parents: Vec<serde_json::Value> = read_json(&root.join(metric).join("parents.json"))?;
        ensure!(
            branch.numerically_pending
                == parents.iter().any(|p| p["numerical_status"]
                    .as_str()
                    .is_some_and(|s| s != "resolved")),
            "finalist numerical status differs from parents"
        );

        let mut components = BTreeMap::new();
        ensure!(parents.len() == MODELS.len(), "parent coverage incomplete");
        for (parent, model) in parents.iter().zip(MODELS) {
            ensure!(parent["component"] == model, "parent component mismatch");
            let ids: Vec<String> = serde_json::from_value(parent["s_op"].clone())?;
            for id in ids {
                ensure!(
                    components.insert(id, model.to_owned()).is_none(),
                    "duplicate finalist"
                );
            }
        }
        ensure!(
            branch.component_by_system == components
                && branch.systems == components.keys().cloned().collect::<Vec<_>>(),
            "finalist roster differs from parent winners"
        );
        let status = match branch.systems.len() {
            0 => "empty",
            1 => "singleton",
            _ => "tournament",
        };
        ensure!(branch.status == status, "invalid finalist status");
        if branch.systems.len() >= 2 {
            let bundle =
                load_bundle(&root.join(metric).join("bundle")).map_err(anyhow::Error::msg)?;
            ensure!(
                bundle
                    .registry
                    .sorted_systems()
                    .iter()
                    .map(|s| s.system_id.clone())
                    .collect::<Vec<_>>()
                    == branch.systems,
                "finalist bundle roster differs"
            );
        }
    }
    Ok(branches)
}

pub fn summarize_components(
    config: &Path,
    finalists: &Path,
    selections: &Path,
    output: &Path,
) -> Result<()> {
    let branches = audit_finalists(finalists)?;
    let mut summary = BTreeMap::new();
    for (metric, branch) in branches {
        let mut pending = branch.numerically_pending;
        let ids = if branch.systems.len() < 2 {
            branch.systems
        } else {
            let bundle =
                load_bundle(&finalists.join(&metric).join("bundle")).map_err(anyhow::Error::msg)?;
            let selection = audit_selection_for(&selections.join(&metric), config, &bundle)?;
            pending |= selection
                .numerical_status
                .as_deref()
                .is_some_and(|s| s != "resolved");
            selection.s_op
        };
        let components: BTreeSet<_> = ids
            .iter()
            .map(|id| {
                branch
                    .component_by_system
                    .get(id)
                    .context("unknown final system")
            })
            .collect::<Result<_>>()?;
        summary.insert(metric, json!({"s_op": ids, "possible_components": components, "winning_components": if pending { None } else { Some(&components) },
            "numerically_pending": pending,
            "unique_winning_component": if !pending && components.len() == 1 { components.first().copied() } else { None },
            "status": if pending { "pending_numerical_resolution" } else if components.is_empty() { "empty" } else if components.len() == 1 { "unique_component" } else { "tied_components" }}));
    }
    let value = json!({"schema_version": 1, "selection_scope": "hierarchical_component_finalists", "metrics": summary});
    if output.exists() {
        ensure!(
            read_json::<serde_json::Value>(output)? == value,
            "existing summary differs"
        );
    } else {
        write_json_new(output, &value)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn identity_alignment_handles_order_and_rejects_label_duplicates_and_gaps() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("ids.csv");
        let evaluation = directed_round_robin_organizer::input::LoadedEvaluation {
            evaluation_id: "nci".into(),
            endpoint_ids: vec!["e0".into(), "e1".into()],
            labels: vec![true, false],
            positive_count: 1,
            negative_count: 1,
            label_vector_hash: String::new(),
            scores: BTreeMap::new(),
            score_vector_hashes: BTreeMap::new(),
            raw_endpoints_hash: String::new(),
            raw_scores_hash: String::new(),
        };
        let header = "endpoint_id,peptide,hla,patient_id,gene,cancer_type,wt_mt_group_id,label\n";
        fs::write(
            &path,
            format!("{header}e1,a,H,P,G,C,W,0\ne0,z,H,P,G,C,W,1\n"),
        )
        .unwrap();
        let aligned = parse_identities(&path, &evaluation).unwrap();
        assert_eq!(
            aligned.values().map(|(i, _)| *i).collect::<Vec<_>>(),
            [1, 0]
        );
        for rows in [
            "e0,a,H,P,G,C,W,0\ne1,z,H,P,G,C,W,0\n", // wrong label
            "e0,a,H,P,G,C,W,1\ne1,a,H,P,G,C,W,0\n", // same biological key
            "e0,a,H,P,G,C,W,1\n",                   // omitted endpoint
            "e0,a,H,P,G,C,W,1\ne0,z,H,P,G,C,W,1\n",
        ] {
            // duplicated endpoint
            fs::write(&path, format!("{header}{rows}")).unwrap();
            assert!(parse_identities(&path, &evaluation).is_err());
        }
    }

    #[test]
    fn empty_and_singleton_finalists_publish_without_fabricating_a_tournament() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let mut branches = BTreeMap::new();
        for metric in ["pr", "roc"] {
            let path = root.join(metric);
            fs::create_dir(&path).unwrap();
            let id = "full_hla__roc__regime_0";
            let parents: Vec<_> = MODELS
                .iter()
                .map(|model| {
                    json!({"component": model,
                "s_op": if metric == "roc" && *model == "full_hla" { vec![id] } else { vec![] }})
                })
                .collect();
            write_json(&path.join("parents.json"), &parents).unwrap();
            branches.insert(
                metric,
                FinalistBranch {
                    numerically_pending: false,
                    systems: if metric == "roc" {
                        vec![id.into()]
                    } else {
                        vec![]
                    },
                    component_by_system: if metric == "roc" {
                        BTreeMap::from([(id.into(), "full_hla".into())])
                    } else {
                        BTreeMap::new()
                    },
                    status: if metric == "roc" {
                        "singleton"
                    } else {
                        "empty"
                    }
                    .into(),
                },
            );
        }
        write_json(
            &root.join("manifest.json"),
            &json!({"schema_version": 1, "kind": "nci_component_finalists",
            "branches": branches, "source_files": {}, "files": collect_file_hashes(root).unwrap()}),
        )
        .unwrap();
        let output = root.parent().unwrap().join(format!(
            "summary-{}.json",
            root.file_name().unwrap().to_string_lossy()
        ));
        summarize_components(
            &root.join("unused_config"),
            root,
            &root.join("unused_selections"),
            &output,
        )
        .unwrap();
        let value: serde_json::Value = read_json(&output).unwrap();
        assert_eq!(value["metrics"]["pr"]["status"], "empty");
        assert_eq!(
            value["metrics"]["roc"]["unique_winning_component"],
            "full_hla"
        );
        fs::remove_file(&output).unwrap();
        // A singleton inherited from an unresolved parent is only possible,
        // even though there is no finalist tournament left to run.
        let parents_path = root.join("roc/parents.json");
        let mut parents: Vec<serde_json::Value> = read_json(&parents_path).unwrap();
        parents[0]["numerical_status"] = json!("unresolved_incoming");
        write_json(&parents_path, &parents).unwrap();
        let mut manifest: serde_json::Value = read_json(&root.join("manifest.json")).unwrap();
        manifest["branches"]["roc"]["numerically_pending"] = json!(true);
        manifest["files"] = json!(collect_file_hashes(root).unwrap());
        write_json(&root.join("manifest.json"), &manifest).unwrap();
        summarize_components(
            &root.join("unused_config"),
            root,
            &root.join("unused_selections"),
            &output,
        )
        .unwrap();
        let value: serde_json::Value = read_json(&output).unwrap();
        assert_eq!(
            value["metrics"]["roc"]["status"],
            "pending_numerical_resolution"
        );
        assert!(value["metrics"]["roc"]["unique_winning_component"].is_null());
        assert!(value["metrics"]["roc"]["winning_components"].is_null());
        assert_eq!(
            value["metrics"]["roc"]["possible_components"],
            json!(["full_hla"])
        );
        fs::remove_file(output).unwrap();
    }
}
