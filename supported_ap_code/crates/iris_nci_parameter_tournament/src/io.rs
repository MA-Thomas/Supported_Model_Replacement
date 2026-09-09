use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Serialize;
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};

pub fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    serde_json::from_reader(BufReader::new(file))
        .with_context(|| format!("parsing JSON {}", path.display()))
}

pub fn write_json_new<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| format!("creating {}", path.display()))?;
    let mut writer = BufWriter::new(file);
    serde_json::to_writer_pretty(&mut writer, value)?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    Ok(())
}

pub fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let file = File::create(path).with_context(|| format!("creating {}", path.display()))?;
    let mut writer = BufWriter::new(file);
    serde_json::to_writer_pretty(&mut writer, value)?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    Ok(())
}

pub fn sha256_file(path: &Path) -> Result<String> {
    let mut file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 1 << 20];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

pub fn stage_path(output: &Path) -> Result<PathBuf> {
    if output.exists() {
        bail!("output already exists: {}", output.display());
    }
    let parent = output.parent().context("output has no parent")?;
    fs::create_dir_all(parent)?;
    let name = output
        .file_name()
        .and_then(|value| value.to_str())
        .context("output name is not UTF-8")?;
    let stage = parent.join(format!(".{name}.stage-{}", std::process::id()));
    if stage.exists() {
        bail!("staging path already exists: {}", stage.display());
    }
    fs::create_dir(&stage)?;
    Ok(stage)
}

pub fn collect_file_hashes(root: &Path) -> Result<std::collections::BTreeMap<String, String>> {
    fn walk(root: &Path, directory: &Path, paths: &mut Vec<PathBuf>) -> Result<()> {
        for entry in fs::read_dir(directory)? {
            let path = entry?.path();
            if path.is_dir() {
                walk(root, &path, paths)?;
            } else if path.is_file() {
                paths.push(path.strip_prefix(root)?.to_path_buf());
            }
        }
        Ok(())
    }

    let mut paths = Vec::new();
    walk(root, root, &mut paths)?;
    paths.sort();
    paths
        .into_iter()
        .filter(|path| path != Path::new("manifest.json"))
        .map(|relative| {
            let hash = sha256_file(&root.join(&relative))?;
            Ok((relative.to_string_lossy().into_owned(), hash))
        })
        .collect()
}
