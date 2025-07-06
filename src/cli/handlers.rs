use crate::{
    chat::{Chat, ChatStreamer, Message, MessageType, Role},
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

        // Get the message and chat; if the command is `commit`, a new chat is always used.
        let (message_type, chat) = match self.cli_command.command {
            Some(Commands::Commit) => {
                // If there is not a stdin, try to get the git diff
                // in the current directory
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
            // Default
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
    ) -> anyhow::Result<()> {
        // First request
        debug!("Processing first message");
        self.process_request(cli, streamer.clone(), writer, stdin_str).await?;

        let tracked_paths = self.chat.tracked_paths();
        // Attach the files without range
        let message_type = MessageType::Code {
            user_prompt: None,
            files: Some(tracked_paths.into_iter().map(|p| p.to_string()).collect()),
        };
        self.message_type = message_type;

        let mut stdout = std::io::stdout();

        loop {
            let mut read_str = String::new();

            debug!("Capturing new message");
            if atty::is(atty::Stream::Stdin) {
                // For interactive mode, use blocking stdin.
                let stdin = std::io::stdin();
                println!();
                println!();
                print!("> ");
                stdout.flush()?;

                debug!("Reading from interactive mode");
                stdin.read_line(&mut read_str)?;
            } else {
                // For non-interactive use, prefer using a socket because stdin may have been closed
                // by the client, which can make re-attaching tricky.
                let req = read_from_socket().await?;

                // Separate the prompt to to prompt acoording to protocol
                // <file>:<range>@prompt
                read_str = req.prompt;
                self.message_type = MessageType::Code {
                    user_prompt: None,
                    files: req.files,
                }
            }

            debug!(%read_str, "New user message");
            if read_str == "exit" {
                break;
            }

            let writer = tokio::io::stdout();
            self.process_request(cli, streamer.clone(), writer, read_str.trim().to_string())
                .await?;
            self.chat.save_chat(None)?;
        }
        Ok(())
    }

    pub async fn process_request(
        &mut self,
        cli: &Cli,
        streamer: ChatStreamer,
        writer: tokio::io::Stdout,
        stdin_str: String,
    ) -> anyhow::Result<()> {
        let message = Message {
            role: Role::User,
            content: stdin_str,
        };

        debug!(?self.message_type, "User message");

        // Send with stream by default, maybe in the future a buffered
        // response can be returned if it is configured
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

/// Separate the prompt to to prompt acoording to protocol
/// <file>:<range>@prompt
struct RequestProtocol {
    prompt: String,
    files: Option<Vec<String>>,
}

impl RequestProtocol {
    fn from_input(raw_input: &str) -> Self {
        let splitted = raw_input.split_once("@");
        let (file_str, prompt) = if let Some(splited) = splitted {
            splited
        } else {
            ("", raw_input)
        };

        // TODO: Handle multiples files
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
    let mut input = String::new();

    // TODO: Make this dynamic
    let tcp = TcpListener::bind("127.0.0.1:4000").await?;
    info!("Listening on 127.0.0.1:4000");

    let (mut connection, addr) = tcp.accept().await?;

    info!(%addr, "Connection received");

    while input.is_empty() {
        connection.read_to_string(&mut input).await?;
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }

    debug!(%input, "Received");

    Ok(RequestProtocol::from_input(&input))
}
