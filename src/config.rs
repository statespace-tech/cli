//! The saved session, shared with the SDKs for local development.

use std::{fs, path::PathBuf};

use anyhow::Context;
use serde::{Deserialize, Serialize};

pub const DEFAULT_ENDPOINT: &str = "https://api.statespace.com";

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Settings {
    pub endpoint: Option<String>,
    pub token: Option<String>,
}

impl Settings {
    /// `$STATESPACE_CONFIG`, or `statespace/config.toml` in the user's configuration directory.
    pub fn path() -> anyhow::Result<PathBuf> {
        if let Some(path) = std::env::var_os("STATESPACE_CONFIG") {
            return Ok(PathBuf::from(path));
        }
        Ok(dirs::config_dir()
            .context("the user configuration directory is unavailable")?
            .join("statespace/config.toml"))
    }

    pub fn load() -> anyhow::Result<Self> {
        let path = Self::path()?;
        match fs::read_to_string(&path) {
            Ok(text) => {
                toml::from_str(&text).with_context(|| format!("invalid {}", path.display()))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error.into()),
        }
    }

    /// Write the file atomically, readable only by the current user.
    pub fn save(&self) -> anyhow::Result<()> {
        let path = Self::path()?;
        let directory = path.parent().context("invalid configuration path")?;
        fs::create_dir_all(directory)?;
        let file = tempfile::NamedTempFile::new_in(directory)?;
        fs::write(file.path(), toml::to_string_pretty(self)?)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(file.path(), fs::Permissions::from_mode(0o600))?;
        }
        file.persist(&path)?;
        Ok(())
    }
}
