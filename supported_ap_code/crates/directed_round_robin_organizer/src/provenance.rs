use std::path::Path;
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::Result;
use crate::identity::sha256_file;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildProvenance {
    pub repository_git_commit: Option<String>,
    pub repository_dirty: Option<bool>,
    pub cargo_lock_sha256: Option<String>,
    pub rustc_version: Option<String>,
    pub compilation_target: String,
    pub enabled_features: Vec<String>,
    pub executable_sha256: Option<String>,
    pub host: Option<String>,
}

pub fn capture_build_provenance() -> Result<BuildProvenance> {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let repository_git_commit = command_output(
        Command::new("git")
            .arg("-C")
            .arg(&workspace)
            .args(["rev-parse", "HEAD"]),
    );
    let repository_dirty = command_output(Command::new("git").arg("-C").arg(&workspace).args([
        "status",
        "--porcelain",
        "--untracked-files=normal",
    ]))
    .map(|status| !status.is_empty());
    let cargo_lock = workspace.join("Cargo.lock");
    let cargo_lock_sha256 = cargo_lock
        .is_file()
        .then(|| sha256_file(&cargo_lock))
        .transpose()?;
    let rustc_version = command_output(Command::new("rustc").arg("--version"));
    let executable_sha256 = std::env::current_exe()
        .ok()
        .filter(|path| path.is_file())
        .map(|path| sha256_file(&path))
        .transpose()?;
    Ok(BuildProvenance {
        repository_git_commit,
        repository_dirty,
        cargo_lock_sha256,
        rustc_version,
        compilation_target: format!("{}-{}", std::env::consts::ARCH, std::env::consts::OS),
        enabled_features: vec!["supported-ap/parallel".into()],
        executable_sha256,
        host: std::env::var("HOSTNAME")
            .or_else(|_| std::env::var("COMPUTERNAME"))
            .ok(),
    })
}

fn command_output(command: &mut Command) -> Option<String> {
    let output = command.output().ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout)
        .ok()
        .map(|value| value.trim().to_owned())
}
