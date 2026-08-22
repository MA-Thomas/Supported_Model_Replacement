use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

use crate::config::{BuildConfig, DatasetConfig, ResponseKind, resolve_path};
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

fn biological_key(row: &Row, path: &Path) -> Result<Vec<String>> {
    Ok(vec![
        field(row, "PatientID", path)?.trim().to_owned(),
        field(row, "peptide", path)?.trim().to_uppercase(),
        normalize_hla(field(row, "HLA-RE", path)?),
        field(row, "TCGA_EXPR_TYPE", path)?.trim().to_owned(),
        row.get("gene").map_or("", String::as_str).trim().to_owned(),
        row.get("SetNeoepitopeSampleID")
            .map_or("", String::as_str)
            .trim()
            .to_owned(),
    ])
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
    unique_query_rows: usize,
    mono_environments: usize,
    mapping_rows: usize,
    scoreable_rows: usize,
    floor_rows: usize,
) -> Result<()> {
    let expected = dataset.expected.as_ref().cloned().unwrap_or_default();
    expected_count(
        dataset,
        "unique_query_rows",
        unique_query_rows,
        expected.unique_query_rows,
    )?;
    expected_count(
        dataset,
        "mono_environments",
        mono_environments,
        expected.mono_environments,
    )?;
    expected_count(dataset, "mapping_rows", mapping_rows, expected.mapping_rows)?;
    expected_count(
        dataset,
        "scoreable_rows",
        scoreable_rows,
        expected.scoreable_rows,
    )?;
    expected_count(dataset, "floor_rows", floor_rows, expected.floor_rows)
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

fn build_dataset(
    spec: &DatasetConfig,
    input_root: &Path,
    output_dir: &Path,
) -> Result<DatasetBuild> {
    let query_path = resolve_path(input_root, &spec.query);
    let env_path = resolve_path(input_root, &spec.env_dict);
    let mapping_path = resolve_path(input_root, &spec.mapping);
    let template_path = resolve_path(input_root, &spec.config_template);
    for path in [&query_path, &env_path, &mapping_path, &template_path] {
        if !path.is_file() {
            return Err(InputError::contract(format!(
                "required input is not a file: {}",
                path.display()
            )));
        }
    }
    if let Some(expected) = &spec.expected_source_sha256 {
        verify_expected_hash(&query_path, Some(&expected.query), "query")?;
        verify_expected_hash(
            &env_path,
            Some(&expected.env_dict),
            "environment dictionary",
        )?;
        verify_expected_hash(&mapping_path, Some(&expected.mapping), "source mapping")?;
        verify_expected_hash(
            &template_path,
            Some(&expected.config_template),
            "configuration template",
        )?;
    }

    let env_table = read_csv(&env_path)?;
    require_columns(&env_table, &["env_id", "allele_environment"], &env_path)?;
    let mut full_envs: BTreeMap<u32, Vec<String>> = BTreeMap::new();
    let mut full_env_text: BTreeMap<u32, String> = BTreeMap::new();
    for row in &env_table.rows {
        let env_id = parse_u32(field(row, "env_id", &env_path)?, "env_id", &env_path)?;
        if full_envs.contains_key(&env_id) {
            return Err(InputError::contract(format!(
                "duplicate full env_id={env_id} in {}",
                env_path.display()
            )));
        }
        let alleles: Vec<String> = field(row, "allele_environment", &env_path)?
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
        full_env_text.insert(env_id, alleles.join(","));
        full_envs.insert(env_id, alleles);
    }

    let query_table = read_csv(&query_path)?;
    require_columns(
        &query_table,
        &[
            "peptide",
            "HLA-RE",
            "PatientID",
            "TCGA_EXPR_TYPE",
            "env_id",
            "gene",
        ],
        &query_path,
    )?;
    let mut unique_query = Vec::new();
    let mut by_compute: BTreeMap<(String, String, u32, String), Row> = BTreeMap::new();
    let mut duplicate_compute_rows = 0_usize;
    for source_row in &query_table.rows {
        let mut row = source_row.clone();
        let peptide = field(&row, "peptide", &query_path)?.to_uppercase();
        let hla = normalize_hla(field(&row, "HLA-RE", &query_path)?);
        row.insert("peptide".to_owned(), peptide.clone());
        row.insert("HLA-RE".to_owned(), hla.clone());
        let full_env_id = parse_u32(field(&row, "env_id", &query_path)?, "env_id", &query_path)?;
        let alleles = full_envs.get(&full_env_id).ok_or_else(|| {
            InputError::contract(format!(
                "query references absent full env_id={full_env_id}: {}",
                query_path.display()
            ))
        })?;
        if !alleles.iter().any(|allele| allele == &hla) {
            return Err(InputError::contract(format!(
                "query HLA {hla} is absent from full env {full_env_id} ({})",
                full_env_text.get(&full_env_id).map_or("", String::as_str)
            )));
        }
        let compute_key = (
            peptide,
            hla,
            full_env_id,
            field(&row, "TCGA_EXPR_TYPE", &query_path)?.to_owned(),
        );
        if let Some(previous) = by_compute.get(&compute_key) {
            if previous != &row {
                return Err(InputError::contract(format!(
                    "conflicting rows share compute key {:?} in {}",
                    compute_key,
                    query_path.display()
                )));
            }
            duplicate_compute_rows += 1;
            continue;
        }
        by_compute.insert(compute_key, row.clone());
        unique_query.push(row);
    }

    let full_biological_keys: Vec<Vec<String>> = unique_query
        .iter()
        .map(|row| biological_key(row, &query_path))
        .collect::<Result<_>>()?;
    if full_biological_keys.iter().collect::<BTreeSet<_>>().len() != full_biological_keys.len() {
        return Err(InputError::contract(format!(
            "biological query identity is not unique in {}",
            query_path.display()
        )));
    }

    let env_hla_pairs: Vec<(u32, String)> = unique_query
        .iter()
        .map(|row| {
            Ok((
                parse_u32(field(row, "env_id", &query_path)?, "env_id", &query_path)?,
                field(row, "HLA-RE", &query_path)?.to_owned(),
            ))
        })
        .collect::<Result<BTreeSet<_>>>()?
        .into_iter()
        .collect();
    let pair_to_mono: BTreeMap<(u32, String), usize> = env_hla_pairs
        .iter()
        .cloned()
        .enumerate()
        .map(|(mono_id, pair)| (pair, mono_id))
        .collect();

    let mut mono_query_rows = Vec::with_capacity(unique_query.len());
    let mut observation_crosswalk_rows = Vec::with_capacity(unique_query.len());
    for row in &unique_query {
        let full_env_id = parse_u32(field(row, "env_id", &query_path)?, "env_id", &query_path)?;
        let hla = field(row, "HLA-RE", &query_path)?.to_owned();
        let mono_env_id = pair_to_mono
            .get(&(full_env_id, hla.clone()))
            .copied()
            .ok_or_else(|| InputError::contract("internal missing mono environment"))?;
        let mut mono_row = row.clone();
        mono_row.insert("env_id".to_owned(), mono_env_id.to_string());
        mono_query_rows.push(mono_row);
        observation_crosswalk_rows.push(Row::from([
            ("dataset".to_owned(), spec.id.clone()),
            (
                "PatientID".to_owned(),
                field(row, "PatientID", &query_path)?.to_owned(),
            ),
            (
                "peptide".to_owned(),
                field(row, "peptide", &query_path)?.to_owned(),
            ),
            ("HLA-RE".to_owned(), hla),
            (
                "TCGA_EXPR_TYPE".to_owned(),
                field(row, "TCGA_EXPR_TYPE", &query_path)?.to_owned(),
            ),
            (
                "gene".to_owned(),
                row.get("gene").cloned().unwrap_or_default(),
            ),
            (
                "SetNeoepitopeSampleID".to_owned(),
                row.get("SetNeoepitopeSampleID")
                    .cloned()
                    .unwrap_or_default(),
            ),
            ("full_env_id".to_owned(), full_env_id.to_string()),
            ("mono_env_id".to_owned(), mono_env_id.to_string()),
        ]));
    }

    let mono_compute_keys: BTreeSet<(String, String, u32, String)> = mono_query_rows
        .iter()
        .map(|row| {
            Ok((
                field(row, "peptide", &query_path)?.to_owned(),
                field(row, "HLA-RE", &query_path)?.to_owned(),
                parse_u32(field(row, "env_id", &query_path)?, "env_id", &query_path)?,
                field(row, "TCGA_EXPR_TYPE", &query_path)?.to_owned(),
            ))
        })
        .collect::<Result<_>>()?;
    if mono_compute_keys.len() != mono_query_rows.len() {
        return Err(InputError::contract(format!(
            "derived mono compute identity is not unique for {}",
            spec.id
        )));
    }
    let mono_biological_keys: BTreeSet<Vec<String>> = mono_query_rows
        .iter()
        .map(|row| biological_key(row, &query_path))
        .collect::<Result<_>>()?;
    let full_biological_set: BTreeSet<Vec<String>> = full_biological_keys.into_iter().collect();
    if mono_biological_keys != full_biological_set {
        return Err(InputError::contract(format!(
            "full/mono biological identity mismatch for {}",
            spec.id
        )));
    }

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

    let mapping_table = read_csv(&mapping_path)?;
    require_columns(
        &mapping_table,
        &["patient_id", "env_id", "long_peptide", "nmer"],
        &mapping_path,
    )?;
    require_columns(
        &mapping_table,
        &spec
            .endpoint_fields
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        &mapping_path,
    )?;
    require_columns(&mapping_table, &[&spec.response_column], &mapping_path)?;

    let source_unique: BTreeSet<Vec<String>> = mapping_table
        .rows
        .iter()
        .map(|row| row_identity(row, &mapping_table.headers))
        .collect();
    let exact_duplicate_source_rows = mapping_table.rows.len() - source_unique.len();

    let mut query_hlas: BTreeMap<(String, u32, String), BTreeSet<String>> = BTreeMap::new();
    for row in &unique_query {
        let key = (
            field(row, "PatientID", &query_path)?.to_owned(),
            parse_u32(field(row, "env_id", &query_path)?, "env_id", &query_path)?,
            field(row, "peptide", &query_path)?.to_owned(),
        );
        query_hlas
            .entry(key)
            .or_default()
            .insert(field(row, "HLA-RE", &query_path)?.to_owned());
    }

    let mut mapping_headers = mapping_table.headers.clone();
    append_header(&mut mapping_headers, "full_env_id");
    append_header(&mut mapping_headers, "HLA-RE");
    append_header(&mut mapping_headers, "mapping_status");
    let mut expanded_mapping = Vec::new();
    let mut matched_source_rows = 0_usize;
    let mut unmatched_source_rows = 0_usize;
    for source_row in &mapping_table.rows {
        let mut row = source_row.clone();
        let full_env_id = parse_u32(
            field(&row, "env_id", &mapping_path)?,
            "env_id",
            &mapping_path,
        )?;
        let nmer = field(&row, "nmer", &mapping_path)?.to_uppercase();
        row.insert("nmer".to_owned(), nmer.clone());
        let patient = field(&row, "patient_id", &mapping_path)?.to_owned();
        let scoreable_hlas = query_hlas
            .get(&(patient, full_env_id, nmer))
            .cloned()
            .unwrap_or_default();
        let env_alleles = full_envs.get(&full_env_id).ok_or_else(|| {
            InputError::contract(format!(
                "{} mapping references full env_id={full_env_id} absent from {}",
                spec.id,
                env_path.display()
            ))
        })?;
        let distinct_alleles: BTreeSet<String> = env_alleles.iter().cloned().collect();
        if scoreable_hlas.is_empty() {
            unmatched_source_rows += 1;
        } else {
            matched_source_rows += 1;
        }
        for hla in distinct_alleles {
            let mut mapped = row.clone();
            mapped.insert("full_env_id".to_owned(), full_env_id.to_string());
            mapped.insert("HLA-RE".to_owned(), hla.clone());
            if scoreable_hlas.contains(&hla) {
                let mono_id = pair_to_mono
                    .get(&(full_env_id, hla.clone()))
                    .copied()
                    .ok_or_else(|| InputError::contract("scoreable HLA lacks mono environment"))?;
                mapped.insert("env_id".to_owned(), mono_id.to_string());
                mapped.insert("mapping_status".to_owned(), "scoreable".to_owned());
            } else {
                mapped.insert(
                    "env_id".to_owned(),
                    (env_hla_pairs.len() + full_env_id as usize).to_string(),
                );
                mapped.insert("mapping_status".to_owned(), "floor".to_owned());
            }
            expanded_mapping.push(mapped);
        }
    }

    let mut mapping_seen = BTreeSet::new();
    let mut mono_mapping = Vec::new();
    let mut duplicate_mapping_rows = 0_usize;
    for row in expanded_mapping {
        if mapping_seen.insert(row_identity(&row, &mapping_headers)) {
            mono_mapping.push(row);
        } else {
            duplicate_mapping_rows += 1;
        }
    }

    let scoreable_rows = mono_mapping
        .iter()
        .filter(|row| {
            row.get("mapping_status")
                .is_some_and(|status| status == "scoreable")
        })
        .count();
    let floor_rows = mono_mapping.len() - scoreable_rows;
    for row in mono_mapping.iter().filter(|row| {
        row.get("mapping_status")
            .is_some_and(|status| status == "scoreable")
    }) {
        let lookup = (
            field(row, "patient_id", &mapping_path)?.to_owned(),
            parse_u32(
                field(row, "full_env_id", &mapping_path)?,
                "full_env_id",
                &mapping_path,
            )?,
            field(row, "nmer", &mapping_path)?.to_owned(),
        );
        let hla = field(row, "HLA-RE", &mapping_path)?;
        if !query_hlas
            .get(&lookup)
            .is_some_and(|hlas| hlas.contains(hla))
        {
            return Err(InputError::contract(format!(
                "derived scoreable mapping row lacks query support: {:?}/{hla}",
                lookup
            )));
        }
        let expected_mono = pair_to_mono
            .get(&(lookup.1, hla.to_owned()))
            .copied()
            .ok_or_else(|| InputError::contract("scoreable mapping HLA lacks mono ID"))?;
        if field(row, "env_id", &mapping_path)? != expected_mono.to_string() {
            return Err(InputError::contract(format!(
                "derived mapping has incorrect mono env_id: {:?}",
                row
            )));
        }
    }

    let noncontained: Vec<Row> = mono_mapping
        .iter()
        .filter(|row| {
            row.get("mapping_status")
                .is_some_and(|status| status == "scoreable")
        })
        .filter(|row| {
            let nmer = row.get("nmer").map_or("", String::as_str).to_uppercase();
            let long = row
                .get("long_peptide")
                .map_or("", String::as_str)
                .to_uppercase();
            !long.contains(&nmer)
        })
        .cloned()
        .collect();
    if let Some(example) = noncontained.first() {
        return Err(InputError::contract(format!(
            "{}: {} of {} scoreable mapping rows link an nmer to a long peptide that does not contain it (nmer={:?}, long_peptide={:?}, patient={:?})",
            spec.id,
            noncontained.len(),
            scoreable_rows,
            example.get("nmer"),
            example.get("long_peptide"),
            example.get("patient_id")
        )));
    }

    let source_endpoints: BTreeSet<Vec<String>> = mapping_table
        .rows
        .iter()
        .map(|row| endpoint_key(row, &spec.endpoint_fields))
        .collect();
    let output_endpoints: BTreeSet<Vec<String>> = mono_mapping
        .iter()
        .map(|row| endpoint_key(row, &spec.endpoint_fields))
        .collect();
    if source_endpoints != output_endpoints {
        return Err(InputError::contract(format!(
            "mapping endpoint population changed for {}",
            spec.id
        )));
    }

    let mut endpoint_responses: BTreeMap<Vec<String>, BTreeSet<String>> = BTreeMap::new();
    let mut endpoint_labels: BTreeMap<Vec<String>, BTreeSet<u8>> = BTreeMap::new();
    for row in &mono_mapping {
        let endpoint = endpoint_key(row, &spec.endpoint_fields);
        let response = field(row, &spec.response_column, &mapping_path)?
            .trim()
            .to_owned();
        endpoint_responses
            .entry(endpoint.clone())
            .or_default()
            .insert(response.clone());
        let label = match spec.response_kind {
            ResponseKind::Binary => {
                let value: i64 = response.parse().map_err(|_| {
                    InputError::contract(format!(
                        "invalid binary label {response:?} for endpoint {:?}",
                        endpoint
                    ))
                })?;
                if !matches!(value, 0 | 1) {
                    return Err(InputError::contract(format!(
                        "binary label must be 0 or 1, got {value} for endpoint {:?}",
                        endpoint
                    )));
                }
                value as u8
            }
            ResponseKind::Continuous => {
                let value: f64 = response.parse().map_err(|_| {
                    InputError::contract(format!(
                        "invalid continuous response {response:?} for endpoint {:?}",
                        endpoint
                    ))
                })?;
                u8::from(value > LABEL_ZERO_THRESHOLD)
            }
        };
        endpoint_labels.entry(endpoint).or_default().insert(label);
    }
    let conflicting_response_endpoints: Vec<_> = endpoint_responses
        .iter()
        .filter(|(_, values)| values.len() > 1)
        .collect();
    let conflicting_label_endpoints: Vec<_> = endpoint_labels
        .iter()
        .filter(|(_, values)| values.len() > 1)
        .collect();

    let mut materialized_label_columns = Vec::new();
    if spec.response_kind == ResponseKind::Continuous
        && let Some(noise_threshold) = spec.noise_ceiling_threshold
    {
        materialized_label_columns.extend([
            LABEL_ZERO_COLUMN.to_owned(),
            LABEL_NOISE_CEILING_COLUMN.to_owned(),
        ]);
        append_header(&mut mapping_headers, LABEL_ZERO_COLUMN);
        append_header(&mut mapping_headers, LABEL_NOISE_CEILING_COLUMN);
        for row in &mut mono_mapping {
            let raw = field(row, &spec.response_column, &mapping_path)?.trim();
            if raw.is_empty() {
                row.insert(LABEL_ZERO_COLUMN.to_owned(), String::new());
                row.insert(LABEL_NOISE_CEILING_COLUMN.to_owned(), String::new());
            } else {
                let value: f64 = raw.parse().map_err(|_| {
                    InputError::contract(format!("invalid continuous response {raw:?}"))
                })?;
                row.insert(
                    LABEL_ZERO_COLUMN.to_owned(),
                    u8::from(value > LABEL_ZERO_THRESHOLD).to_string(),
                );
                row.insert(
                    LABEL_NOISE_CEILING_COLUMN.to_owned(),
                    u8::from(value > noise_threshold).to_string(),
                );
            }
        }
    }

    let endpoints_positive_at = |threshold: f64| -> Result<usize> {
        let mut positive = 0_usize;
        for values in endpoint_responses.values() {
            let numeric: Vec<f64> = values
                .iter()
                .filter(|value| !value.is_empty())
                .map(|value| {
                    value
                        .parse::<f64>()
                        .map_err(|_| InputError::contract(format!("invalid response {value:?}")))
                })
                .collect::<Result<_>>()?;
            if !numeric.is_empty()
                && numeric.iter().copied().fold(f64::INFINITY, f64::min) > threshold
            {
                positive += 1;
            }
        }
        Ok(positive)
    };

    validate_expected(
        spec,
        unique_query.len(),
        env_hla_pairs.len(),
        mono_mapping.len(),
        scoreable_rows,
        floor_rows,
    )?;

    let mut full_mapping = mono_mapping.clone();
    for row in &mut full_mapping {
        let full_env_id = field(row, "full_env_id", &mapping_path)?.to_owned();
        row.insert("env_id".to_owned(), full_env_id);
    }
    let canonical_headers: Vec<String> = mapping_headers
        .iter()
        .map(|header| {
            if header == "env_id" {
                "mono_env_id".to_owned()
            } else {
                header.clone()
            }
        })
        .collect();
    if canonical_headers.iter().collect::<BTreeSet<_>>().len() != canonical_headers.len() {
        return Err(InputError::contract(format!(
            "{} source mapping already contains mono_env_id; canonical header would be ambiguous",
            spec.id
        )));
    }
    let canonical_mapping: Vec<Row> = mono_mapping
        .iter()
        .map(|row| {
            let mut canonical = row.clone();
            let mono_id = canonical.remove("env_id").unwrap_or_default();
            canonical.insert("mono_env_id".to_owned(), mono_id);
            canonical
        })
        .collect();

    let query_name = format!("{}_mono_query.csv", spec.prefix);
    let env_name = format!("{}_mono_hla_env_dict.csv", spec.prefix);
    let env_crosswalk_name = format!("{}_full_to_mono_env_crosswalk.csv", spec.prefix);
    let observation_crosswalk_name =
        format!("{}_full_to_mono_observation_crosswalk.csv", spec.prefix);
    let mono_mapping_name = format!("{}_mono_longpep_mapping.csv", spec.prefix);
    let full_mapping_name = format!("{}_full_longpep_mapping.csv", spec.prefix);
    let canonical_mapping_name = format!("{}_candidate_roster.csv", spec.prefix);
    let audit_name = format!("{}_inputs_audit.json", spec.prefix);
    let full_query_name = format!("{}_full_deduplicated_query.csv", spec.prefix);

    write_csv(
        &output_dir.join(&query_name),
        &query_table.headers,
        &mono_query_rows,
    )?;
    write_csv(
        &output_dir.join(&env_name),
        &["env_id".to_owned(), "allele_environment".to_owned()],
        &mono_env_rows,
    )?;
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
            "PatientID".to_owned(),
            "peptide".to_owned(),
            "HLA-RE".to_owned(),
            "TCGA_EXPR_TYPE".to_owned(),
            "gene".to_owned(),
            "SetNeoepitopeSampleID".to_owned(),
            "full_env_id".to_owned(),
            "mono_env_id".to_owned(),
        ],
        &observation_crosswalk_rows,
    )?;
    write_csv(
        &output_dir.join(&mono_mapping_name),
        &mapping_headers,
        &mono_mapping,
    )?;
    write_csv(
        &output_dir.join(&full_mapping_name),
        &mapping_headers,
        &full_mapping,
    )?;
    write_csv(
        &output_dir.join(&canonical_mapping_name),
        &canonical_headers,
        &canonical_mapping,
    )?;

    let mono_root = "${EXTERNAL_VALIDATION_INPUT_ROOT}";
    let template = read_text(&template_path)?;
    let mono_config = replace_toml_string(
        &replace_toml_string(
            &template,
            "hla_env_dict",
            &format!("{mono_root}/{env_name}"),
        )?,
        "query_peptide_input_tuples_file",
        &format!("{mono_root}/{query_name}"),
    )?;
    write_text(
        &output_dir.join(&spec.generated_mono_config),
        &format!(
            "# Generated by external_validation_inputs; source cohort directories remain read-only.\n{mono_config}"
        ),
    )?;

    let mut output_files = BTreeMap::from([
        ("query", query_name.clone()),
        ("env_dict", env_name.clone()),
        ("env_crosswalk", env_crosswalk_name.clone()),
        ("observation_crosswalk", observation_crosswalk_name.clone()),
        ("canonical_mapping", canonical_mapping_name.clone()),
        ("full_mapping", full_mapping_name.clone()),
        ("mono_mapping", mono_mapping_name.clone()),
        ("mono_config", spec.generated_mono_config.clone()),
    ]);
    if let Some(full_config_name) = &spec.generated_full_deduplicated_config {
        write_csv(
            &output_dir.join(&full_query_name),
            &query_table.headers,
            &unique_query,
        )?;
        let full_config = replace_toml_string(
            &template,
            "query_peptide_input_tuples_file",
            &format!("${{EXTERNAL_VALIDATION_INPUT_ROOT}}/{full_query_name}"),
        )?;
        write_text(
            &output_dir.join(full_config_name),
            &format!(
                "# Generated by external_validation_inputs; exact duplicate query rows removed.\n{full_config}"
            ),
        )?;
        output_files.insert("full_deduplicated_query", full_query_name.clone());
        output_files.insert("full_deduplicated_config", full_config_name.clone());
    }

    let conflict_examples: Vec<Value> = conflicting_label_endpoints
        .iter()
        .take(20)
        .map(|(key, _)| {
            let mut object = Map::new();
            for (name, value) in spec.endpoint_fields.iter().zip(key.iter()) {
                object.insert(name.clone(), Value::String(value.clone()));
            }
            object.insert(
                "response_values".to_owned(),
                serde_json::to_value(endpoint_responses.get(*key).cloned().unwrap_or_default())
                    .unwrap_or(Value::Null),
            );
            Value::Object(object)
        })
        .collect();
    let audit = json!({
        "schema_version": 2,
        "generator": "external_validation_inputs",
        "dataset": spec.id,
        "source_files": {
            "query": source_record(&query_path)?,
            "env_dict": source_record(&env_path)?,
            "mapping": source_record(&mapping_path)?,
            "config_template": source_record(&template_path)?,
        },
        "full_query_rows": query_table.rows.len(),
        "unique_full_query_compute_rows": unique_query.len(),
        "exact_duplicate_query_rows_removed": duplicate_compute_rows,
        "full_environment_rows": full_envs.len(),
        "mono_environment_rows": env_hla_pairs.len(),
        "mono_query_rows": mono_query_rows.len(),
        "full_only_biological_identities": 0,
        "mono_only_biological_identities": 0,
        "duplicate_mono_compute_identities": 0,
        "mapping_source_rows": mapping_table.rows.len(),
        "exact_duplicate_source_mapping_rows": exact_duplicate_source_rows,
        "mapping_source_rows_with_query_support": matched_source_rows,
        "mapping_source_rows_outside_query_roster": unmatched_source_rows,
        "candidate_roster_rows": mono_mapping.len(),
        "scoreable_candidate_rows": scoreable_rows,
        "floor_candidate_rows": floor_rows,
        "blank_hla_rows": 0,
        "noncontained_scoreable_mapping_rows": noncontained.len(),
        "exact_duplicate_candidate_rows_removed": duplicate_mapping_rows,
        "endpoint_key_fields": spec.endpoint_fields,
        "mapping_endpoints": source_endpoints.len(),
        "mapping_endpoints_with_conflicting_response_values": conflicting_response_endpoints.len(),
        "mapping_endpoints_with_conflicting_binary_labels": conflicting_label_endpoints.len(),
        "conflicting_binary_label_examples": conflict_examples,
        "label_zero_threshold": LABEL_ZERO_THRESHOLD,
        "noise_ceiling_threshold": spec.noise_ceiling_threshold,
        "materialized_label_columns": materialized_label_columns,
        "endpoints_positive_label_thr0": endpoints_positive_at(LABEL_ZERO_THRESHOLD)?,
        "endpoints_positive_label_noise_ceiling": match spec.noise_ceiling_threshold {
            Some(threshold) => Some(endpoints_positive_at(threshold)?),
            None => None,
        },
        "input_audit_status": if conflicting_label_endpoints.is_empty() { "pass" } else { "blocked_conflicting_endpoint_labels" },
        "suggested_smoke_env_chunks": env_hla_pairs.len(),
        "suggested_production_env_chunks": suggested_env_chunks(env_hla_pairs.len(), 4),
        "output_files": output_files,
    });
    write_json(&output_dir.join(&audit_name), &audit)?;

    Ok(DatasetBuild {
        summary: json!({
            "candidate_rows": mono_mapping.len(),
            "scoreable_rows": scoreable_rows,
            "floor_rows": floor_rows,
            "mono_environments": env_hla_pairs.len(),
            "query_rows": mono_query_rows.len(),
            "endpoints": source_endpoints.len(),
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
        let mono_name = output_files["mono_mapping"]
            .as_str()
            .ok_or_else(|| InputError::contract(format!("dataset {dataset} lacks mono_mapping")))?;
        let full_name = output_files["full_mapping"]
            .as_str()
            .ok_or_else(|| InputError::contract(format!("dataset {dataset} lacks full_mapping")))?;
        let canonical_name = output_files["canonical_mapping"].as_str().ok_or_else(|| {
            InputError::contract(format!("dataset {dataset} lacks canonical_mapping"))
        })?;
        let mono = read_csv(&bundle.join(mono_name))?;
        let full = read_csv(&bundle.join(full_name))?;
        let canonical = read_csv(&bundle.join(canonical_name))?;
        require_columns(
            &mono,
            &["HLA-RE", "mapping_status", "full_env_id", "env_id"],
            &bundle.join(mono_name),
        )?;
        if mono.headers != full.headers
            || mono.rows.len() != full.rows.len()
            || mono.rows.len() != canonical.rows.len()
        {
            return Err(InputError::contract(format!(
                "dataset {dataset} canonical/full/mono mapping shape mismatch"
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
                "dataset {dataset} canonical mapping header mismatch"
            )));
        }
        let mut observed_scoreable = 0_usize;
        let mut observed_floor = 0_usize;
        for (idx, (mono_row, full_row)) in mono.rows.iter().zip(&full.rows).enumerate() {
            let hla = mono_row.get("HLA-RE").map_or("", String::as_str);
            let status = mono_row.get("mapping_status").map_or("", String::as_str);
            if hla.is_empty() || !matches!(status, "scoreable" | "floor") {
                return Err(InputError::contract(format!(
                    "dataset {dataset} invalid mapping row {idx}: HLA={hla:?}, status={status:?}"
                )));
            }
            if status == "scoreable" {
                observed_scoreable += 1;
            } else {
                observed_floor += 1;
            }
            for header in &mono.headers {
                if header != "env_id" && mono_row.get(header) != full_row.get(header) {
                    return Err(InputError::contract(format!(
                        "dataset {dataset} full/mono mapping differs in {header} at row {idx}"
                    )));
                }
            }
            if full_row.get("env_id") != full_row.get("full_env_id") {
                return Err(InputError::contract(format!(
                    "dataset {dataset} full mapping env_id != full_env_id at row {idx}"
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
                        "dataset {dataset} canonical/mono mapping differs in {header} at row {idx}"
                    )));
                }
            }
        }
        if audit["candidate_roster_rows"].as_u64() != Some(mono.rows.len() as u64)
            || audit["scoreable_candidate_rows"].as_u64() != Some(observed_scoreable as u64)
            || audit["floor_candidate_rows"].as_u64() != Some(observed_floor as u64)
        {
            return Err(InputError::contract(format!(
                "dataset {dataset} audit counts differ from mapping contents"
            )));
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
