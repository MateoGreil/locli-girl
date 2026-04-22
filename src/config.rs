use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    pub last_station_slug: String,
    pub volume: u8,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            last_station_slug: "lofi-hip-hop-radio-beats-to-relax-study-to".to_string(),
            volume: 80,
        }
    }
}

impl Config {
    pub fn load() -> Result<Self> {
        Self::load_from(&config_path())
    }

    pub fn load_from(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(path)?;
        Ok(toml::from_str(&text).unwrap_or_default())
    }

    pub fn save(&self) -> Result<()> {
        self.save_to(&config_path())
    }

    pub fn save_to(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, toml::to_string(self)?)?;
        Ok(())
    }
}

fn config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("locli-girl")
        .join("config.toml")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn default_volume_is_80() {
        let cfg = Config::default();
        assert_eq!(cfg.volume, 80);
    }

    #[test]
    fn default_slug_is_non_empty() {
        let cfg = Config::default();
        assert!(!cfg.last_station_slug.is_empty());
    }

    #[test]
    fn round_trip_save_load() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let cfg = Config {
            last_station_slug: "synthwave".to_string(),
            volume: 55,
        };
        cfg.save_to(&path).unwrap();
        let loaded = Config::load_from(&path).unwrap();
        assert_eq!(loaded.last_station_slug, "synthwave");
        assert_eq!(loaded.volume, 55);
    }

    #[test]
    fn missing_file_returns_default() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("nonexistent.toml");
        let cfg = Config::load_from(&path).unwrap();
        assert_eq!(cfg.volume, 80);
    }
}
