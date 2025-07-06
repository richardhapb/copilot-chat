use crate::{
    chat::{Chat, ChatStreamer, Message, MessageType, Role},
    cli::commands::{Cli, Commands},
    client::{CopilotClient, provider::Provider},
    tools::cli::CliExecutor,
};
use std::io::{Read, Write};
use tracing::debug;

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
    stdin_str: String,
    user_prompt: Option<&'a str>,
}

impl<'a> CommandHandler<'a> {
    pub fn new(cli_command: &'a Cli, stdin_str: String, user_prompt: Option<&'a str>) -> Self {
        Self {
            cli_command,
            stdin_str,
            user_prompt,
        }
    }

    pub async fn prepare(&mut self, client: CopilotClient) -> anyhow::Result<ExecutionAttributes> {
        let mut execution_type = ExecutionType::Exit;

        // Get the message and chat; if the command is `commit`, a new chat is always used.
        let (message_type, chat) = match self.cli_command.command {
            Some(Commands::Commit) => {
                // If there is not a stdin, try to get the git diff
                // in the current directory
                if self.stdin_str.is_empty() {
                    self.stdin_str = CliExecutor::new().execute("git", &["diff", "--staged"]).await?;

                    if self.stdin_str.is_empty() {
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
            None => (
                Some(MessageType::Code {
                    user_prompt: self.user_prompt.map(|p| p.to_string()),
                    files: self.cli_command.files.clone(),
                }),
                Some(match Chat::try_load_chat(None)? {
                    Some(chat) => chat.with_provider(client),
                    None => Chat::new(client),
                }),
            ),
        };

        let chat = chat.unwrap_or(Chat::new(CopilotClient::default()));
        let message_type = message_type.unwrap_or(MessageType::Commit(None));

        Ok(ExecutionAttributes {
            chat,
            message_type,
            execution_type,
            stdin_str: self.stdin_str.clone(),
        })
    }
}

#[derive(Debug)]
pub struct ExecutionAttributes {
    pub chat: Chat<CopilotClient>,
    pub message_type: MessageType,
    pub execution_type: ExecutionType,
    pub stdin_str: String,
}

impl ExecutionAttributes {
    pub async fn process_loop(
        &mut self,
        cli: &Cli,
        streamer: &ChatStreamer,
        writer: tokio::io::Stdout,
    ) -> anyhow::Result<()> {
        // First request
        debug!("Processing first message");
        self.process_request(cli, streamer.clone(), writer).await?;

        let mut stdout = std::io::stdout();

        loop {
            let writer = tokio::io::stdout();
            debug!("Capturing new message");
            println!();
            println!();
            print!("> ");
            stdout.flush()?;

            let stdin = std::io::stdin();
            let mut read_str = String::new();

            if !atty::is(atty::Stream::Stdin) {
                debug!("Reading from stdin piped");
                // STDIN
                stdin.lock().read_to_string(&mut read_str)?;
            } else {
                debug!("Reading from interactive mode");
                stdin.read_line(&mut read_str)?;
            }

            debug!(%read_str, "New user message");
            if read_str == "exit" {
                break;
            }

            self.process_request(cli, streamer.clone(), writer).await?;
        }

        Ok(())
    }

    pub async fn process_request(
        &mut self,
        cli: &Cli,
        streamer: ChatStreamer,
        writer: tokio::io::Stdout,
    ) -> anyhow::Result<()> {
        let message = Message {
            role: Role::User,
            content: self.stdin_str.clone(),
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
