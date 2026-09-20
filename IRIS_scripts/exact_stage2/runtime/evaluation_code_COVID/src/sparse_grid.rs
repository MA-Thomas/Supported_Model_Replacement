//! Exact geometry/MN coverage. No P, N, Q or Pi calculations are performed here.
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use anyhow::{bail, Context, Result};
use serde::Deserialize;
use super::assembly_contract::ExpectedGrid;

#[derive(Debug, Deserialize)]
pub struct GridFile { pub path: PathBuf, pub sha256: String, pub bytes: u64 }
#[derive(Debug, Deserialize)]
pub struct Geometry {
    pub geometry_idx: usize,
    pub geometry_params_str: [String; 4],
    pub mn_pairs: Vec<(u32, u32)>,
    pub mn_file: GridFile,
}
#[derive(Debug, Deserialize)]
pub struct SparseGrid {
    pub schema_version: u32,
    pub kind: String,
    pub construction: String,
    pub hla_environment_representation: String,
    pub pn_hla_scope: String,
    pub n_regimes: usize,
    pub tau_values_str: Vec<String>,
    pub parameter_file: GridFile,
    pub mn_union_file: GridFile,
    pub geometries: Vec<Geometry>,
}

fn bits(values: (f32, f32, f32, f32)) -> [u32; 4] {
    [values.0.to_bits(), values.1.to_bits(), values.2.to_bits(), values.3.to_bits()]
}
fn geometry_bits(g: &Geometry) -> Result<[u32; 4]> {
    let v = g.geometry_params_str.iter().map(|s| s.parse::<f32>()).collect::<std::result::Result<Vec<_>, _>>()?;
    if v.iter().any(|x| !x.is_finite() || *x <= 0.0) { bail!("invalid sparse geometry"); }
    Ok(bits((v[0], v[1], v[2], v[3])))
}

impl SparseGrid {
    pub fn validate(&self, expected: &ExpectedGrid, representation: &str, scope: &str) -> Result<()> {
        if self.schema_version != 1 || self.kind != "survivor_exact_stage2_grid" {
            bail!("exact survivor grid v1 required");
        }
        let construction = match (representation, scope) {
            ("full", "all") => "full", ("full", "focal") => "focal", ("mono", "all") => "mono",
            _ => anyhow::bail!("invalid representation/scope"),
        };
        if self.construction != construction || self.hla_environment_representation != representation || self.pn_hla_scope != scope {
            bail!("survivor grid construction differs from run");
        }
        let taus = ["1000", "50052", "91691", "120604", "156295", "200000"];
        if self.tau_values_str != taus || expected.tau_values.len() != 6 {
            bail!("all six frozen taus required");
        }
        let actual_tau: HashSet<_> = expected.tau_values.iter().map(|x| x.to_bits()).collect();
        let desired_tau: HashSet<_> = taus.iter().map(|x| x.parse::<f32>().unwrap().to_bits()).collect();
        if actual_tau != desired_tau { bail!("tau values differ from frozen grid"); }
        let mut geometries = HashSet::new();
        let mut union = HashSet::new();
        let mut count = 0;
        for (i, g) in self.geometries.iter().enumerate() {
            if g.geometry_idx != i || !geometries.insert(geometry_bits(g)?) { bail!("duplicate/misindexed geometry"); }
            let pairs: HashSet<_> = g.mn_pairs.iter().copied().collect();
            if pairs.is_empty() || pairs.len() != g.mn_pairs.len() || pairs.iter().any(|p| p.0 == 0 || p.1 == 0) { bail!("invalid sparse M/N pairs"); }
            count += pairs.len(); union.extend(pairs);
        }
        let expected_geometry: HashSet<_> = expected.geometry_params.iter().map(|g| bits(*g)).collect();
        let expected_mn: HashSet<_> = expected.mn_tuples.iter().copied().collect();
        if geometries != expected_geometry || union != expected_mn || count != self.n_regimes || count == 0 {
            bail!("sparse geometry/MN axes disagree with manifest CSV inputs");
        }
        Ok(())
    }

    pub fn cells(&self, geometry: &[(f32, f32, f32, f32)], n_tau: usize, mn: &[(u32,u32)]) -> Result<HashSet<(u32,u32)>> {
        let indices: HashMap<_,_> = geometry.iter().enumerate().map(|(i,g)| (bits(*g),i)).collect();
        let mn_indices: HashMap<_,_> = mn.iter().enumerate().map(|(i,p)| (*p,i)).collect();
        let mut result = HashSet::new();
        for g in &self.geometries {
            let index = indices.get(&geometry_bits(g)?).context("required geometry absent from tensor")?;
            for pair in &g.mn_pairs {
                let m = mn_indices.get(pair).context("required M/N absent from tensor")?;
                for tau in 0..n_tau { result.insert(((index * n_tau + tau) as u32, *m as u32)); }
            }
        }
        Ok(result)
    }
}

pub fn verify_file(root: &Path, record: &GridFile) -> Result<PathBuf> {
    let root = root.canonicalize()?;
    let path = root.join(&record.path).canonicalize()?;
    if !path.starts_with(&root) || std::fs::metadata(&path)?.len() != record.bytes
        || super::assembly_contract::sha256_file(&path)? != record.sha256 {
        bail!("sparse input hash/path mismatch: {}", path.display());
    }
    Ok(path)
}

pub fn validate_cells(entries: &[(usize,u32,u32,f32,f32,f32,f32)], n_observations: usize,
                      allowed: &HashSet<(u32,u32)>) -> Result<()> {
    if allowed.is_empty() || n_observations == 0 { bail!("empty exact tensor roster"); }
    let rank: HashMap<_,_> = allowed.iter().enumerate().map(|(i, p)| (*p,i)).collect();
    let size = n_observations.checked_mul(rank.len()).context("sparse coverage size overflow")?;
    let mut seen = vec![false; size];
    for &(obs, p, m, q, pos, neg, pi) in entries {
        if obs >= n_observations { bail!("invalid observation index"); }
        if [q,pos,neg,pi].iter().any(|v| !v.is_finite()) { bail!("nonfinite tensor component"); }
        let r = rank.get(&(p,m)).context("unrequested geometry/MN tensor cell")?;
        let index = obs * rank.len() + r;
        if seen[index] { bail!("Duplicate tensor cell for obs={obs} param={p} mn={m}"); }
        seen[index] = true;
    }
    if seen.iter().any(|v| !v) { bail!("Tensor coverage is incomplete for exact survivor grid"); }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sparse_holes_are_allowed_but_missing_extra_and_duplicate_cells_fail() {
        let allowed = HashSet::from([(0,0),(1,1)]);
        let rows = vec![(0,0,0,0.1,0.2,0.3,0.4),(0,1,1,0.1,0.2,0.3,0.4)];
        validate_cells(&rows,1,&allowed).unwrap();
        assert!(validate_cells(&rows[..1],1,&allowed).is_err());
        let mut extra=rows.clone();extra.push((0,0,1,0.1,0.2,0.3,0.4));
        assert!(validate_cells(&extra,1,&allowed).is_err());
        let mut duplicate=rows.clone();duplicate.push(rows[0]);
        assert!(validate_cells(&duplicate,1,&allowed).is_err());
        let mut substituted=rows.clone();substituted[1]=rows[0];
        assert!(validate_cells(&substituted,1,&allowed).is_err());
    }
}
