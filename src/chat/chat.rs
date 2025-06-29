use std::fmt::Display;

use crate::{
    chat::prompts::GENERAL,
    client::provider::Provider,
    tools::{files::{FileRange, FileReader}, reader::ReaderTool},
};
use serde::{Deserialize, Serialize};
use tokio::{io::AsyncWrite, sync::mpsc::channel};
use tracing::{debug, error, info};

use super::{
    prompts::{CODE, COMMIT, GIT},
    stream::Streamer,
};

/// Main Chat structure, contains all chat-related attributes and methods
#[allow(dead_code)]
pub struct Chat<P: Provider> {
    messages: Vec<Message>,
    provider: P,
}

impl<P: Provider> Chat<P> {
    /// Create new chat
    pub fn new(provider: P) -> Self {
        Self {
            // All the messages of the chat
            messages: vec![],

            // Provider wich connect with the API
            provider,
        }
    }

    /// Send a message to Copilot and write the response to `Stdout` using the streamed data
    /// also returns the `System` message when it is ready.
    pub async fn send_message_with_stream(
        &self,
        message: Message,
        message_type: MessageType,
        streamer: impl Streamer + Send + 'static,
        mut writer: impl AsyncWrite + Send + Unpin + 'static,
    ) -> anyhow::Result<Message> {
        // Create the prompt
        let builder = self
            .provider
            .builder()
            .with(Message {
                role: Role::User,
                content: GENERAL.to_string(),
            })
            .with(Message {
                role: Role::User,
                content: message_type.to_string(),
            })
            .with(message);

        // Add the user message if it exists
        let builder = match message_type.resolve_user_prompt() {
            Some(user_message) => builder.with(user_message),
            None => builder,
        };

        let builder = match message_type {
            MessageType::Code {
                user_prompt: _,
                files,
            } => {
                let mut builder = builder;
                // Add the files to copilot prompt
                if let Some(files) = files {
                    for file in files {
                        let mut reader = FileReader::from_file_arg(&file);
                        let range = FileRange::from_file_arg(&file);
                        let readable = reader.get_readable();
                        reader.read(&readable).await?;
                        builder = builder.with(Message {
                            content: reader.prepare_for_copilot(&readable, range.as_ref()).await?,
                            role: Role::User,
                        })
                    }
                }
                builder
            }
            _ => builder,
        };

        let mut stream = builder.request().await?;

        debug!("Creating channels");
        let (sender, receiver) = channel(32);

        let streamer_clone = streamer.clone();

        // Write the stream to stdout
        let job = tokio::spawn(async move {
            streamer_clone
                .write_at_end(&mut writer, receiver)
                .await
                .unwrap_or_else(|e| {
                    error!(%e, "Error processing stream");
                });
        });

        info!("Collecting message");

        // Collect the message
        let message = streamer
            .handle_stream(std::pin::pin!(stream), sender)
            .await?;

        job.await?;

        info!("Message collected");
        Ok(message)
    }
}

/// A chat message
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

/// The sender of the message
#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum Role {
    #[serde(rename = "system")]
    System,
    #[serde(rename = "user")]
    User,
}

/// A builder for the initial prompt
pub struct Builder<'a, P: Provider> {
    client: &'a P,
    messages: Vec<Message>,
}

impl<'a, P: Provider> Provider for Builder<'a, P> {
    fn request(
        &self,
        messages: &Vec<Message>,
    ) -> impl Future<
        Output = anyhow::Result<impl futures_util::Stream<Item = reqwest::Result<bytes::Bytes>>>,
    > {
        self.client.request(messages)
    }
}

impl<'a, P: Provider> Builder<'a, P> {
    pub fn new(provider: &'a P) -> Self {
        Self {
            client: provider,
            messages: vec![],
        }
    }

    /// Append a message to the builder
    pub fn with(mut self, message: Message) -> Self {
        self.messages.push(message);
        self
    }

    pub fn request(
        &self,
    ) -> impl Future<
        Output = anyhow::Result<impl futures_util::Stream<Item = reqwest::Result<bytes::Bytes>>>,
    > {
        self.client.request(&self.messages)
    }
}

/// Message type to be sent to Copilot
/// each type include an user prompt
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum MessageType {
    Commit(Option<String>),
    Code {
        user_prompt: Option<String>,
        files: Option<Vec<String>>,
    },
    Git(Option<String>),
}

impl Display for MessageType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let prompt = match self {
            MessageType::Code {
                user_prompt: _,
                files: _,
            } => CODE,
            MessageType::Commit(_) => COMMIT,
            MessageType::Git(_) => GIT,
        };

        write!(f, "{}", prompt)
    }
}

impl MessageType {
    fn resolve_user_prompt(&self) -> Option<Message> {
        let prompt = match self {
            MessageType::Code {
                user_prompt,
                files: _,
            } => user_prompt,
            MessageType::Commit(user_prompt) => user_prompt,
            MessageType::Git(user_prompt) => user_prompt,
        };

        match prompt {
            Some(content) => Some(Message {
                role: Role::User,
                content: content.to_string(),
            }),
            None => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::stream::tests::TestStreamer;
    use crate::client::provider::tests::TestProvider;

    /// Simulate the > /dev/null
    struct TestWriter;
    impl AsyncWrite for TestWriter {
        fn poll_write(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
            _buf: &[u8],
        ) -> std::task::Poll<Result<usize, std::io::Error>> {
            std::task::Poll::Ready(Ok(10))
        }

        fn poll_flush(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), std::io::Error>> {
            std::task::Poll::Ready(Ok(()))
        }

        fn poll_shutdown(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), std::io::Error>> {
            std::task::Poll::Ready(Ok(()))
        }
    }

    #[tokio::test]
    async fn test_send_message_with_stream() {
        let chunk = "
        {\"choices\":[{\"index\":0,\"content_filter_offsets\":{\"check_offset\":175,\"start_offset\":176,\"end_offset\":280},
        \"content_filter_results\":{\"hate\":{\"filtered\":false,\"severity\":\"safe\"},\"self_harm\":{\"filtered\":false,
        \"severity\":\"safe\"},\"sexual\":{\"filtered\":false,\"severity\":\"safe\"},\"violence\":{\"filtered\":false,\"severity\":\"safe\"}},
        \"delta\":{\"content\":\"Rust \"}}],\"created\":1751000792,\"id\":\"chatcmpl-BmvaCUrU0DjRli6juhycOsjF1OAZr\",
        \"model\":\"gpt-4o-2024-11-20\",\"system_fingerprint\":\"fp_b705f0c291\"}
        ";

        let provider = TestProvider::new(10, chunk);
        let chat = Chat::new(provider);
        let streamer = TestStreamer;
        let writer = TestWriter;

        let message = Message {
            role: Role::User,
            content: "hello".to_string(),
        };

        let response = chat
            .send_message_with_stream(
                message,
                MessageType::Code {
                    user_prompt: None,
                    files: None,
                },
                streamer,
                writer,
            )
            .await
            .expect("process the stream");

        assert_eq!(response.content, "Rust ".repeat(10));
    }

    #[tokio::test]
    async fn test_custom_user_message() {
        let provider = TestProvider::new(10, "");
        let chat = Chat::new(provider);
        let streamer = TestStreamer;
        let writer = TestWriter;

        let message = Message {
            role: Role::User,
            content: "hello".to_string(),
        };

        chat.send_message_with_stream(
            message,
            MessageType::Code {
                user_prompt: Some("I am an user".to_string()),
                files: None,
            },
            streamer,
            writer,
        )
        .await
        .expect("process the stream");

        // The user message must be in request
        let mut exists = false;
        for message in chat.provider.input_messages.into_inner() {
            if message.content == "I am an user" {
                exists = true;
                break;
            }
        }

        assert!(exists);
    }
}
