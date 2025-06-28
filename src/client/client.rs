use futures_util::Stream;

use crate::chat::Message;
use serde::{Deserialize, Serialize};

use super::{auth::CopilotAuth, provider::Provider};
use anyhow::anyhow;
use tracing::{debug, error, info, trace};

/// # Endpoints
/// Endpoint where the auth token is retrieved for use it in completions
static HEADERS_URL: &str = "https://api.github.com/copilot_internal/v2/token";
/// Endpoint where Copilot returns a response
static COMPLETION_URL: &str = "https://api.githubcopilot.com/chat/completions";
/// This is used because Copilot requires a specified agent; otherwise, it returns a 403 status code.
static USER_AGENT: &str = "curl/8.7.1";

/// Main Copilot client
pub struct CopilotClient {
    auth: CopilotAuth,
    client: reqwest::Client,
}

/// Struct used for retrieving the token from `HEADERS_URL`
#[derive(Deserialize, Debug)]
struct HeadersResponse {
    token: String,
}

impl Provider for CopilotClient {
    /// Make a request to copilot, passing the message provided by the user
    async fn request(
        &self,
        messages: &Vec<Message>,
    ) -> anyhow::Result<impl Stream<Item = reqwest::Result<bytes::Bytes>>> {
        let headers = self.get_headers().await?;

        info!("Making request");
        trace!(?headers);
        let body = CopilotBody {
            temperature: 0.1,
            max_tokens: 4096,
            model: "gpt-4o".to_string(),
            messages,
            stream: true,
        };

        trace!(?body);
        let req = self
            .client
            .post(COMPLETION_URL)
            .header("Authorization", format!("Bearer {}", headers.auth_token))
            .header("Copilot-Integration-Id", headers.copilot_integration_id)
            .header("Editor-Version", headers.editor_version)
            .header("Editor-Plugin-Version", headers.editor_plugin_version)
            .header("User-Agent", USER_AGENT)
            .body(serde_json::to_string(&body)?);

        let resp = req.send().await?;
        debug!(?resp);

        // Stream for processing the response
        let stream = resp.bytes_stream();
        Ok(stream)
    }
}

impl CopilotClient {
    /// Create a new client
    pub fn new(auth: CopilotAuth) -> Self {
        Self {
            auth,
            client: reqwest::Client::new(),
        }
    }

    /// Get the headers and token for use in requests
    async fn get_headers(&self) -> anyhow::Result<CopilotHeaders> {
        // Main auth token is required
        if self.auth.get_token().is_none() {
            let token = self.auth.get_token();
            error!(?token, "token not found");
            return Err(anyhow!("Token not found"));
        }

        trace!(%HEADERS_URL, "retrieving headers");

        let req = self
            .client
            .get(HEADERS_URL)
            .header(
                "Authorization",
                format!("token {}", self.auth.get_token().expect("token string")),
            )
            .header("User-Agent", USER_AGENT);

        let resp = req.send().await?;
        trace!(?resp, "raw response");

        if !resp.status().is_success() {
            return Err(anyhow!("error in request, status code {:?}", resp.status()));
        }

        let resp = resp.json::<HeadersResponse>().await?;

        trace!(?resp);

        Ok(CopilotHeaders {
            auth_token: resp.token,
            editor_version: "Neovim/0.11.1".to_string(),
            editor_plugin_version: "copilot-chat".to_string(),
            copilot_integration_id: "vscode-chat".to_string(),
        })
    }
}

/// Contain all the required headers for making a request
#[derive(Debug)]
struct CopilotHeaders {
    auth_token: String,
    editor_version: String,
    editor_plugin_version: String,
    copilot_integration_id: String,
}

/// Contain the commons parameters of the model for use in requests
#[derive(Serialize, Debug)]
struct CopilotBody<'a> {
    temperature: f32,
    max_tokens: i32,
    model: String,
    stream: bool,
    messages: &'a Vec<Message>,
}
