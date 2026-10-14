use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use anyhow::{Context, Result};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
    pub refresh_token: Option<String>,
    pub folder_id: Option<String>,
}

impl Config {
    pub fn load() -> Result<Self> {
        let path = config_path()?;
        let mut config: Config = if path.exists() {
            let content = fs::read_to_string(&path).with_context(|| format!("não consegui ler {}", path.display()))?;
            toml::from_str(&content).with_context(|| format!("config inválido em {}", path.display()))?
        } else {
            Config::default()
        };
        if let Ok(id) = std::env::var("CX_CLIENT_ID") {
            config.client_id = Some(id);
        }
        if let Ok(secret) = std::env::var("CX_CLIENT_SECRET") {
            config.client_secret = Some(secret);
        }
        Ok(config)
    }

    pub fn save(&self) -> Result<PathBuf> {
        let path = config_path()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, toml::to_string_pretty(self)?)?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
        Ok(path)
    }

    pub fn credentials(&self) -> Result<(&str, &str)> {
        let id = non_empty(&self.client_id);
        let secret = non_empty(&self.client_secret);
        match (id, secret) {
            (Some(id), Some(secret)) => Ok((id, secret)),
            _ => anyhow::bail!("client_id/client_secret não configurados (rode `cx config`)"),
        }
    }

    pub fn refresh_token(&self) -> Result<&str> {
        non_empty(&self.refresh_token).context("não está logado (rode `cx login`)")
    }
}

fn non_empty(value: &Option<String>) -> Option<&str> {
    value.as_deref().filter(|value| !value.is_empty())
}

pub fn config_path() -> Result<PathBuf> {
    let dirs = ProjectDirs::from("", "", "caixa").context("não consegui descobrir o diretório home")?;
    Ok(dirs.config_dir().join("config.toml"))
}

pub fn run_interactive() -> Result<()> {
    let path = config_path()?;
    let mut config: Config = if path.exists() {
        toml::from_str(&fs::read_to_string(&path)?)?
    } else {
        Config::default()
    };

    println!("Enter mantém o valor atual.");
    config.client_id = ask("OAuth client ID", config.client_id, false)?;
    config.client_secret = ask("OAuth client secret", config.client_secret, true)?;

    let saved = config.save()?;
    println!("Salvo em {} (permissão 600). Agora rode `cx login`.", saved.display());
    Ok(())
}

fn ask(label: &str, current: Option<String>, secret: bool) -> Result<Option<String>> {
    let shown = match (&current, secret) {
        (Some(value), true) if !value.is_empty() => format!(" [{}]", mask(value)),
        (Some(value), false) if !value.is_empty() => format!(" [{value}]"),
        _ => String::new(),
    };
    print!("{label}{shown}: ");
    io::stdout().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let input = input.trim();
    Ok(if input.is_empty() {
        current
    } else {
        Some(input.to_string())
    })
}

fn mask(value: &str) -> String {
    let visible: String = value
        .chars()
        .rev()
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("****{visible}")
}
