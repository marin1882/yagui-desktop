use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    pub api_key:      String,
    pub tunnel_url:   String,
    pub tunnel_token: String,
}

impl Config {
    pub fn is_complete(&self) -> bool {
        !self.api_key.is_empty()
            && !self.tunnel_url.is_empty()
            && !self.tunnel_token.is_empty()
    }
}

pub fn config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("yagui").join("config.json"))
}

pub fn load() -> Config {
    let Some(path) = config_path() else {
        return Config::default();
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Config::default();
    };
    serde_json::from_str(&text).unwrap_or_default()
}

pub fn save(cfg: &Config) -> std::io::Result<()> {
    let path = config_path().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::NotFound, "no config dir")
    })?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(cfg)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(&path, text)
}
