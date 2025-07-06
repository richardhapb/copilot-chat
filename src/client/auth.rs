use anyhow::anyhow;
use std::{fs::File, io::Read};

use serde::Deserialize;
use tracing::{debug, error, trace};

/// Handle all related authorizations features of Copilot
#[derive(Debug, Deserialize, Default)]
pub struct CopilotAuth {
    oauth_token: Option<String>,
}

impl CopilotAuth {
    /// Try to get the main token and return an instance.
    /// If the token cannot be found, it will be None.
    pub fn new() -> Self {
        let mut auth = Self { oauth_token: None };
        auth.get_token_from_file().unwrap();

        auth
    }

    /// Return an [`Option`], maybe with the token
    pub fn get_token(&self) -> Option<&str> {
        self.oauth_token.as_deref()
    }

    /// Retrieve the Copilot token from known directories
    /// By default, it looks in `~/.config/github-copilot/apps.json`
    fn get_token_from_file(&mut self) -> anyhow::Result<Option<&str>> {
        // Return an existent token if exists
        if self.oauth_token.is_some() {
            return Ok(self.oauth_token.as_deref());
        }

        debug!("Token not found; searching for it in the file.");

        let config_path = dirs::home_dir().expect("path is resolved");
        let copilot_file = config_path.join(".config").join("github-copilot").join("apps.json");

        debug!(?copilot_file, "Looking for token");

        let mut file = File::open(copilot_file)?;
        let mut file_str = String::new();
        let n = file.read_to_string(&mut file_str)?;
        if n == 0 {
            error!("Emptyt config file");
            return Err(anyhow!("Empty config file"));
        }

        trace!(%file_str, "File found");

        let clean_str = match file_str[14..].split_once(":") {
            Some(substr) => substr.1.trim()[..substr.1.trim().len() - 1].to_string(),
            None => file_str.clone(),
        };

        trace!(%clean_str);

        let copilot_auth = serde_json::from_str::<CopilotAuth>(&clean_str)?;
        trace!(?copilot_auth.oauth_token, "Token found");

        self.oauth_token = copilot_auth.oauth_token;
        Ok(self.oauth_token.as_deref())
    }
}
