// src/config.rs
use serde::Deserialize;
use std::error::Error;
use config::{Config, File};

#[derive(Debug, Deserialize)]
pub struct WindchimeConfig {
    pub demultiplex_barcodes: Option<String>,
    pub pipeline_env: Option<String>,
    pub skip_existing: Option<bool>,
}

impl Default for WindchimeConfig {
    fn default() -> Self {
        WindchimeConfig {
            demultiplex_barcodes: None,
            pipeline_env: None,
            skip_existing: None,
        }
    }
}

pub fn load_config(path: &str) -> Result<WindchimeConfig, Box<dyn Error>> {
    let mut settings = Config::default();
    // Reads "path.toml" or "path.json" if you added "json" feature
    settings.merge(File::with_name(path))?;

    // Either:
    let cfg: WindchimeConfig = settings.try_deserialize()?;
    // or:
    // let cfg: WindchimeConfig = settings.try_into()?;

    Ok(cfg)
}
