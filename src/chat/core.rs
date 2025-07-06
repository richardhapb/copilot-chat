use std::{
    cell::RefCell,
    env::current_dir,
    fmt::Display,
    fs::{File, create_dir_all},
    io::Write,
    path::PathBuf,
    str::FromStr,
};

use crate::{
    chat::prompts::GENERAL,
    client::provider::Provider,
    tools::{
        diff::{DiffsManager, Range},
        files::{FileReader, TrackedFile},
        reader::{Readable, ReaderTool},
    },
};
use percent_encoding::{NON_ALPHANUMERIC, percent_encode};
use serde::{Deserialize, Serialize};
use tokio::{io::AsyncWrite, sync::mpsc::channel};
use tracing::{debug, error, info, trace};

use super::{
    errors::ChatError,
    prompts::{CODE, COMMIT, GIT},
    stream::Streamer,
};

/// Main Chat structure, contains all chat-related attributes and methods
#[derive(Serialize, Deserialize, Debug)]
pub struct Chat<P: Provider> {
    messages: RefCell<Vec<Message>>,
    #[serde(skip)]
    provider: P,
    tracked_files: Vec<TrackedFile>,
}

impl<P: Provider + Default> Chat<P> {
    /// Create new chat
    pub fn new(provider: P) -> Self {
        Self {
            messages: RefCell::new(vec![]),
            provider,
            tracked_files: vec![],
        }
    }

    pub fn with_provider(mut self, provider: P) -> Self {
        self.provider = provider;
        self
    }

    pub fn add_message(&self, message: Message) {
        self.messages.borrow_mut().push(message);
    }

    /// Try to load a chat for the current directory
    pub fn try_load_chat(path: Option<&str>) -> Result<Option<Self>, ChatError> {
        let cache = Self::get_cache_path(path)?;
        let cwd = current_dir()?;
        let encoded = percent_encode(
            cwd.to_str()
                .ok_or_else(|| {
                    ChatError::CacheError(std::io::Error::new(std::io::ErrorKind::InvalidData, "Invalid path"))
                })?
                .as_bytes(),
            NON_ALPHANUMERIC,
        );

        let cache_file = cache.join(format!("{}.json", encoded));
        if !cache_file.exists() {
            return Ok(None);
        }

        let chat_str = std::fs::read_to_string(&cache_file)?;
        Ok(Some(serde_json::from_str(&chat_str)?))
    }

    /// Send a message to Copilot and write the response to `Stdout` using the streamed data
    /// also returns the `System` message when it is ready.
    pub async fn send_message_with_stream(
        &mut self,
        model: Option<&str>,
        message: Message,
        message_type: MessageType,
        streamer: impl Streamer + 'static,
        mut writer: impl AsyncWrite + Send + Unpin + 'static,
    ) -> Result<Message, ChatError> {
        let mut builder = prepare_builder(&self.provider, &self.messages, message, &message_type)?;
        Self::handle_files(&mut self.tracked_files, &message_type, &mut builder).await?;

        trace!("sending request to copilot");
        let stream = builder
            .request(model.unwrap_or("gpt-4o"))
            .await
            .map_err(|e| ChatError::ProviderError(e.to_string()))?;

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
            .await
            .map_err(|e| ChatError::StreamError(e.to_string()))?;

        job.await?;

        info!("Message collected");
        Ok(message)
    }

    /// Save the chat for the current directory
    pub fn save_chat(&self, path: Option<&str>) -> Result<(), ChatError> {
        let cache = Self::get_cache_path(path)?;
        create_dir_all(&cache)?;
        info!(?cache, "Saving chat");

        let cwd = current_dir()?;
        let encoded = percent_encode(
            cwd.to_str()
                .ok_or_else(|| {
                    ChatError::CacheError(std::io::Error::new(std::io::ErrorKind::InvalidData, "Invalid path"))
                })?
                .as_bytes(),
            NON_ALPHANUMERIC,
        );

        let cache_file = cache.join(format!("{}.json", encoded));
        let mut file = File::create(&cache_file)?;
        file.write_all(serde_json::to_string(self)?.as_bytes())?;
        info!(?cache_file, "Chat saved successfully");
        Ok(())
    }

    /// Delete the saved chat for the current directory
    pub fn remove_chat(&self, path: Option<&str>) -> Result<(), ChatError> {
        let cache = Self::get_cache_path(path)?;
        info!(?cache, "Deleting chat");

        let cwd = current_dir()?;
        let encoded = percent_encode(
            cwd.to_str()
                .ok_or_else(|| {
                    ChatError::CacheError(std::io::Error::new(std::io::ErrorKind::InvalidData, "Invalid path"))
                })?
                .as_bytes(),
            NON_ALPHANUMERIC,
        );

        let cache_file = cache.join(format!("{}.json", encoded));
        if cache_file.exists() {
            std::fs::remove_file(&cache_file)?;
            info!(?cache_file, "Chat deleted successfully");
        } else {
            info!(?cache_file, "Chat not found; skipping deletion.");
        }
        Ok(())
    }

    fn get_cache_path(path: Option<&str>) -> Result<PathBuf, ChatError> {
        if let Some(path) = path {
            PathBuf::from_str(path)
                .map_err(|e| ChatError::CacheError(std::io::Error::new(std::io::ErrorKind::InvalidData, e)))
        } else {
            dirs::home_dir()
                .map(|home| home.join(".cache").join("copilot-chat"))
                .ok_or_else(|| {
                    ChatError::CacheError(std::io::Error::new(
                        std::io::ErrorKind::NotFound,
                        "Home directory not found",
                    ))
                })
        }
    }

    async fn handle_files<'a>(
        tracked_files: &mut Vec<TrackedFile>,
        message_type: &MessageType,
        builder: &mut Builder<'a, P>,
    ) -> Result<(), ChatError> {
        if let MessageType::Code { files, .. } = message_type {
            if let Some(files) = files {
                for file in files {
                    debug!(%file, "Processing file");
                    Self::process_file(tracked_files, file, builder).await?;
                }
            }
        }
        Ok(())
    }

    async fn process_file<'a>(
        tracked_files: &mut Vec<TrackedFile>,
        file: &str,
        builder: &mut Builder<'a, P>,
    ) -> Result<(), ChatError> {
        let reader = FileReader;
        let range = Range::from_file_arg(file);
        let file_path = if let Some((path, _)) = file.split_once(':') {
            path
        } else {
            file
        };

        if let Some(index) = tracked_files.iter().position(|p| p.path == file_path) {
            let mut tracked_file = tracked_files.remove(index);

            if tracked_file.content().is_empty() {
                info!(%file, "Tracked file content empty, reading");
                reader
                    .read(&mut tracked_file)
                    .await
                    .map_err(|e| ChatError::ToolError(e.to_string()))?;
            }

            let file_path = tracked_file.location();
            info!(?file_path, "File tracked, checking for differences");

            let diff_man = reader
                .get_diffs(&tracked_file)
                .map_err(|e| ChatError::ToolError(e.to_string()))?;
            reader
                .read(&mut tracked_file)
                .await
                .map_err(|e| ChatError::ToolError(e.to_string()))?;

            if let Some(diff_man) = diff_man {
                info!("Differences found, sending to copilot");
                debug!("Differences: {:?}", diff_man);
                builder.with_diffs(&diff_man, tracked_file.location());
            } else {
                debug!("No differences found, skipping the update.");
            }

            let file_content = tracked_file
                .prepare_for_copilot(range.as_ref())
                .await
                .map_err(|e| ChatError::ToolError(e.to_string()))?;
            builder.with(Message {
                content: file_content,
                role: Role::User,
            });

            tracked_files.insert(index, tracked_file);
        } else {
            let mut tracked_file = TrackedFile::from_file_arg(file);
            reader
                .read(&mut tracked_file)
                .await
                .map_err(|e| ChatError::ToolError(e.to_string()))?;

            let file_path = tracked_file.location();
            info!(?file_path, "File not tracked, sending to copilot");

            let load_content = tracked_file
                .prepare_load_once()
                .await
                .map_err(|e| ChatError::ToolError(e.to_string()))?;
            builder.with(Message {
                content: load_content,
                role: Role::User,
            });

            let copilot_content = tracked_file
                .prepare_for_copilot(range.as_ref())
                .await
                .map_err(|e| ChatError::ToolError(e.to_string()))?;
            builder.with(Message {
                content: copilot_content,
                role: Role::User,
            });

            tracked_files.push(tracked_file);
        }
        Ok(())
    }
}

fn prepare_builder<'a, P: Provider>(
    provider: &'a P,
    messages: &'a RefCell<Vec<Message>>,
    message: Message,
    message_type: &MessageType,
) -> Result<Builder<'a, P>, ChatError> {
    let mut builder = provider.builder(messages);
    if builder.messages.borrow().is_empty() {
        builder
            .with(Message {
                role: Role::User,
                content: GENERAL.to_string(),
            })
            .with(Message {
                role: Role::User,
                content: message_type.to_string(),
            });
    }
    builder.with(message);

    if let Some(user_message) = message_type.resolve_user_prompt() {
        builder.with(user_message);
    }

    Ok(builder)
}

/// A chat message
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

/// The sender of the message
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum Role {
    #[serde(rename = "system")]
    System,
    #[serde(rename = "user")]
    User,
}

/// A builder for the initial prompt
pub struct Builder<'a, P: Provider> {
    client: &'a P,
    messages: &'a RefCell<Vec<Message>>,
}

impl<'a, P: Provider> Builder<'a, P> {
    pub fn new(provider: &'a P, messages: &'a RefCell<Vec<Message>>) -> Self {
        Self {
            client: provider,
            messages,
        }
    }

    /// Append a message to the builder
    pub fn with(&mut self, message: Message) -> &mut Self {
        self.messages.borrow_mut().push(message);
        self
    }

    pub async fn request(
        &self,
        model: &str,
    ) -> anyhow::Result<impl futures_util::Stream<Item = reqwest::Result<bytes::Bytes>>> {
        self.client.request(model, self.messages).await
    }

    pub fn with_diffs(&mut self, diff_man: &DiffsManager, filename: &str) -> &mut Self {
        if diff_man.diffs.is_empty() {
            debug!("There is not differences, skipping attach them");
            return self;
        }

        let mut content = format!(
            "Here the updates of the file {}:

",
            filename
        );

        for diff in diff_man.diffs.iter() {
            content.push_str(&diff.to_string());
        }

        let message = Message {
            role: Role::User,
            content,
        };

        debug!(?message, "Attaching differences");
        self.messages.borrow_mut().push(message);
        self
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
            MessageType::Code { .. } => CODE,
            MessageType::Commit(_) => COMMIT,
            MessageType::Git(_) => GIT,
        };
        write!(f, "{}", prompt)
    }
}

impl MessageType {
    fn resolve_user_prompt(&self) -> Option<Message> {
        let prompt = match self {
            MessageType::Code { user_prompt, .. } => user_prompt,
            MessageType::Commit(user_prompt) => user_prompt,
            MessageType::Git(user_prompt) => user_prompt,
        };

        prompt.as_ref().map(|content| Message {
            role: Role::User,
            content: content.to_string(),
        })
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
        let chunk = r#"
        {"choices":[{"index":0,"delta":{"content":"Rust "}}]}
        "#;

        let provider = TestProvider::new(10, chunk);
        let mut chat = Chat::new(provider);
        let streamer = TestStreamer;
        let writer = TestWriter;

        let message = Message {
            role: Role::User,
            content: "hello".to_string(),
        };

        let response = chat
            .send_message_with_stream(
                None,
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
        let mut chat = Chat::new(provider);
        let streamer = TestStreamer;
        let writer = TestWriter;

        let message = Message {
            role: Role::User,
            content: "hello".to_string(),
        };

        chat.send_message_with_stream(
            None,
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

    #[test]
    fn save_and_load_chat() {
        let file = "/tmp";
        let provider = TestProvider::new(0, "");
        let chat1 = Chat::new(provider);

        chat1.add_message(Message {
            content: "Hello".to_string(),
            role: Role::User,
        });
        chat1.add_message(Message {
            content: "Hello, how are you?".to_string(),
            role: Role::System,
        });

        chat1.save_chat(Some(file)).expect("save the chat");

        let provider = TestProvider::new(0, "");
        let chat2 = Chat::try_load_chat(Some(file))
            .expect("load chat")
            .expect("retrieve chat")
            .with_provider(provider);

        assert_eq!(
            chat1
                .messages
                .borrow()
                .first()
                .expect("first message in chat 1")
                .content,
            chat2
                .messages
                .borrow()
                .first()
                .expect("first message in chat 2")
                .content
        );

        assert_eq!(
            chat1.messages.borrow().first().expect("first message in chat 1").role,
            chat2.messages.borrow().first().expect("first message in chat 2").role
        )
    }
}
