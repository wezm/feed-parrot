use std::fs;
use std::path::{Path, PathBuf};

use reqwest::Url;
use serde::{Deserialize, Serialize};
use simple_eyre::eyre;
use simple_eyre::eyre::{eyre, WrapErr};

#[derive(Deserialize, Serialize)]
pub(crate) struct Config {
    client_id: String,
    client_secret: String,
    instance: String,
    pub(crate) access_token: String,
    pub(crate) last_seen_id: Option<String>,
    pub(crate) archive_path: String,
    #[serde(skip)]
    /// Path where this configuration was loaded from
    path: Option<PathBuf>,
}

impl Config {
    pub(crate) fn instance_url(&self) -> eyre::Result<Url> {
        self.instance
            .parse()
            .wrap_err("unable to parse instance url in configuration")
    }
}

impl Config {
    pub fn new(
        client_id: String,
        client_secret: String,
        instance: String,
        token: String,
        archive_path: String,
    ) -> Self {
        Config {
            client_id,
            client_secret,
            instance,
            access_token: token,
            last_seen_id: None,
            archive_path,
            path: None,
        }
    }

    /// Read the config file path and the supplied path or default if `None`.
    pub fn read(config_path: PathBuf) -> eyre::Result<Config> {
        // let dirs = crate::dirs::new()?;
        // FIXME: Bail if there's no config file: you need to auth
        // let config_path = config_path.ok_or(()).or_else(|()| {
        //     dirs.place_config_file("config.json")
        //         .wrap_err("unable to create path to config file")
        // })?;
        let raw_config = fs::read_to_string(&config_path).wrap_err_with(|| {
            format!(
                "unable to read configuration file: {}",
                config_path.display()
            )
        })?;
        let mut config: Config = serde_json::from_str(&raw_config).wrap_err_with(|| {
            format!(
                "unable to parse configuration file: {}",
                config_path.display()
            )
        })?;
        config.path = Some(config_path);
        Ok(config)
    }

    /// Write this config to the supplied path.
    pub fn write(&self) -> eyre::Result<()> {
        match self.path {
            Some(ref path) => self.write_to_path(path),
            None => Err(eyre!("unable to write config without a path")),
        }
    }

    fn write_to_path(&self, config_path: &Path) -> eyre::Result<()> {
        let config_json = serde_json::to_string_pretty(self)?;
        // TODO: Consider atomic write
        fs::write(config_path, config_json.as_bytes())?;
        Ok(())
    }

    /// Create the config file at the supplied path or the default path if `None`.
    pub fn create(config_path: PathBuf, mut config: Config) -> eyre::Result<Config> {
        // let dirs = crate::dirs::new()?;
        // let config_path = config_path.ok_or(()).or_else(|()| {
        //     dirs.place_config_file("config.json")
        //         .wrap_err("unable to create path to config file")
        // })?;
        config.path = Some(config_path);
        config.write()?;
        Ok(config)
    }
}
