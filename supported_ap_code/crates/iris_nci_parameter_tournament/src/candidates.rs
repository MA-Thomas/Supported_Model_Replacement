use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum MetricBranch {
    Pr,
    Roc,
}

impl MetricBranch {
    pub const ALL: [Self; 2] = [Self::Pr, Self::Roc];

    pub const fn id(self) -> &'static str {
        match self {
            Self::Pr => "pr",
            Self::Roc => "roc",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct MetricRow {
    pub regime_idx: usize,
    pub param_idx: usize,
    pub mn_idx: usize,
    pub geometry_idx: usize,
    pub d_pos: f64,
    pub d_neg: f64,
    pub steepness_pos: f64,
    pub steepness_neg: f64,
    #[serde(rename = "M")]
    pub m: u32,
    #[serde(rename = "N")]
    pub n: u32,
    pub roc_auc: f64,
    pub pr_auc: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct BiologicalKey {
    d_pos_bits: u64,
    d_neg_bits: u64,
    m: u32,
    n: u32,
}

impl From<&MetricRow> for BiologicalKey {
    fn from(row: &MetricRow) -> Self {
        Self {
            d_pos_bits: row.d_pos.to_bits(),
            d_neg_bits: row.d_neg.to_bits(),
            m: row.m,
            n: row.n,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Candidate {
    pub system_id: String,
    pub model_id: String,
    pub metric: MetricBranch,
    pub selected_rank: usize,
    #[serde(flatten)]
    pub parameters: MetricRow,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct ScanRow {
    pub ranked_position: usize,
    pub selected_rank: Option<usize>,
    pub disposition: String,
    pub representative_system_id: Option<String>,
    #[serde(flatten)]
    pub parameters: MetricRow,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CandidateSelection {
    pub selected: Vec<Candidate>,
    pub scan: Vec<ScanRow>,
}

pub fn load_metric_rows(path: &Path) -> Result<Vec<MetricRow>> {
    let mut reader = csv::Reader::from_path(path)
        .with_context(|| format!("opening metric table {}", path.display()))?;
    let headers = reader.headers()?.clone();
    let required = [
        "regime_idx",
        "param_idx",
        "mn_idx",
        "geometry_idx",
        "d_pos",
        "d_neg",
        "steepness_pos",
        "steepness_neg",
        "M",
        "N",
        "roc_auc",
        "pr_auc",
    ];
    let indices: BTreeMap<&str, usize> = required
        .into_iter()
        .map(|name| {
            headers
                .iter()
                .position(|header| header == name)
                .map(|index| (name, index))
                .with_context(|| format!("{} lacks column {name}", path.display()))
        })
        .collect::<Result<_>>()?;
    let get = |record: &csv::StringRecord, name: &str| -> Result<String> {
        Ok(record
            .get(indices[name])
            .with_context(|| format!("short row in {}", path.display()))?
            .to_owned())
    };
    let mut rows = Vec::new();
    for record in reader.records() {
        let record = record?;
        // Historical discovery tables contain a duplicated header row.
        if record.get(indices["regime_idx"]) == Some("regime_idx") {
            continue;
        }
        let row = MetricRow {
            regime_idx: get(&record, "regime_idx")?.parse()?,
            param_idx: get(&record, "param_idx")?.parse()?,
            mn_idx: get(&record, "mn_idx")?.parse()?,
            geometry_idx: get(&record, "geometry_idx")?.parse()?,
            d_pos: get(&record, "d_pos")?.parse()?,
            d_neg: get(&record, "d_neg")?.parse()?,
            steepness_pos: get(&record, "steepness_pos")?.parse()?,
            steepness_neg: get(&record, "steepness_neg")?.parse()?,
            m: get(&record, "M")?.parse()?,
            n: get(&record, "N")?.parse()?,
            roc_auc: get(&record, "roc_auc")?.parse()?,
            pr_auc: get(&record, "pr_auc")?.parse()?,
        };
        let finite = [
            row.d_pos,
            row.d_neg,
            row.steepness_pos,
            row.steepness_neg,
            row.roc_auc,
            row.pr_auc,
        ]
        .into_iter()
        .all(f64::is_finite);
        if !finite || row.m == 0 || row.n == 0 {
            bail!("invalid metric row for regime {}", row.regime_idx);
        }
        rows.push(row);
    }
    if rows.is_empty() {
        bail!("metric table {} is empty", path.display());
    }
    rows.sort_by_key(|row| row.regime_idx);
    if rows
        .windows(2)
        .any(|pair| pair[0].regime_idx == pair[1].regime_idx)
    {
        bail!("metric table contains duplicate regime_idx values");
    }
    Ok(rows)
}

fn metric_order(branch: MetricBranch, left: &MetricRow, right: &MetricRow) -> Ordering {
    let (left_primary, right_primary) = match branch {
        MetricBranch::Pr => (left.pr_auc, right.pr_auc),
        MetricBranch::Roc => (left.roc_auc, right.roc_auc),
    };
    right_primary
        .total_cmp(&left_primary)
        .then_with(|| left.regime_idx.cmp(&right.regime_idx))
}

pub fn select_candidates(
    model_id: &str,
    branch: MetricBranch,
    rows: &[MetricRow],
    candidate_count: usize,
) -> Result<CandidateSelection> {
    let mut ranked = rows.to_vec();
    ranked.sort_by(|left, right| metric_order(branch, left, right));
    let mut representatives = BTreeMap::<BiologicalKey, String>::new();
    let mut selected = Vec::with_capacity(candidate_count);
    let mut scan = Vec::with_capacity(ranked.len());
    for (offset, row) in ranked.into_iter().enumerate() {
        let ranked_position = offset + 1;
        let key = BiologicalKey::from(&row);
        let (selected_rank, disposition, representative_system_id) =
            if selected.len() >= candidate_count {
                (None, "after_candidate_limit".to_owned(), None)
            } else if let Some(representative) = representatives.get(&key) {
                (
                    None,
                    "duplicate_biological_key".to_owned(),
                    Some(representative.clone()),
                )
            } else {
                let rank = selected.len() + 1;
                let system_id = format!("{model_id}__{}__regime_{}", branch.id(), row.regime_idx);
                representatives.insert(key, system_id.clone());
                selected.push(Candidate {
                    system_id: system_id.clone(),
                    model_id: model_id.to_owned(),
                    metric: branch,
                    selected_rank: rank,
                    parameters: row.clone(),
                });
                (Some(rank), "selected".to_owned(), Some(system_id))
            };
        scan.push(ScanRow {
            ranked_position,
            selected_rank,
            disposition,
            representative_system_id,
            parameters: row,
        });
    }
    if selected.len() != candidate_count {
        bail!(
            "{model_id}/{} supplied only {} unique (d_pos,d_neg,M,N) tuples; requested {candidate_count}",
            branch.id(),
            selected.len()
        );
    }
    Ok(CandidateSelection { selected, scan })
}

pub fn write_scan(path: &Path, rows: &[ScanRow]) -> Result<()> {
    let mut writer = csv::Writer::from_path(path)?;
    writer.write_record([
        "ranked_position",
        "selected_rank",
        "disposition",
        "representative_system_id",
        "regime_idx",
        "param_idx",
        "mn_idx",
        "geometry_idx",
        "d_pos",
        "d_neg",
        "steepness_pos",
        "steepness_neg",
        "M",
        "N",
        "roc_auc",
        "pr_auc",
    ])?;
    for row in rows {
        let parameters = &row.parameters;
        writer.write_record([
            row.ranked_position.to_string(),
            row.selected_rank
                .map(|value| value.to_string())
                .unwrap_or_default(),
            row.disposition.clone(),
            row.representative_system_id.clone().unwrap_or_default(),
            parameters.regime_idx.to_string(),
            parameters.param_idx.to_string(),
            parameters.mn_idx.to_string(),
            parameters.geometry_idx.to_string(),
            parameters.d_pos.to_string(),
            parameters.d_neg.to_string(),
            parameters.steepness_pos.to_string(),
            parameters.steepness_neg.to_string(),
            parameters.m.to_string(),
            parameters.n.to_string(),
            parameters.roc_auc.to_string(),
            parameters.pr_auc.to_string(),
        ])?;
    }
    writer.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(regime_idx: usize, d_pos: f64, d_neg: f64, m: u32, n: u32, pr: f64) -> MetricRow {
        MetricRow {
            regime_idx,
            param_idx: regime_idx,
            mn_idx: 0,
            geometry_idx: regime_idx,
            d_pos,
            d_neg,
            steepness_pos: regime_idx as f64,
            steepness_neg: 1.0,
            m,
            n,
            roc_auc: pr,
            pr_auc: pr,
        }
    }

    #[test]
    fn rank_scan_profiles_steepness_within_biological_key() {
        let rows = vec![
            row(0, 15.0, 9.0, 1, 1, 0.90),
            row(1, 15.0, 9.0, 1, 1, 0.89),
            row(2, 14.0, 8.0, 1, 1, 0.88),
            row(3, 13.0, 7.0, 1, 2, 0.87),
        ];
        let result = select_candidates("full_hla", MetricBranch::Pr, &rows, 2).unwrap();
        assert_eq!(
            result
                .selected
                .iter()
                .map(|row| row.parameters.regime_idx)
                .collect::<Vec<_>>(),
            vec![0, 2]
        );
        assert_eq!(result.scan[1].disposition, "duplicate_biological_key");
        assert_eq!(result.scan[3].disposition, "after_candidate_limit");
    }
}
