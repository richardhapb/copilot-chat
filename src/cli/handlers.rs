use crate::{
    chat::{Chat, ChatStreamer, Message, MessageType, Role, errors::ChatError},
    cli::commands::{Cli, Commands},
    client::{CopilotClient, provider::Provider},
    tools::cli::CliExecutor,
};
use std::io::Write;
use tokio::{io::AsyncReadExt, net::TcpListener};
use tracing::{debug, info};

#[derive(Debug, PartialEq)]
#[allow(dead_code)]
pub enum ExecutionType {
    Once,
    Interactive,
    Pipe,
    Exit,
}

pub struct CommandHandler<'a> {
    cli_command: &'a Cli,
    user_prompt: Option<&'a str>,
}

impl<'a> CommandHandler<'a> {
    pub fn new(cli_command: &'a Cli, user_prompt: Option<&'a str>) -> Self {
        Self {
            cli_command,
            user_prompt,
        }
    }

    pub async fn prepare(
        &mut self,
        client: CopilotClient,
        stdin_str: &mut String,
    ) -> anyhow::Result<ExecutionAttributes> {
        let mut execution_type = ExecutionType::Exit;

        let (message_type, chat) = match self.cli_command.command {
            Some(Commands::Commit) => {
                if stdin_str.is_empty() {
                    *stdin_str = CliExecutor::new().execute("git", &["diff", "--staged"]).await?;

                    if stdin_str.is_empty() {
                        eprintln!("Git diff is empty. Ensure you are in a repository and that the changes are staged.");
                        std::process::exit(1);
                    }
                }

                execution_type = ExecutionType::Once;
                (
                    Some(MessageType::Commit(self.user_prompt.map(|p| p.to_string()))),
                    Some(Chat::new(client)),
                )
            }
            Some(Commands::Models) => {
                client.get_models().await?;
                (None, None)
            }
            Some(Commands::Clear) => match Chat::<CopilotClient>::try_load_chat(None)? {
                Some(chat) => {
                    chat.remove_chat(None)?;
                    println!("Chat cleared successfully");

                    (None, None)
                }
                None => {
                    println!("Chat not found; skipping clearing.");
                    (None, None)
                }
            },
            None => {
                execution_type = ExecutionType::Interactive;
                (
                    Some(MessageType::Code {
                        user_prompt: self.user_prompt.map(|p| p.to_string()),
                        files: self.cli_command.files.clone(),
                    }),
                    Some(match Chat::try_load_chat(None)? {
                        Some(chat) => chat.with_provider(client),
                        None => Chat::new(client),
                    }),
                )
            }
        };

        let chat = chat.unwrap_or(Chat::new(CopilotClient::default()));
        debug!(?message_type, "Received");
        let message_type = message_type.unwrap_or(MessageType::Commit(None));

        Ok(ExecutionAttributes {
            chat,
            message_type,
            execution_type,
        })
    }
}

#[derive(Debug)]
pub struct ExecutionAttributes {
    pub chat: Chat<CopilotClient>,
    pub message_type: MessageType,
    pub execution_type: ExecutionType,
}

impl ExecutionAttributes {
    pub async fn process_loop(
        &mut self,
        cli: &Cli,
        streamer: &ChatStreamer,
        writer: tokio::io::Stdout,
        stdin_str: String,
    ) -> Result<(), ChatError> {
        debug!("Processing first message");
        let stdin_str = if !stdin_str.is_empty() { Some(stdin_str) } else { None };

        self.process_request(cli, streamer.clone(), writer, stdin_str).await?;
        self.message_type.clear_user_prompt();

        let mut stdout = std::io::stdout();

        loop {
            debug!("Capturing new message");

            self.message_type = if atty::is(atty::Stream::Stdin) {
                let mut read_str = String::new();
                let stdin = std::io::stdin();
                print!("\n\n> ");
                stdout.flush().map_err(|e| ChatError::CacheError(e))?;

                let files = match &mut self.message_type {
                    MessageType::Code { user_prompt: _, files } => files.take(),
                    _ => None,
                };

                debug!("Reading from interactive mode");
                stdin.read_line(&mut read_str).map_err(|e| ChatError::CacheError(e))?;

                if read_str.trim() == "exit" {
                    break;
                }

                MessageType::Code {
                    user_prompt: Some(read_str.trim().to_string()),
                    files,
                }
            } else {
                let req = read_from_socket()
                    .await
                    .map_err(|e| ChatError::RequestError(e.to_string()))?;

                if req.prompt.trim() == "exit" {
                    break;
                }

                MessageType::Code {
                    user_prompt: Some(req.prompt.trim().to_string()),
                    files: req.files,
                }
            };

            let writer = tokio::io::stdout();
            self.process_request(cli, streamer.clone(), writer, None).await?;
            self.chat.save_chat(None)?;
        }
        Ok(())
    }

    pub async fn process_request(
        &mut self,
        cli: &Cli,
        streamer: ChatStreamer,
        writer: tokio::io::Stdout,
        stdin_str: Option<String>,
    ) -> Result<(), ChatError> {
        let message = if stdin_str.is_some() {
            let message = Message {
                role: Role::User,
                content: stdin_str.unwrap(),
            };
            Some(message)
        } else {
            None
        };

        debug!(?self.message_type, "User message");

        let response_message = self
            .chat
            .send_message_with_stream(
                cli.model.as_deref(),
                message,
                self.message_type.clone(),
                streamer.clone(),
                writer,
            )
            .await?;
        self.chat.add_message(response_message);

        Ok(())
    }
}

struct RequestProtocol {
    prompt: String,
    files: Option<Vec<String>>,
}

impl RequestProtocol {
    fn from_input(raw_input: &str) -> Self {
        let (file_str, prompt) = match raw_input.split_once('@') {
            Some(split) => split,
            None => ("", raw_input),
        };
        let files = if file_str.is_empty() {
            None
        } else {
            Some(vec![file_str.to_string()])
        };
        Self {
            prompt: prompt.to_string(),
            files,
        }
    }
}

async fn read_from_socket() -> anyhow::Result<RequestProtocol> {
    let tcp = TcpListener::bind("127.0.0.1:4000").await?;
    info!("Listening on 127.0.0.1:4000");
    let (mut connection, addr) = tcp.accept().await?;
    info!(%addr, "Connection received");
    let mut input = String::new();
    let mut buffer = [0u8; 1024];

    let start = std::time::Instant::now();
    let timeout = std::time::Duration::from_secs(5);

    while input.is_empty() && start.elapsed() < timeout {
        match connection.read(&mut buffer).await {
            Ok(0) => break,
            Ok(n) => input.push_str(std::str::from_utf8(&buffer[..n])?),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                continue;
            }
            Err(e) => return Err(e.into()),
        }

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    if input.is_empty() {
        return Err(anyhow::anyhow!("No data received within timeout"));
    }
    debug!(%input, "Received");
    Ok(RequestProtocol::from_input(&input))
}
