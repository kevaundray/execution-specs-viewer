use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Config {
    pub spec_root: PathBuf,
    #[serde(default = "default_output")]
    pub output: PathBuf,
    #[serde(default)]
    pub source_url: Option<String>,
    #[serde(rename = "forks")]
    pub forks: Vec<ForkConfig>,
}

fn default_output() -> PathBuf {
    PathBuf::from("diff-output")
}

#[derive(Debug, Deserialize)]
pub struct ForkConfig {
    pub name: String,
    pub short_name: String,
    pub path: PathBuf,
    /// Original (relative) path as written in config, before resolution.
    #[serde(skip)]
    pub original_path: PathBuf,
}

impl Config {
    pub fn load(path: &Path) -> Result<Config> {
        let content =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let mut config: Config =
            toml::from_str(&content).with_context(|| format!("parsing {}", path.display()))?;

        // Resolve spec_root relative to the config file's directory.
        let config_dir = path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
        if config.spec_root.is_relative() {
            config.spec_root = config_dir.join(&config.spec_root);
        }

        // Resolve fork paths relative to spec_root.
        for fork in &mut config.forks {
            fork.original_path = fork.path.clone();
            if fork.path.is_relative() {
                fork.path = config.spec_root.join(&fork.path);
            }
        }

        // Resolve output relative to spec_root.
        if config.output.is_relative() {
            config.output = config.spec_root.join(&config.output);
        }

        Ok(config)
    }
}
