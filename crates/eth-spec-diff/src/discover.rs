use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::config::{Config, ForkConfig};

#[derive(Debug, Clone)]
pub struct Fork {
    pub name: String,
    pub short_name: String,
    pub path: PathBuf,
    /// Original (relative) path as written in config, for building GitHub URLs.
    pub original_path: PathBuf,
}

impl From<&ForkConfig> for Fork {
    fn from(fc: &ForkConfig) -> Self {
        Self {
            name: fc.name.clone(),
            short_name: fc.short_name.clone(),
            path: fc.path.clone(),
            original_path: fc.original_path.clone(),
        }
    }
}

#[derive(Debug)]
pub struct SourcePair {
    pub before_path: Option<PathBuf>,
    pub after_path: Option<PathBuf>,
    pub relative_path: PathBuf,
    pub output_path: PathBuf,
}

#[derive(Debug)]
pub struct DiffPair {
    pub before_fork: Fork,
    pub after_fork: Fork,
    pub sources: Vec<SourcePair>,
}

/// Walk a directory for `*.py` files, returning paths relative to `root`.
fn discover_python_files(root: &Path) -> Result<BTreeSet<PathBuf>> {
    let mut files = BTreeSet::new();
    if !root.exists() {
        return Ok(files);
    }
    walk_dir(root, root, &mut files)?;
    Ok(files)
}

fn walk_dir(base: &Path, dir: &Path, out: &mut BTreeSet<PathBuf>) -> Result<()> {
    let entries = std::fs::read_dir(dir)
        .with_context(|| format!("reading directory {}", dir.display()))?;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            walk_dir(base, &path, out)?;
        } else if path.extension().is_some_and(|ext| ext == "py") {
            let rel = path
                .strip_prefix(base)
                .with_context(|| format!("stripping prefix from {}", path.display()))?;
            out.insert(rel.to_path_buf());
        }
    }
    Ok(())
}

#[derive(Debug)]
pub struct ForkFiles {
    pub fork: Fork,
    pub files: Vec<ForkSource>,
}

#[derive(Debug)]
pub struct ForkSource {
    pub absolute_path: PathBuf,
    pub relative_path: PathBuf,
}

/// Discover all files for each individual fork.
pub fn discover_forks(config: &Config) -> Result<Vec<ForkFiles>> {
    let forks: Vec<Fork> = config.forks.iter().map(Fork::from).collect();
    let mut result = Vec::new();

    for fork in forks {
        let py_files = discover_python_files(&fork.path)
            .with_context(|| format!("discovering files in {}", fork.path.display()))?;
        let files = py_files
            .into_iter()
            .map(|rel| ForkSource {
                absolute_path: fork.path.join(&rel),
                relative_path: rel,
            })
            .collect();
        result.push(ForkFiles { fork, files });
    }

    Ok(result)
}

/// Discover all diff pairs from the config.
pub fn discover_pairs(config: &Config) -> Result<Vec<DiffPair>> {
    let forks: Vec<Fork> = config.forks.iter().map(Fork::from).collect();

    if forks.len() < 2 {
        bail!("need at least 2 forks to produce diffs");
    }

    // Discover Python files in each fork.
    let mut fork_files: BTreeMap<String, BTreeSet<PathBuf>> = BTreeMap::new();
    for fork in &forks {
        let files = discover_python_files(&fork.path)
            .with_context(|| format!("discovering files in {}", fork.path.display()))?;
        fork_files.insert(fork.short_name.clone(), files);
    }

    // Pair consecutive forks.
    let mut pairs = Vec::new();
    for window in forks.windows(2) {
        let before = &window[0];
        let after = &window[1];

        let before_files = fork_files
            .get(&before.short_name)
            .cloned()
            .unwrap_or_default();
        let after_files = fork_files
            .get(&after.short_name)
            .cloned()
            .unwrap_or_default();

        let all_paths: BTreeSet<PathBuf> = before_files
            .union(&after_files)
            .cloned()
            .collect();

        let mut sources = Vec::new();
        for rel_path in &all_paths {
            let before_path = if before_files.contains(rel_path) {
                Some(before.path.join(rel_path))
            } else {
                None
            };
            let after_path = if after_files.contains(rel_path) {
                Some(after.path.join(rel_path))
            } else {
                None
            };
            let output_path = PathBuf::from(format!(
                "{}_to_{}",
                before.short_name, after.short_name
            ))
            .join(rel_path);

            sources.push(SourcePair {
                before_path,
                after_path,
                relative_path: rel_path.clone(),
                output_path,
            });
        }

        pairs.push(DiffPair {
            before_fork: before.clone(),
            after_fork: after.clone(),
            sources,
        });
    }

    Ok(pairs)
}
