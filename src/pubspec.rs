use std::fs;
use std::path::Path;
use anyhow::{anyhow, Result};

#[derive(Debug, Clone)]
pub struct PubspecInfo {
    pub name: String,
    pub version: String,
    pub is_flutter: bool,
}

pub struct PubspecScanner;

impl PubspecScanner {
    /// Parsea de forma ultraveloz `pubspec.yaml` sin sobrecarga de runtime
    pub fn scan(project_dir: &Path) -> Result<PubspecInfo> {
        let pubspec_path = project_dir.join("pubspec.yaml");
        if !pubspec_path.exists() {
            return Err(anyhow!("No se encontró pubspec.yaml en {:?}", project_dir));
        }

        let content = fs::read_to_string(&pubspec_path)?;
        let mut name = String::new();
        let mut version = String::new();
        let mut is_flutter = false;

        for line in content.lines() {
            let line = line.trim();
            if line.starts_with('#') {
                continue;
            }

            if line.starts_with("name:") {
                name = line.strip_prefix("name:").unwrap_or("").trim().to_string();
            } else if line.starts_with("version:") {
                version = line.strip_prefix("version:").unwrap_or("").trim().to_string();
            } else if line.contains("sdk: flutter") || line == "flutter:" {
                is_flutter = true;
            }
        }

        if name.is_empty() {
            name = "unnamed".to_string();
        }

        Ok(PubspecInfo {
            name,
            version,
            is_flutter,
        })
    }
}
