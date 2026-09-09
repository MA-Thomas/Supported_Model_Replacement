use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

use crate::config::{BuildConfig, DatasetConfig, ExpectedViewCounts, ResponseKind, resolve_path};
use crate::error::{InputError, Result};
use crate::io::{
    Row, copy_file, field, filename, read_csv, read_json, read_text, require_columns, sha256_file,
    write_csv, write_json, write_text,
};

const BUNDLE_SCHEMA_VERSION: u32 = 1;
const LABEL_ZERO_THRESHOLD: f64 = 0.0;
const LABEL_ZERO_COLUMN: &str = "long_peptide_label_thr0";
const LABEL_NOISE_CEILING_COLUMN: &str = "long_peptide_label_noise_ceiling";

#[derive(Clone, Debug)]
struct DatasetBuild {
    audit: Value,
    summary: Value,
}

pub fn normalize_hla(value: &str) -> String {
    let upper = value.trim().to_uppercase();
    let without_prefix = upper.strip_prefix("HLA-").unwrap_or(&upper);
    without_prefix.replace(['*', ':'], "")
}

fn parse_u32(value: &str, label: &str, path: &Path) -> Result<u32> {
    value.parse::<u32>().map_err(|_| {
        InputError::contract(format!("invalid {label} {value:?} in {}", path.display()))
    })
}

fn suggested_env_chunks(total_envs: usize, max_envs_per_chunk: usize) -> usize {
    let minimum = total_envs.div_ceil(max_envs_per_chunk);
    (minimum..=total_envs)
        .find(|chunks| total_envs % chunks == 0)
        .unwrap_or(total_envs)
}

fn append_header(headers: &mut Vec<String>, name: &str) {
    if !headers.iter().any(|header| header == name) {
        headers.push(name.to_owned());
    }
}

fn row_identity(row: &Row, headers: &[String]) -> Vec<String> {
    headers
        .iter()
        .map(|header| row.get(header).cloned().unwrap_or_default())
        .collect()
}

fn endpoint_key(row: &Row, fields: &[String]) -> Vec<String> {
    fields
        .iter()
        .map(|name| row.get(name).map_or("", String::as_str).trim().to_owned())
        .collect()
}

fn replace_toml_string(text: &str, key: &str, value: &str) -> Result<String> {
    let mut count = 0_usize;
    let mut output = String::with_capacity(text.len() + value.len());
    for original in text.split_inclusive('\n') {
        let has_newline = original.ends_with('\n');
        let line = original.strip_suffix('\n').unwrap_or(original);
        let trimmed = line.trim_start();
        let key_match = trimmed
            .strip_prefix(key)
            .is_some_and(|suffix| suffix.trim_start().starts_with('='));
        if !key_match {
            output.push_str(original);
            continue;
        }
        let equals = line
            .find('=')
            .ok_or_else(|| InputError::contract(format!("malformed TOML assignment for {key}")))?;
        let after_equals = &line[equals + 1..];
        let first_quote_rel = after_equals.find('"').ok_or_else(|| {
            InputError::contract(format!("TOML assignment {key} is not a quoted string"))
        })?;
        let first_quote = equals + 1 + first_quote_rel;
        let rest = &line[first_quote + 1..];
        let second_quote_rel = rest.find('"').ok_or_else(|| {
            InputError::contract(format!("TOML assignment {key} has no closing quote"))
        })?;
        let second_quote = first_quote + 1 + second_quote_rel;
        output.push_str(&line[..=first_quote]);
        output.push_str(value);
        output.push_str(&line[second_quote..]);
        if has_newline {
            output.push('\n');
        }
        count += 1;
    }
    if count != 1 {
        return Err(InputError::contract(format!(
            "expected exactly one {key:?} TOML assignment; found {count}"
        )));
    }
    Ok(output)
}

fn expected_count(
    dataset: &DatasetConfig,
    name: &str,
    observed: usize,
    expected: Option<usize>,
) -> Result<()> {
    if let Some(expected) = expected
        && observed != expected
    {
        return Err(InputError::contract(format!(
            "{} expected {name}={expected}, observed {observed}",
            dataset.id
        )));
    }
    Ok(())
}

fn validate_expected(
    dataset: &DatasetConfig,
    full_query_rows: usize,
    mono_environments: usize,
    primary_source_rows: usize,
    primary_mapping_rows: usize,
    primary_endpoints: usize,
) -> Result<()> {
    let expected = dataset.expected.as_ref().cloned().unwrap_or_default();
    expected_count(
        dataset,
        "full_query_rows",
        full_query_rows,
        expected.full_query_rows,
    )?;
    expected_count(
        dataset,
        "mono_environments",
        mono_environments,
        expected.mono_environments,
    )?;
    expected_count(
        dataset,
        "primary_source_rows",
        primary_source_rows,
        expected.primary_source_rows,
    )?;
    expected_count(
        dataset,
        "primary_mapping_rows",
        primary_mapping_rows,
        expected.primary_mapping_rows,
    )?;
    expected_count(
        dataset,
        "primary_endpoints",
        primary_endpoints,
        expected.primary_endpoints,
    )
}

fn source_record(path: &Path) -> Result<Value> {
    Ok(json!({
        "path": path.to_string_lossy(),
        "sha256": sha256_file(path)?,
        "bytes": fs::metadata(path)
            .map_err(|source| InputError::Io { path: path.to_path_buf(), source })?
            .len(),
    }))
}

fn verify_expected_hash(path: &Path, expected: Option<&str>, role: &str) -> Result<()> {
    if let Some(expected) = expected {
        let observed = sha256_file(path)?;
        if observed != expected {
            return Err(InputError::contract(format!(
                "{role} SHA-256 mismatch for {}: expected {expected}, observed {observed}",
                path.display()
            )));
        }
    }
    Ok(())
}

type ComputeKey = (String, String, u32, String);

#[derive(Clone, Debug)]
struct PreparedView {
    headers: Vec<String>,
    source_rows: usize,
    exact_duplicate_source_rows: usize,
    exact_duplicate_candidate_rows: usize,
    endpoints: BTreeSet<Vec<String>>,
    endpoint_nmer_keys: BTreeSet<Vec<String>>,
    compute_keys: BTreeSet<ComputeKey>,
    rows: Vec<Row>,
    endpoints_positive_zero: usize,
    endpoints_positive_noise: Option<usize>,
    materialized_label_columns: Vec<String>,
}

fn endpoint_nmer_key(row: &Row, fields: &[String]) -> Vec<String> {
    let mut key = endpoint_key(row, fields);
    key.push(
        row.get("nmer")
            .map_or("", String::as_str)
            .trim()
            .to_uppercase(),
    );
    key
}

fn endpoint_candidate_key(row: &Row, fields: &[String]) -> Vec<String> {
    let mut key = endpoint_nmer_key(row, fields);
    key.push(normalize_hla(row.get("HLA-RE").map_or("", String::as_str)));
    key
}

fn compute_key(row: &Row, expression: &str, path: &Path) -> Result<ComputeKey> {
    Ok((
        field(row, "nmer", path)?.trim().to_uppercase(),
        normalize_hla(field(row, "HLA-RE", path)?),
        parse_u32(field(row, "full_env_id", path)?, "full_env_id", path)?,
        expression.to_owned(),
    ))
}

fn check_expected_view(
    dataset: &DatasetConfig,
    view_id: &str,
    expected: Option<&ExpectedViewCounts>,
    prepared: &PreparedView,
) -> Result<()> {
    let expected = expected.cloned().unwrap_or_default();
    for (name, observed, wanted) in [
        ("source_rows", prepared.source_rows, expected.source_rows),
        ("mapping_rows", prepared.rows.len(), expected.mapping_rows),
        ("endpoints", prepared.endpoints.len(), expected.endpoints),
        (
            "compute_keys",
            prepared.compute_keys.len(),
            expected.compute_keys,
        ),
    ] {
        if let Some(wanted) = wanted
            && observed != wanted
        {
            return Err(InputError::contract(format!(
                "{} view {view_id} expected {name}={wanted}, observed {observed}",
                dataset.id
            )));
        }
    }
    Ok(())
}

fn prepare_view(
    spec: &DatasetConfig,
    mapping_path: &Path,
    view_id: &str,
    view_role: &str,
    full_envs: &BTreeMap<u32, BTreeSet<String>>,
) -> Result<PreparedView> {
    let table = read_csv(mapping_path)?;
    require_columns(
        &table,
        &["patient_id", "env_id", "long_peptide", "nmer"],
        mapping_path,
    )?;
    require_columns(
        &table,
        &spec
            .endpoint_fields
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        mapping_path,
    )?;
    require_columns(&table, &[&spec.response_column], mapping_path)?;

    let source_unique: BTreeSet<Vec<String>> = table
        .rows
        .iter()
        .map(|row| row_identity(row, &table.headers))
        .collect();
    let exact_duplicate_source_rows = table.rows.len() - source_unique.len();
    let mut headers = table.headers.clone();
    for name in [
        "view_id",
        "view_role",
        "full_env_id",
        "HLA-RE",
        "mapping_status",
        "compute_key_id",
    ] {
        append_header(&mut headers, name);
    }

    let mut candidates: BTreeMap<Vec<String>, Row> = BTreeMap::new();
    let mut endpoint_nmer_keys = BTreeSet::new();
    for source in &table.rows {
        let mut normalized = source.clone();
        let patient = field(&normalized, "patient_id", mapping_path)?
            .trim()
            .to_owned();
        let long_peptide = field(&normalized, "long_peptide", mapping_path)?
            .trim()
            .to_uppercase();
        let nmer = field(&normalized, "nmer", mapping_path)?
            .trim()
            .to_uppercase();
        if patient.is_empty() || long_peptide.is_empty() || nmer.is_empty() {
            return Err(InputError::contract(format!(
                "blank endpoint candidate field in {}",
                mapping_path.display()
            )));
        }
        if !long_peptide.contains(&nmer) {
            return Err(InputError::contract(format!(
                "{view_id} nmer {nmer:?} is not contained in long peptide {long_peptide:?}"
            )));
        }
        let full_env_id = parse_u32(
            field(&normalized, "env_id", mapping_path)?,
            "env_id",
            mapping_path,
        )?;
        let alleles = full_envs.get(&full_env_id).ok_or_else(|| {
            InputError::contract(format!(
                "{view_id} references absent full env_id={full_env_id}"
            ))
        })?;
        normalized.insert("patient_id".to_owned(), patient);
        normalized.insert("long_peptide".to_owned(), long_peptide);
        normalized.insert("nmer".to_owned(), nmer);
        normalized.insert("view_id".to_owned(), view_id.to_owned());
        normalized.insert("view_role".to_owned(), view_role.to_owned());
        normalized.insert("full_env_id".to_owned(), full_env_id.to_string());
        normalized.insert("mapping_status".to_owned(), "scoreable".to_owned());
        normalized.insert("compute_key_id".to_owned(), String::new());
        endpoint_nmer_keys.insert(endpoint_nmer_key(&normalized, &spec.endpoint_fields));

        for hla in alleles {
            let mut candidate = normalized.clone();
            candidate.insert("HLA-RE".to_owned(), hla.clone());
            let key = endpoint_candidate_key(&candidate, &spec.endpoint_fields);
            if let Some(previous) = candidates.get(&key) {
                if field(previous, &spec.response_column, mapping_path)?
                    != field(&candidate, &spec.response_column, mapping_path)?
                    || field(previous, "full_env_id", mapping_path)?
                        != field(&candidate, "full_env_id", mapping_path)?
                {
                    return Err(InputError::contract(format!(
                        "conflicting rows share endpoint candidate identity {:?} in {}",
                        key,
                        mapping_path.display()
                    )));
                }
                continue;
            }
            candidates.insert(key, candidate);
        }
    }

    let exact_duplicate_candidate_rows = table
        .rows
        .iter()
        .map(|row| {
            let env_id = parse_u32(field(row, "env_id", mapping_path)?, "env_id", mapping_path)?;
            Ok(full_envs.get(&env_id).map_or(0, BTreeSet::len))
        })
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .sum::<usize>()
        .saturating_sub(candidates.len());
    let mut rows: Vec<Row> = candidates.into_values().collect();
    let endpoints: BTreeSet<Vec<String>> = rows
        .iter()
        .map(|row| endpoint_key(row, &spec.endpoint_fields))
        .collect();

    let mut responses: BTreeMap<Vec<String>, BTreeSet<String>> = BTreeMap::new();
    for row in &rows {
        responses
            .entry(endpoint_key(row, &spec.endpoint_fields))
            .or_default()
            .insert(
                field(row, &spec.response_column, mapping_path)?
                    .trim()
                    .to_owned(),
            );
    }
    if let Some((endpoint, values)) = responses.iter().find(|(_, values)| values.len() != 1) {
        return Err(InputError::contract(format!(
            "{view_id} endpoint {:?} has conflicting response values {:?}",
            endpoint, values
        )));
    }
    let parse_response = |raw: &str| -> Result<f64> {
        let value = raw
            .parse::<f64>()
            .map_err(|_| InputError::contract(format!("invalid response {raw:?} in {view_id}")))?;
        if !value.is_finite() {
            return Err(InputError::contract(format!(
                "non-finite response {raw:?} in {view_id}"
            )));
        }
        if spec.response_kind == ResponseKind::Binary && !matches!(value, 0.0 | 1.0) {
            return Err(InputError::contract(format!(
                "binary response must be 0 or 1, got {raw:?} in {view_id}"
            )));
        }
        Ok(value)
    };
    let endpoints_positive_zero = responses
        .values()
        .filter_map(|values| values.iter().next())
        .map(|raw| parse_response(raw))
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .filter(|value| *value > LABEL_ZERO_THRESHOLD)
        .count();

    let mut materialized_label_columns = Vec::new();
    let endpoints_positive_noise = if spec.response_kind == ResponseKind::Continuous {
        if let Some(threshold) = spec.noise_ceiling_threshold {
            materialized_label_columns.extend([
                LABEL_ZERO_COLUMN.to_owned(),
                LABEL_NOISE_CEILING_COLUMN.to_owned(),
            ]);
            append_header(&mut headers, LABEL_ZERO_COLUMN);
            append_header(&mut headers, LABEL_NOISE_CEILING_COLUMN);
            for row in &mut rows {
                let value = parse_response(field(row, &spec.response_column, mapping_path)?)?;
                row.insert(
                    LABEL_ZERO_COLUMN.to_owned(),
                    u8::from(value > LABEL_ZERO_THRESHOLD).to_string(),
                );
                row.insert(
                    LABEL_NOISE_CEILING_COLUMN.to_owned(),
                    u8::from(value > threshold).to_string(),
                );
            }
            Some(
                responses
                    .values()
                    .filter_map(|values| values.iter().next())
                    .map(|raw| parse_response(raw))
                    .collect::<Result<Vec<_>>>()?
                    .into_iter()
                    .filter(|value| *value > threshold)
                    .count(),
            )
        } else {
            None
        }
    } else {
        None
    };

    let compute_keys = rows
        .iter()
        .map(|row| compute_key(row, &spec.expression, mapping_path))
        .collect::<Result<BTreeSet<_>>>()?;
    Ok(PreparedView {
        headers,
        source_rows: table.rows.len(),
        exact_duplicate_source_rows,
        exact_duplicate_candidate_rows,
        endpoints,
        endpoint_nmer_keys,
        compute_keys,
        rows,
        endpoints_positive_zero,
        endpoints_positive_noise,
        materialized_label_columns,
    })
}

fn representation_rows(
    rows: &[Row],
    mapping_path: &Path,
    pair_to_mono: &BTreeMap<(u32, String), usize>,
    compute_ids: &BTreeMap<ComputeKey, String>,
    expression: &str,
) -> Result<(Vec<Row>, Vec<Row>, Vec<Row>)> {
    let mut mono = Vec::with_capacity(rows.len());
    let mut full = Vec::with_capacity(rows.len());
    let mut canonical = Vec::with_capacity(rows.len());
    for source in rows {
        let key = compute_key(source, expression, mapping_path)?;
        let compute_id = compute_ids.get(&key).ok_or_else(|| {
            InputError::contract(format!(
                "mapping compute key is outside primary roster: {key:?}"
            ))
        })?;
        let mono_id = pair_to_mono
            .get(&(key.2, key.1.clone()))
            .ok_or_else(|| InputError::contract("mapping HLA lacks mono environment"))?;
        let mut full_row = source.clone();
        full_row.insert("env_id".to_owned(), key.2.to_string());
        full_row.insert("compute_key_id".to_owned(), compute_id.clone());
        let mut mono_row = full_row.clone();
        mono_row.insert("env_id".to_owned(), mono_id.to_string());
        let mut canonical_row = mono_row.clone();
        canonical_row.remove("env_id");
        canonical_row.insert("mono_env_id".to_owned(), mono_id.to_string());
        full.push(full_row);
        mono.push(mono_row);
        canonical.push(canonical_row);
    }
    Ok((mono, full, canonical))
}

fn build_dataset(
    spec: &DatasetConfig,
    input_root: &Path,
    output_dir: &Path,
) -> Result<DatasetBuild> {
    let env_path = resolve_path(input_root, &spec.env_dict);
    let primary_path = resolve_path(input_root, &spec.primary_mapping);
    let template_path = resolve_path(input_root, &spec.config_template);
    for path in [&env_path, &primary_path, &template_path] {
        if !path.is_file() {
            return Err(InputError::contract(format!(
                "required input is not a file: {}",
                path.display()
            )));
        }
    }
    if let Some(expected) = &spec.expected_source_sha256 {
        verify_expected_hash(
            &env_path,
            Some(&expected.env_dict),
            "environment dictionary",
        )?;
        verify_expected_hash(
            &primary_path,
            Some(&expected.primary_mapping),
            "primary mapping",
        )?;
        verify_expected_hash(
            &template_path,
            Some(&expected.config_template),
            "configuration template",
        )?;
    }

    let env_table = read_csv(&env_path)?;
    require_columns(&env_table, &["env_id", "allele_environment"], &env_path)?;
    let mut full_envs: BTreeMap<u32, BTreeSet<String>> = BTreeMap::new();
    let mut full_env_text: BTreeMap<u32, String> = BTreeMap::new();
    for row in &env_table.rows {
        let env_id = parse_u32(field(row, "env_id", &env_path)?, "env_id", &env_path)?;
        let alleles: BTreeSet<String> = field(row, "allele_environment", &env_path)?
            .split(',')
            .filter(|value| !value.trim().is_empty())
            .map(normalize_hla)
            .collect();
        if alleles.is_empty() || alleles.iter().any(String::is_empty) {
            return Err(InputError::contract(format!(
                "empty or invalid full environment {env_id} in {}",
                env_path.display()
            )));
        }
        if full_envs.insert(env_id, alleles.clone()).is_some() {
            return Err(InputError::contract(format!(
                "duplicate full env_id={env_id} in {}",
                env_path.display()
            )));
        }
        full_env_text.insert(env_id, alleles.into_iter().collect::<Vec<_>>().join(","));
    }

    let primary = prepare_view(
        spec,
        &primary_path,
        &spec.primary_view_id,
        "primary",
        &full_envs,
    )?;
    let compute_ids: BTreeMap<ComputeKey, String> = primary
        .compute_keys
        .iter()
        .cloned()
        .enumerate()
        .map(|(index, key)| (key, format!("{}:{index:08}", spec.prefix)))
        .collect();
    let env_hla_pairs: Vec<(u32, String)> = primary
        .compute_keys
        .iter()
        .map(|key| (key.2, key.1.clone()))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let pair_to_mono: BTreeMap<(u32, String), usize> = env_hla_pairs
        .iter()
        .cloned()
        .enumerate()
        .map(|(mono_id, pair)| (pair, mono_id))
        .collect();

    let query_headers = vec![
        "peptide".to_owned(),
        "HLA-RE".to_owned(),
        "SetNeoepitopeSampleID".to_owned(),
        "PatientID".to_owned(),
        "TCGA_EXPR_TYPE".to_owned(),
        "env_id".to_owned(),
        "gene".to_owned(),
        "count".to_owned(),
        "compute_key_id".to_owned(),
    ];
    let mut representative: BTreeMap<ComputeKey, &Row> = BTreeMap::new();
    for row in &primary.rows {
        representative
            .entry(compute_key(row, &spec.expression, &primary_path)?)
            .or_insert(row);
    }
    let mut full_query = Vec::with_capacity(primary.compute_keys.len());
    let mut mono_query = Vec::with_capacity(primary.compute_keys.len());
    let mut observation_crosswalk = Vec::with_capacity(primary.compute_keys.len());
    for key in &primary.compute_keys {
        let source = representative
            .get(key)
            .ok_or_else(|| InputError::contract("missing representative query metadata"))?;
        let compute_id = compute_ids
            .get(key)
            .ok_or_else(|| InputError::contract("missing compute_key_id"))?;
        let mono_id = pair_to_mono
            .get(&(key.2, key.1.clone()))
            .ok_or_else(|| InputError::contract("missing mono environment"))?;
        let full_row = Row::from([
            ("peptide".to_owned(), key.0.clone()),
            ("HLA-RE".to_owned(), key.1.clone()),
            ("SetNeoepitopeSampleID".to_owned(), String::new()),
            (
                "PatientID".to_owned(),
                field(source, "patient_id", &primary_path)?.to_owned(),
            ),
            ("TCGA_EXPR_TYPE".to_owned(), key.3.clone()),
            ("env_id".to_owned(), key.2.to_string()),
            (
                "gene".to_owned(),
                source.get("gene").cloned().unwrap_or_default(),
            ),
            ("count".to_owned(), "1".to_owned()),
            ("compute_key_id".to_owned(), compute_id.clone()),
        ]);
        let mut mono_row = full_row.clone();
        mono_row.insert("env_id".to_owned(), mono_id.to_string());
        full_query.push(full_row);
        mono_query.push(mono_row);
        observation_crosswalk.push(Row::from([
            ("dataset".to_owned(), spec.id.clone()),
            ("compute_key_id".to_owned(), compute_id.clone()),
            ("peptide".to_owned(), key.0.clone()),
            ("HLA-RE".to_owned(), key.1.clone()),
            ("TCGA_EXPR_TYPE".to_owned(), key.3.clone()),
            ("full_env_id".to_owned(), key.2.to_string()),
            ("mono_env_id".to_owned(), mono_id.to_string()),
        ]));
    }
    let (primary_mono, primary_full, primary_canonical) = representation_rows(
        &primary.rows,
        &primary_path,
        &pair_to_mono,
        &compute_ids,
        &spec.expression,
    )?;
    let canonical_headers: Vec<String> = primary
        .headers
        .iter()
        .map(|header| {
            if header == "env_id" {
                "mono_env_id".to_owned()
            } else {
                header.clone()
            }
        })
        .collect();

    validate_expected(
        spec,
        full_query.len(),
        env_hla_pairs.len(),
        primary.source_rows,
        primary.rows.len(),
        primary.endpoints.len(),
    )?;

    let mono_query_name = format!("{}_mono_query.csv", spec.prefix);
    let full_query_name = format!("{}_full_deduplicated_query.csv", spec.prefix);
    let full_env_name = format!("{}_full_hla_env_dict.csv", spec.prefix);
    let mono_env_name = format!("{}_mono_hla_env_dict.csv", spec.prefix);
    let env_crosswalk_name = format!("{}_full_to_mono_env_crosswalk.csv", spec.prefix);
    let observation_crosswalk_name =
        format!("{}_full_to_mono_observation_crosswalk.csv", spec.prefix);
    let mono_mapping_name = format!("{}_mono_longpep_mapping.csv", spec.prefix);
    let full_mapping_name = format!("{}_full_longpep_mapping.csv", spec.prefix);
    let canonical_mapping_name = format!("{}_candidate_roster.csv", spec.prefix);
    let audit_name = format!("{}_inputs_audit.json", spec.prefix);

    write_csv(
        &output_dir.join(&mono_query_name),
        &query_headers,
        &mono_query,
    )?;
    write_csv(
        &output_dir.join(&full_query_name),
        &query_headers,
        &full_query,
    )?;
    let full_env_rows: Vec<Row> = full_env_text
        .iter()
        .map(|(env_id, alleles)| {
            Row::from([
                ("env_id".to_owned(), env_id.to_string()),
                ("allele_environment".to_owned(), alleles.clone()),
            ])
        })
        .collect();
    write_csv(
        &output_dir.join(&full_env_name),
        &["env_id".to_owned(), "allele_environment".to_owned()],
        &full_env_rows,
    )?;
    let mono_env_rows: Vec<Row> = env_hla_pairs
        .iter()
        .enumerate()
        .map(|(mono_id, (_, hla))| {
            Row::from([
                ("env_id".to_owned(), mono_id.to_string()),
                ("allele_environment".to_owned(), hla.clone()),
            ])
        })
        .collect();
    write_csv(
        &output_dir.join(&mono_env_name),
        &["env_id".to_owned(), "allele_environment".to_owned()],
        &mono_env_rows,
    )?;
    let env_crosswalk_rows: Vec<Row> = env_hla_pairs
        .iter()
        .enumerate()
        .map(|(mono_id, (full_env_id, hla))| {
            Row::from([
                ("dataset".to_owned(), spec.id.clone()),
                ("mono_env_id".to_owned(), mono_id.to_string()),
                ("full_env_id".to_owned(), full_env_id.to_string()),
                ("HLA-RE".to_owned(), hla.clone()),
                (
                    "full_allele_environment".to_owned(),
                    full_env_text.get(full_env_id).cloned().unwrap_or_default(),
                ),
            ])
        })
        .collect();
    write_csv(
        &output_dir.join(&env_crosswalk_name),
        &[
            "dataset".to_owned(),
            "mono_env_id".to_owned(),
            "full_env_id".to_owned(),
            "HLA-RE".to_owned(),
            "full_allele_environment".to_owned(),
        ],
        &env_crosswalk_rows,
    )?;
    write_csv(
        &output_dir.join(&observation_crosswalk_name),
        &[
            "dataset".to_owned(),
            "compute_key_id".to_owned(),
            "peptide".to_owned(),
            "HLA-RE".to_owned(),
            "TCGA_EXPR_TYPE".to_owned(),
            "full_env_id".to_owned(),
            "mono_env_id".to_owned(),
        ],
        &observation_crosswalk,
    )?;
    write_csv(
        &output_dir.join(&mono_mapping_name),
        &primary.headers,
        &primary_mono,
    )?;
    write_csv(
        &output_dir.join(&full_mapping_name),
        &primary.headers,
        &primary_full,
    )?;
    write_csv(
        &output_dir.join(&canonical_mapping_name),
        &canonical_headers,
        &primary_canonical,
    )?;

    let template = read_text(&template_path)?;
    let root = "${EXTERNAL_VALIDATION_INPUT_ROOT}";
    let mono_config = replace_toml_string(
        &replace_toml_string(
            &template,
            "hla_env_dict",
            &format!("{root}/{mono_env_name}"),
        )?,
        "query_peptide_input_tuples_file",
        &format!("{root}/{mono_query_name}"),
    )?;
    write_text(
        &output_dir.join(&spec.generated_mono_config),
        &format!(
            "# Generated complete-F mono query; source cohort directories remain read-only.\n{mono_config}"
        ),
    )?;
    let full_config = replace_toml_string(
        &replace_toml_string(
            &template,
            "hla_env_dict",
            &format!("{root}/{full_env_name}"),
        )?,
        "query_peptide_input_tuples_file",
        &format!("{root}/{full_query_name}"),
    )?;
    write_text(
        &output_dir.join(&spec.generated_full_deduplicated_config),
        &format!("# Generated complete-F full query; no presentation filtering.\n{full_config}"),
    )?;

    let mut view_audits = Map::new();
    let mut view_output_names = Vec::new();
    for view in &spec.views {
        let view_path = resolve_path(input_root, &view.mapping);
        if !view_path.is_file() {
            return Err(InputError::contract(format!(
                "secondary view mapping is not a file: {}",
                view_path.display()
            )));
        }
        verify_expected_hash(
            &view_path,
            view.expected_sha256.as_deref(),
            &format!("secondary view {}", view.id),
        )?;
        let prepared = prepare_view(spec, &view_path, &view.id, "secondary", &full_envs)?;
        if view.require_subset_of != spec.primary_view_id {
            return Err(InputError::contract(format!(
                "view {} does not name primary parent {}",
                view.id, spec.primary_view_id
            )));
        }
        if !prepared
            .endpoint_nmer_keys
            .is_subset(&primary.endpoint_nmer_keys)
        {
            return Err(InputError::contract(format!(
                "view {} has endpoint+nmer rows outside primary view {}",
                view.id, spec.primary_view_id
            )));
        }
        if !prepared.compute_keys.is_subset(&primary.compute_keys) {
            return Err(InputError::contract(format!(
                "view {} has compute keys outside primary view {}",
                view.id, spec.primary_view_id
            )));
        }
        check_expected_view(spec, &view.id, view.expected.as_ref(), &prepared)?;
        let (mono, full, canonical) = representation_rows(
            &prepared.rows,
            &view_path,
            &pair_to_mono,
            &compute_ids,
            &spec.expression,
        )?;
        let token = view.id.to_lowercase().replace('-', "_");
        let redundant_prefix = format!("{}_", spec.prefix);
        let view_token = token.strip_prefix(&redundant_prefix).unwrap_or(&token);
        let mono_name = format!("{}_{}_mono_longpep_mapping.csv", spec.prefix, view_token);
        let full_name = format!("{}_{}_full_longpep_mapping.csv", spec.prefix, view_token);
        let canonical_name = format!("{}_{}_candidate_roster.csv", spec.prefix, view_token);
        let view_canonical_headers: Vec<String> = prepared
            .headers
            .iter()
            .map(|header| {
                if header == "env_id" {
                    "mono_env_id".to_owned()
                } else {
                    header.clone()
                }
            })
            .collect();
        write_csv(&output_dir.join(&mono_name), &prepared.headers, &mono)?;
        write_csv(&output_dir.join(&full_name), &prepared.headers, &full)?;
        write_csv(
            &output_dir.join(&canonical_name),
            &view_canonical_headers,
            &canonical,
        )?;
        view_output_names.extend([mono_name.clone(), full_name.clone(), canonical_name.clone()]);
        view_audits.insert(
            view.id.clone(),
            json!({
                "role": view.role,
                "parent_view_id": view.require_subset_of,
                "selection_eligible": view.selection_eligible,
                "bundle_eligible": view.bundle_eligible,
                "source": source_record(&view_path)?,
                "source_rows": prepared.source_rows,
                "exact_duplicate_source_rows": prepared.exact_duplicate_source_rows,
                "mapping_rows": prepared.rows.len(),
                "exact_duplicate_candidate_rows_removed": prepared.exact_duplicate_candidate_rows,
                "endpoints": prepared.endpoints.len(),
                "compute_keys": prepared.compute_keys.len(),
                "subset_status": "pass",
                "endpoints_positive_label_thr0": prepared.endpoints_positive_zero,
                "endpoints_positive_label_noise_ceiling": prepared.endpoints_positive_noise,
                "materialized_label_columns": prepared.materialized_label_columns,
                "output_files": {
                    "mono_mapping": mono_name,
                    "full_mapping": full_name,
                    "canonical_mapping": canonical_name,
                },
            }),
        );
    }

    let output_files = json!({
        "mono_query": mono_query_name,
        "full_deduplicated_query": full_query_name,
        "full_env_dict": full_env_name,
        "env_dict": mono_env_name,
        "env_crosswalk": env_crosswalk_name,
        "observation_crosswalk": observation_crosswalk_name,
        "canonical_mapping": canonical_mapping_name,
        "full_mapping": full_mapping_name,
        "mono_mapping": mono_mapping_name,
        "mono_config": spec.generated_mono_config,
        "full_deduplicated_config": spec.generated_full_deduplicated_config,
    });
    let audit = json!({
        "schema_version": 3,
        "generator": "external_validation_inputs",
        "dataset": spec.id,
        "expression": spec.expression,
        "primary_view_id": spec.primary_view_id,
        "source_files": {
            "env_dict": source_record(&env_path)?,
            "primary_mapping": source_record(&primary_path)?,
            "config_template": source_record(&template_path)?,
        },
        "full_environment_rows": full_envs.len(),
        "full_query_rows": full_query.len(),
        "mono_environment_rows": env_hla_pairs.len(),
        "mono_query_rows": mono_query.len(),
        "duplicate_full_compute_identities": 0,
        "duplicate_mono_compute_identities": 0,
        "candidate_roster_rows": primary.rows.len(),
        "scoreable_candidate_rows": primary.rows.len(),
        "floor_candidate_rows": 0,
        "primary_source_rows": primary.source_rows,
        "exact_duplicate_source_mapping_rows": primary.exact_duplicate_source_rows,
        "exact_duplicate_candidate_rows_removed": primary.exact_duplicate_candidate_rows,
        "endpoint_key_fields": spec.endpoint_fields,
        "mapping_endpoints": primary.endpoints.len(),
        "mapping_endpoints_with_conflicting_response_values": 0,
        "mapping_endpoints_with_conflicting_binary_labels": 0,
        "label_zero_threshold": LABEL_ZERO_THRESHOLD,
        "noise_ceiling_threshold": spec.noise_ceiling_threshold,
        "materialized_label_columns": primary.materialized_label_columns,
        "endpoints_positive_label_thr0": primary.endpoints_positive_zero,
        "endpoints_positive_label_noise_ceiling": primary.endpoints_positive_noise,
        "input_audit_status": "pass",
        "suggested_smoke_env_chunks": env_hla_pairs.len(),
        "suggested_production_env_chunks": suggested_env_chunks(env_hla_pairs.len(), 4),
        "output_files": output_files,
        "views": view_audits,
        "secondary_view_output_count": view_output_names.len(),
    });
    write_json(&output_dir.join(&audit_name), &audit)?;

    Ok(DatasetBuild {
        summary: json!({
            "candidate_rows": primary.rows.len(),
            "scoreable_rows": primary.rows.len(),
            "floor_rows": 0,
            "mono_environments": env_hla_pairs.len(),
            "query_rows": full_query.len(),
            "endpoints": primary.endpoints.len(),
            "secondary_views": spec.views.len(),
        }),
        audit,
    })
}

fn output_file_records(output_dir: &Path) -> Result<BTreeMap<String, Value>> {
    let mut records = BTreeMap::new();
    let entries = fs::read_dir(output_dir).map_err(|source| InputError::Io {
        path: output_dir.to_path_buf(),
        source,
    })?;
    for entry in entries {
        let entry = entry.map_err(|source| InputError::Io {
            path: output_dir.to_path_buf(),
            source,
        })?;
        let path = entry.path();
        if path.is_file() && path.file_name().is_some_and(|name| name != "manifest.json") {
            let name = path
                .file_name()
                .and_then(|value| value.to_str())
                .ok_or_else(|| InputError::contract("non-UTF-8 output filename"))?
                .to_owned();
            records.insert(
                name,
                json!({
                    "sha256": sha256_file(&path)?,
                    "bytes": fs::metadata(&path)
                        .map_err(|source| InputError::Io { path: path.clone(), source })?
                        .len(),
                }),
            );
        }
    }
    Ok(records)
}

fn build_inner(config_path: &Path, input_root: &Path, output_dir: &Path) -> Result<Value> {
    let config = BuildConfig::load(config_path)?;
    if !input_root.is_dir() {
        return Err(InputError::contract(format!(
            "input root is not a directory: {}",
            input_root.display()
        )));
    }
    fs::create_dir_all(output_dir).map_err(|source| InputError::Io {
        path: output_dir.to_path_buf(),
        source,
    })?;

    let mut dataset_audits = BTreeMap::new();
    let mut summaries = BTreeMap::new();
    for dataset in &config.datasets {
        let built = build_dataset(dataset, input_root, output_dir)?;
        write_json(
            &output_dir.join(format!("{}_inputs_audit.json", dataset.prefix)),
            &built.audit,
        )?;
        dataset_audits.insert(dataset.id.clone(), built.audit);
        summaries.insert(dataset.id.clone(), built.summary);
    }

    let mut provenance_records = Vec::new();
    for item in &config.provenance {
        let source = resolve_path(input_root, &item.path);
        if !source.is_file() {
            return Err(InputError::contract(format!(
                "provenance input is not a file: {}",
                source.display()
            )));
        }
        verify_expected_hash(&source, item.expected_sha256.as_deref(), &item.role)?;
        let mut record = source_record(&source)?;
        record["role"] = Value::String(item.role.clone());
        record["metadata"] = item.metadata.clone();
        if item.copy_to_output {
            let output_name = item
                .output_name
                .as_ref()
                .map(PathBuf::from)
                .map(Ok)
                .unwrap_or_else(|| filename(&source))?;
            if output_name.components().count() != 1 {
                return Err(InputError::contract(format!(
                    "provenance output_name must be a filename: {}",
                    output_name.display()
                )));
            }
            copy_file(&source, &output_dir.join(&output_name))?;
            record["copied_to"] = Value::String(output_name.to_string_lossy().into_owned());
        }
        provenance_records.push(record);
    }

    let mut passthrough_records = Vec::new();
    for item in &config.passthrough_files {
        let source = resolve_path(input_root, &item.path);
        if !source.is_file() {
            return Err(InputError::contract(format!(
                "passthrough input is not a file: {}",
                source.display()
            )));
        }
        verify_expected_hash(
            &source,
            item.expected_sha256.as_deref(),
            "passthrough input",
        )?;
        let output_name = item
            .output_name
            .as_ref()
            .map(PathBuf::from)
            .map(Ok)
            .unwrap_or_else(|| filename(&source))?;
        if output_name.components().count() != 1 {
            return Err(InputError::contract(format!(
                "passthrough output_name must be a filename: {}",
                output_name.display()
            )));
        }
        copy_file(&source, &output_dir.join(&output_name))?;
        passthrough_records.push(json!({
            "source": source_record(&source)?,
            "output": output_name.to_string_lossy(),
        }));
    }

    let output_files = output_file_records(output_dir)?;
    let manifest = json!({
        "schema_version": BUNDLE_SCHEMA_VERSION,
        "package_kind": "iris_external_validation_full_roster_inputs",
        "generator": {
            "crate": "external_validation_inputs",
            "version": env!("CARGO_PKG_VERSION"),
        },
        "configuration": {
            "path": config_path.to_string_lossy(),
            "sha256": sha256_file(config_path)?,
            "schema_version": config.schema_version,
        },
        "input_root": input_root.to_string_lossy(),
        "datasets": dataset_audits,
        "provenance": provenance_records,
        "passthrough_files": passthrough_records,
        "output_files": output_files,
    });
    write_json(&output_dir.join("manifest.json"), &manifest)?;
    validate_bundle(output_dir)?;
    Ok(json!({
        "output": output_dir.to_string_lossy(),
        "datasets": summaries,
        "manifest": "manifest.json",
    }))
}

pub fn build_bundle(config_path: &Path, input_root: &Path, output: &Path) -> Result<Value> {
    if output.exists() {
        return Err(InputError::contract(format!(
            "output already exists; refusing to overwrite immutable bundle: {}",
            output.display()
        )));
    }
    let parent = output.parent().ok_or_else(|| {
        InputError::contract(format!("output has no parent: {}", output.display()))
    })?;
    if !parent.is_dir() {
        return Err(InputError::contract(format!(
            "output parent is not a directory: {}",
            parent.display()
        )));
    }
    let stem = output
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| InputError::contract("output filename is not valid UTF-8"))?;
    let stage = parent.join(format!(".{stem}.stage-{}", std::process::id()));
    if stage.exists() {
        return Err(InputError::contract(format!(
            "staging path already exists: {}",
            stage.display()
        )));
    }
    let result = build_inner(config_path, input_root, &stage);
    match result {
        Ok(mut summary) => {
            fs::rename(&stage, output).map_err(|source| InputError::Io {
                path: output.to_path_buf(),
                source,
            })?;
            summary["output"] = Value::String(output.to_string_lossy().into_owned());
            Ok(summary)
        }
        Err(error) => {
            if stage.exists() {
                let _ = fs::remove_dir_all(&stage);
            }
            Err(error)
        }
    }
}

fn validate_mapping_family(
    bundle: &Path,
    dataset: &str,
    view_id: &str,
    output_files: &serde_json::Map<String, Value>,
    compute_ids: &BTreeSet<String>,
) -> Result<usize> {
    let filename_for = |role: &str| -> Result<&str> {
        output_files[role].as_str().ok_or_else(|| {
            InputError::contract(format!("dataset {dataset} view {view_id} lacks {role}"))
        })
    };
    let mono_name = filename_for("mono_mapping")?;
    let full_name = filename_for("full_mapping")?;
    let canonical_name = filename_for("canonical_mapping")?;
    let mono = read_csv(&bundle.join(mono_name))?;
    let full = read_csv(&bundle.join(full_name))?;
    let canonical = read_csv(&bundle.join(canonical_name))?;
    require_columns(
        &mono,
        &[
            "HLA-RE",
            "mapping_status",
            "full_env_id",
            "env_id",
            "compute_key_id",
            "view_id",
            "view_role",
        ],
        &bundle.join(mono_name),
    )?;
    if mono.headers != full.headers
        || mono.rows.len() != full.rows.len()
        || mono.rows.len() != canonical.rows.len()
    {
        return Err(InputError::contract(format!(
            "dataset {dataset} view {view_id} canonical/full/mono mapping shape mismatch"
        )));
    }
    let expected_canonical_headers: Vec<String> = mono
        .headers
        .iter()
        .map(|header| {
            if header == "env_id" {
                "mono_env_id".to_owned()
            } else {
                header.clone()
            }
        })
        .collect();
    if canonical.headers != expected_canonical_headers {
        return Err(InputError::contract(format!(
            "dataset {dataset} view {view_id} canonical mapping header mismatch"
        )));
    }
    let unique_rows: BTreeSet<Vec<String>> = full
        .rows
        .iter()
        .map(|row| row_identity(row, &full.headers))
        .collect();
    if unique_rows.len() != full.rows.len() {
        return Err(InputError::contract(format!(
            "dataset {dataset} view {view_id} contains duplicate mapping rows"
        )));
    }
    for (idx, (mono_row, full_row)) in mono.rows.iter().zip(&full.rows).enumerate() {
        let hla = mono_row.get("HLA-RE").map_or("", String::as_str);
        let status = mono_row.get("mapping_status").map_or("", String::as_str);
        if hla.is_empty() || status != "scoreable" {
            return Err(InputError::contract(format!(
                "dataset {dataset} view {view_id} invalid complete-F row {idx}: HLA={hla:?}, status={status:?}"
            )));
        }
        let compute_id = mono_row.get("compute_key_id").map_or("", String::as_str);
        if !compute_ids.contains(compute_id) {
            return Err(InputError::contract(format!(
                "dataset {dataset} view {view_id} row {idx} references unknown compute_key_id {compute_id:?}"
            )));
        }
        if mono_row.get("view_id").map_or("", String::as_str) != view_id {
            return Err(InputError::contract(format!(
                "dataset {dataset} view {view_id} row {idx} carries a different view_id"
            )));
        }
        for header in &mono.headers {
            if header != "env_id" && mono_row.get(header) != full_row.get(header) {
                return Err(InputError::contract(format!(
                    "dataset {dataset} view {view_id} full/mono mapping differs in {header} at row {idx}"
                )));
            }
        }
        if full_row.get("env_id") != full_row.get("full_env_id") {
            return Err(InputError::contract(format!(
                "dataset {dataset} view {view_id} full mapping env_id != full_env_id at row {idx}"
            )));
        }
        let canonical_row = &canonical.rows[idx];
        for header in &canonical.headers {
            let mono_header = if header == "mono_env_id" {
                "env_id"
            } else {
                header.as_str()
            };
            if canonical_row.get(header) != mono_row.get(mono_header) {
                return Err(InputError::contract(format!(
                    "dataset {dataset} view {view_id} canonical/mono mapping differs in {header} at row {idx}"
                )));
            }
        }
    }
    Ok(mono.rows.len())
}

pub fn validate_bundle(bundle: &Path) -> Result<Value> {
    if !bundle.is_dir() {
        return Err(InputError::contract(format!(
            "bundle is not a directory: {}",
            bundle.display()
        )));
    }
    let manifest_path = bundle.join("manifest.json");
    let manifest: Value = read_json(&manifest_path)?;
    if manifest["schema_version"].as_u64() != Some(BUNDLE_SCHEMA_VERSION as u64) {
        return Err(InputError::contract(format!(
            "unsupported bundle manifest schema in {}",
            manifest_path.display()
        )));
    }
    if manifest["package_kind"].as_str() != Some("iris_external_validation_full_roster_inputs") {
        return Err(InputError::contract(format!(
            "unexpected package_kind in {}",
            manifest_path.display()
        )));
    }
    let files = manifest["output_files"]
        .as_object()
        .ok_or_else(|| InputError::contract("manifest output_files is not an object"))?;
    for (name, record) in files {
        if Path::new(name).components().count() != 1 {
            return Err(InputError::contract(format!(
                "manifest output filename is not flat: {name}"
            )));
        }
        let path = bundle.join(name);
        if !path.is_file() {
            return Err(InputError::contract(format!(
                "manifest output is missing: {}",
                path.display()
            )));
        }
        let expected_hash = record["sha256"]
            .as_str()
            .ok_or_else(|| InputError::contract(format!("manifest output {name} lacks sha256")))?;
        let observed_hash = sha256_file(&path)?;
        if observed_hash != expected_hash {
            return Err(InputError::contract(format!(
                "output hash mismatch for {name}: expected {expected_hash}, observed {observed_hash}"
            )));
        }
    }

    let datasets = manifest["datasets"]
        .as_object()
        .ok_or_else(|| InputError::contract("manifest datasets is not an object"))?;
    for (dataset, audit) in datasets {
        let output_files = audit["output_files"]
            .as_object()
            .ok_or_else(|| InputError::contract(format!("dataset {dataset} lacks output_files")))?;
        let query_name = output_files["full_deduplicated_query"]
            .as_str()
            .ok_or_else(|| {
                InputError::contract(format!("dataset {dataset} lacks full_deduplicated_query"))
            })?;
        let query = read_csv(&bundle.join(query_name))?;
        require_columns(
            &query,
            &[
                "peptide",
                "HLA-RE",
                "TCGA_EXPR_TYPE",
                "env_id",
                "compute_key_id",
            ],
            &bundle.join(query_name),
        )?;
        let compute_ids: BTreeSet<String> = query
            .rows
            .iter()
            .map(|row| row.get("compute_key_id").cloned().unwrap_or_default())
            .collect();
        let compute_keys: BTreeSet<(String, String, String, String)> = query
            .rows
            .iter()
            .map(|row| {
                (
                    row.get("peptide").cloned().unwrap_or_default(),
                    row.get("HLA-RE").cloned().unwrap_or_default(),
                    row.get("env_id").cloned().unwrap_or_default(),
                    row.get("TCGA_EXPR_TYPE").cloned().unwrap_or_default(),
                )
            })
            .collect();
        if compute_ids.len() != query.rows.len()
            || compute_ids.contains("")
            || compute_keys.len() != query.rows.len()
        {
            return Err(InputError::contract(format!(
                "dataset {dataset} full query has duplicate or blank compute identities"
            )));
        }
        if audit["full_query_rows"].as_u64() != Some(query.rows.len() as u64) {
            return Err(InputError::contract(format!(
                "dataset {dataset} audit full_query_rows differs from query contents"
            )));
        }
        let primary_view_id = audit["primary_view_id"].as_str().ok_or_else(|| {
            InputError::contract(format!("dataset {dataset} lacks primary_view_id"))
        })?;
        let primary_rows =
            validate_mapping_family(bundle, dataset, primary_view_id, output_files, &compute_ids)?;
        if audit["candidate_roster_rows"].as_u64() != Some(primary_rows as u64)
            || audit["scoreable_candidate_rows"].as_u64() != Some(primary_rows as u64)
            || audit["floor_candidate_rows"].as_u64() != Some(0)
        {
            return Err(InputError::contract(format!(
                "dataset {dataset} audit counts differ from mapping contents"
            )));
        }
        let views = audit["views"].as_object().ok_or_else(|| {
            InputError::contract(format!("dataset {dataset} views is not an object"))
        })?;
        for (view_id, view_audit) in views {
            let view_files = view_audit["output_files"].as_object().ok_or_else(|| {
                InputError::contract(format!(
                    "dataset {dataset} view {view_id} lacks output_files"
                ))
            })?;
            let observed =
                validate_mapping_family(bundle, dataset, view_id, view_files, &compute_ids)?;
            if view_audit["mapping_rows"].as_u64() != Some(observed as u64)
                || view_audit["subset_status"].as_str() != Some("pass")
                || view_audit["selection_eligible"].as_bool() != Some(false)
                || view_audit["bundle_eligible"].as_bool() != Some(false)
            {
                return Err(InputError::contract(format!(
                    "dataset {dataset} view {view_id} audit contract mismatch"
                )));
            }
        }
    }
    Ok(json!({
        "status": "pass",
        "bundle": bundle.to_string_lossy(),
        "datasets": datasets.len(),
        "files": files.len(),
        "manifest_sha256": sha256_file(&manifest_path)?,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hla_normalization_matches_authoritative_builder() {
        assert_eq!(normalize_hla(" HLA-A*02:01 "), "A0201");
        assert_eq!(normalize_hla("b0702"), "B0702");
    }

    #[test]
    fn suggested_chunks_are_divisors() {
        assert_eq!(suggested_env_chunks(93, 4), 31);
        assert_eq!(suggested_env_chunks(71, 4), 71);
        assert_eq!(suggested_env_chunks(67, 4), 67);
    }

    #[test]
    fn toml_replacement_preserves_comment() {
        let input = "x = 1\nhla_env_dict = \"old\" # retained\ny = 2\n";
        let output = replace_toml_string(input, "hla_env_dict", "${ROOT}/new.csv").unwrap();
        assert_eq!(
            output,
            "x = 1\nhla_env_dict = \"${ROOT}/new.csv\" # retained\ny = 2\n"
        );
    }
}
