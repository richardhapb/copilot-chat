use crate::{
    chat::{Chat, ChatStreamer, Message, MessageType, Role, errors::ChatError},
    cli::commands::{Cli, Command},
    client::{CopilotClient, provider::Provider},
};
use std::path::PathBuf;
use std::{fs::read_dir, io::Write};
use tokio::{io::AsyncReadExt, net::TcpListener};
use tracing::{debug, info, warn};

#[derive(Debug, PartialEq)]
#[allow(dead_code)]
pub enum ExecutionType {
    Once,
    Interactive,
    Pipe,
    Exit,
}

impl From<&Command> for ExecutionType {
    fn from(value: &Command) -> Self {
        match value {
            Command::Tcp { port: _ } => ExecutionType::Interactive,
            Command::Commit => ExecutionType::Once,
            Command::Models | Command::Clear => ExecutionType::Exit,
        }
    }
}

pub struct CommandHandler<'a> {
    pub cli_command: &'a Cli,
    pub user_prompt: Option<&'a str>,
}

impl<'a> CommandHandler<'a> {
    pub fn new(cli_command: &'a Cli, user_prompt: Option<&'a str>) -> Self {
        Self {
            cli_command,
            user_prompt,
        }
    }

    pub async fn prepare(&mut self, client: CopilotClient) -> anyhow::Result<ExecutionHandler> {
        let mut is_tcp = false;
        let mut final_port = "4000";

        match &self.cli_command.command {
            Some(Command::Models) => {
                client.get_models().await?;
            }
            Some(Command::Clear) => match Chat::<CopilotClient>::try_load_chat(None)? {
                Some(chat) => {
                    chat.remove_chat(None)?;
                    println!("Chat cleared successfully");
                }
                None => {
                    println!("Chat not found; skipping clearing.");
                }
            },
            Some(Command::Tcp { port }) => {
                if let Some(port) = port {
                    final_port = port
                }
                is_tcp = true;
            }
            Some(Command::Commit) | None => {}
        };

        let chat = self.resolve_chat(client);
        let message_type = MessageType::from(&*self);
        let execution_type = if let Some(command) = &self.cli_command.command {
            ExecutionType::from(command)
        } else {
            ExecutionType::Exit
        };

        debug!(?message_type, "Received");

        Ok(ExecutionHandler {
            chat,
            message_type,
            execution_type,
            is_tcp,
            port: final_port.to_string(),
        })
    }

    /// Expand the operator `*` to retrieve all the files inside the current directory that match
    /// with the extension if any, for example: `*.rs` expanded to all Rust source code inside this
    /// directory and child directories. Also exclude all the file or directory names that match
    /// with any of the `exclude` vector
    pub fn expand_files_from_dir(
        cwd: &PathBuf,
        files: Option<&Vec<String>>,
        exclude: Option<&Vec<String>>,
    ) -> std::io::Result<Option<Vec<String>>> {
        if let Some(files) = files {
            let mut files_result: Vec<String> = vec![];
            for file in files {
                if file.contains("*") {
                    // TODO: This handles `*` if it does not have an extension?
                    let ext = file.strip_prefix("*.").unwrap_or("");
                    files_result.append(&mut Self::find_files_with_ext(cwd.clone(), ext, files, exclude)?);
                } else {
                    files_result.push(file.to_string())
                }
            }
            Ok(Some(files_result))
        } else {
            Ok(None)
        }
    }

    /// Walk through the directories recursively and look for all files that match the pattern
    /// also exlude the files or directories that match with any element in `exlude`
    fn find_files_with_ext(
        dir: PathBuf,
        ext: &str,
        files: &Vec<String>,
        exclude: Option<&Vec<String>>,
    ) -> std::io::Result<Vec<String>> {
        let elements = read_dir(dir)?;
        let mut files_result: Vec<String> = vec![];

        for element in elements {
            let element = element?;
            let metadata = element.metadata()?;

            if let Some(name) = element.file_name().to_str() {
                if let Some(exclude) = exclude
                    && exclude.iter().any(|ex| ex == name)
                {
                    continue;
                }
            }

            if metadata.is_dir() {
                let mut files_inner = Self::find_files_with_ext(element.path(), ext, files, exclude)?;
                files_result.append(&mut files_inner);
            } else if metadata.is_file() {
                // TODO: Enhance this
                let path = element.path().to_str().unwrap_or("").to_string();
                if path.ends_with(&format!(".{}", ext)) && !files.contains(&path) {
                    files_result.push(path);
                }
            }
        }

        Ok(files_result)
    }

    fn resolve_chat(&self, client: CopilotClient) -> Chat<CopilotClient> {
        match self.cli_command.command {
            Some(Command::Commit) => Chat::new(client),
            Some(Command::Tcp { port: _ }) | None => match Chat::try_load_chat(None).unwrap_or_else(|e| {
                warn!("Chat cannot be loaded: {e}");
                None
            }) {
                Some(chat) => chat.with_provider(client),
                None => Chat::new(client),
            },
            Some(Command::Models | Command::Clear) => Chat::new(CopilotClient::default()),
        }
    }
}

#[derive(Debug)]
pub struct ExecutionHandler {
    pub chat: Chat<CopilotClient>,
    pub message_type: MessageType,
    pub execution_type: ExecutionType,
    pub is_tcp: bool,
    pub port: String,
}

impl ExecutionHandler {
    pub async fn process_loop(
        &mut self,
        cli: &Cli,
        streamer: &ChatStreamer,
        writer: tokio::io::Stdout,
        stdin_str: String,
    ) -> Result<(), ChatError> {
        let stdin_str = if !stdin_str.is_empty() { Some(stdin_str) } else { None };

        // Process the first request directly if it is not a TCP request.
        if !self.is_tcp {
            debug!("Processing first message");
            self.process_request(cli, streamer.clone(), writer, stdin_str).await?;
            self.chat.save_chat(None)?;
            self.message_type.clear_user_prompt();
        }

        let mut stdout = std::io::stdout();

        // Main interaction loop
        loop {
            debug!("Capturing new message");

            let req = if self.is_tcp {
                // TCP mode - receive request over socket
                read_from_socket(&self.port)
                    .await
                    .map_err(|e| ChatError::Request(e.to_string()))?
            } else {
                print!("\n\n> ");
                stdout.flush().map_err(ChatError::Cache)?;

                read_from_stdin().await.map_err(|e| ChatError::Request(e.to_string()))?
            };

            if req.prompt.trim() == "exit" {
                break;
            }

            self.message_type = MessageType::Code {
                user_prompt: Some(req.prompt.trim().to_string()),
                files: req.files,
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
        let message = if let Some(stdin_str) = stdin_str {
            let message = Message {
                role: Role::User,
                content: stdin_str,
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

#[derive(Default)]
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

async fn read_from_stdin() -> anyhow::Result<RequestProtocol> {
    let mut read_str = String::new();
    let stdin = std::io::stdin();

    debug!("Reading from interactive mode");
    stdin.read_line(&mut read_str).map_err(ChatError::Cache)?;

    Ok(RequestProtocol::from_input(&read_str))
}

async fn read_from_socket(port: &str) -> anyhow::Result<RequestProtocol> {
    let bind = format!("127.0.0.1:{}", port);
    let tcp = TcpListener::bind(&bind).await?;
    info!("Listening on {}", bind);
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

#[cfg(test)]
mod tests {
    use clap::Parser;
    use std::fs;
    use tempfile::tempdir;

    use super::*;

    // Test the usage of the `*.rs` pattern in the files argument.
    #[test]
    fn expand_files() {
        let mut cli = Cli::parse();
        let temp = tempdir().expect("create temp dir");
        let dir = temp.path();
        let mut expected_files = vec![];

        for d in vec!["subdir1", "subdir2"].iter() {
            let subdir = dir.join(d);
            fs::create_dir_all(&subdir).expect("create dir");
            for f in vec!["file1.rs", "file2.rs"].iter() {
                let file = subdir.join(f);
                fs::File::create(&file).expect("create file");

                fs::File::create(subdir.join(format!("{}_ignored.go", f.to_string()))).expect("create file");
                expected_files.push(file);
            }
            fs::File::create(subdir.join("should_be_ignored.py")).expect("create file");
            // Ensure that finalized files in `rs` are ignored in favor of `.rs`
            fs::File::create(subdir.join("randomrs")).expect("create file");
        }

        let root_file = dir.join("root_file.rs");
        fs::File::create(&root_file).expect("create file");

        expected_files.push(root_file);

        // This should be ignored
        fs::File::create(dir.join("ignored.rs")).expect("create file");
        fs::File::create(dir.join("another.go")).expect("create file");

        cli.files = Some(vec!["*.rs".into()]);
        cli.exclude = Some(vec!["ignored.rs".into()]);

        let result =
            CommandHandler::expand_files_from_dir(&dir.to_path_buf(), cli.files.as_ref(), cli.exclude.as_ref())
                .unwrap();
        let mut expected = expected_files
            .iter()
            .map(|f| f.to_str().expect("convert to str").to_string())
            .collect::<Vec<_>>();

        let mut result = result.unwrap();
        result.sort();
        expected.sort();

        assert_eq!(result, expected)
    }
}
