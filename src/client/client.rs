
use futures_util::StreamExt;

use serde::{Deserialize, Serialize};

use anyhow::anyhow;
use super::auth::CopilotAuth;
use tracing::{debug, error, info, trace};

static HEADERS_URL: &str = "https://api.github.com/copilot_internal/v2/token";
static COMPLETION_URL: &str = "https://api.githubcopilot.com/chat/completions";
static USER_AGENT: &str = "curl/8.7.1";

pub struct CopilotClient {
    auth: CopilotAuth,
    client: reqwest::Client,
}

#[derive(Deserialize, Debug)]
struct HeadersResponse {
    token: String,
}

impl CopilotClient {
    pub fn new(auth: CopilotAuth) -> Self {
        Self {
            auth,
            client: reqwest::Client::new(),
        }
    }

    async fn get_headers(&self) -> anyhow::Result<CopilotHeaders> {
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

    pub async fn request(&self) -> anyhow::Result<()> {
        let headers = self.get_headers().await?;

        info!("Making request");
        trace!(?headers);

        let mut messages: Vec<Message> = vec![];
        messages.push(Message {
            role: "system".to_string(),
            content: "You are a rust expert".to_string(),
        });
        messages.push(Message {role: "user".to_string(), content: "Give a guide for the most important things to learn in rust for developing the best softwares".to_string()});
        let body = CopilotBody {
            temperature: 0.1,
            max_tokens: 1000,
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

        let mut stream = resp.bytes_stream();
        let mut response = String::new();

        debug!("Opening stream");
        let mut partial_chunk = None;
        while let Some(chunk) = stream.next().await {
            debug!(?chunk, "processing");
            let chunk = chunk?;

            let mut chunk_str = String::from_utf8_lossy(&chunk);
            if partial_chunk.is_some() {
                chunk_str = format!("{}{}", partial_chunk.unwrap(), chunk_str).into();
            }
            partial_chunk = process_chunk(&chunk_str, &mut response).unwrap_or(None);
        }
        Ok(())
    }
}

fn process_chunk(chunk: &str, destination: &mut String) -> anyhow::Result<Option<String>> {
    let chunks = chunk.split("\n\n");

    for (i, chunk) in chunks.clone().into_iter().enumerate() {
        if chunk.is_empty() {
            continue;
        }
        match serde_json::from_str::<CopilotResponse>(&chunk[6..]) {
            Ok(resp_msg) => {
                if let Some(choice) = resp_msg.choices.first() {
                    if let Some(msg) = &choice.delta {
                        let msg = msg.content.clone();
                        print!("{}", msg);
                        destination.push_str(&msg);
                    }
                }
            }
            Err(e) => {
                // Is the last, should be a cutted chunk
                if chunks.count() == i + 1 {
                    return Ok(Some(chunk.to_string()));
                }
                return Err(e.into());
            }
        }
    }
    Ok(None)
}

#[derive(Debug)]
struct CopilotHeaders {
    auth_token: String,
    editor_version: String,
    editor_plugin_version: String,
    copilot_integration_id: String,
}

#[derive(Serialize, Debug)]
struct CopilotBody {
    temperature: f32,
    max_tokens: i32,
    model: String,
    stream: bool,
    messages: Vec<Message>,
}

#[derive(Serialize, Deserialize, Debug)]
struct Message {
    role: String,
    content: String,
}

#[derive(Deserialize, Debug)]
struct Delta {
    content: String,
}

#[derive(Debug, Deserialize)]
struct CopilotResponse {
    choices: Vec<Choice>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct Choice {
    delta: Option<Delta>,
    index: i32,
    finish_reason: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chunk_parsing() {
        let chunk = "
        {\"choices\":[{\"index\":0,\"content_filter_offsets\":{\"check_offset\":175,\"start_offset\":176,\"end_offset\":280},
        \"content_filter_results\":{\"hate\":{\"filtered\":false,\"severity\":\"safe\"},\"self_harm\":{\"filtered\":false,
        \"severity\":\"safe\"},\"sexual\":{\"filtered\":false,\"severity\":\"safe\"},\"violence\":{\"filtered\":false,\"severity\":\"safe\"}},
        \"delta\":{\"content\":\" safety\"}}],\"created\":1751000792,\"id\":\"chatcmpl-BmvaCUrU0DjRli6juhycOsjF1OAZr\",
        \"model\":\"gpt-4o-2024-11-20\",\"system_fingerprint\":\"fp_b705f0c291\"}
        ";

        let mut dest = String::new();
        let resp = process_chunk(chunk, &mut dest);

        assert!(resp.is_ok())
    }

    #[test]
    fn test_double_chunk_parsing() {
        let double = "
        {\"choices\":[{\"index\":0,\"content_filter_offsets\":{\"check_offset\":175,\"start_offset\":334,\"end_offset\":435},
        \"content_filter_results\":{\"hate\":{\"filtered\":false,\"severity\":\"safe\"},\"self_harm\":{\"filtered\":false,
        \"severity\":\"safe\"},\"sexual\":{\"filtered\":false,\"severity\":\"safe\"},
        \"violence\":{\"filtered\":false,\"severity\":\"safe\"}},\"delta\":{\"content\":\" the\"}}],
        \"created\":1751000792,\"id\":\"chatcmpl-BmvaCUrU0DjRli6juhycOsjF1OAZr\",\"model\":\"gpt-4o-2024-11-20\",
        \"system_fingerprint\":\"fp_b705f0c291\"}\n\ndata: {\"choices\":[{\"index\":0,\"content_filter_offsets\":{\"check_offset\":175,\"start_offset\":334,
        \"end_offset\":435},\"content_filter_results\":{\"hate\":{\"filtered\":false,\"severity\":\"safe\"},\"self_harm\":{\"filtered\":false,\"severity\":\"safe\"},
        \"sexual\":{\"filtered\":false,\"severity\":\"safe\"},\"violence\":{\"filtered\":false,\"severity\":\"safe\"}},
        \"delta\":{\"content\":\" most\"}}],\"created\":1751000792,\"id\":\"chatcmpl-BmvaCUrU0DjRli6juhycOsjF1OAZr\",
        \"model\":\"gpt-4o-2024-11-20\",\"system_fingerprint\":\"fp_b705f0c291\"}
        ";

        let mut dest = String::new();
        let resp = process_chunk(double, &mut dest);

        assert!(resp.is_ok())
    }
}
