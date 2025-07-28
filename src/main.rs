use chat::ChatStreamer;
use clap::Parser;
use cli::commands::Cli;
use std::io::{self, Read};
use tools::cli::CliExecutor;
use tracing::debug;
use tracing_subscriber::prelude::__tracing_subscriber_SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

use crate::cli::{
    commands::Commands,
    handlers::{CommandHandler, ExecutionType},
};

mod chat;
mod cli;
mod client;
mod tools;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_logging()?;
    let cli = Cli::parse();

    // Dependencies
    let auth = client::auth::CopilotAuth::new();
    let client = client::CopilotClient::new(auth);
    let streamer = ChatStreamer;
    let mut stdin_str = String::new();
    let is_tcp = matches!(cli.command, Some(Commands::Tcp { port: _ }));

    // Read only from piped stdin
    if !atty::is(atty::Stream::Stdin) && !is_tcp {
        debug!("Reading from stdin");
        // STDIN
        let stdin = io::stdin();
        let mut stdin_buf = vec![];
        stdin.lock().read_to_end(&mut stdin_buf)?;
        stdin_str = String::from_utf8_lossy(&stdin_buf).to_string();
    }

    debug! {%stdin_str, "Received"};

    // Parse the user prompt from CLI if exist - clone to avoid partial move
    let user_prompt = cli.prompt.as_ref().map(|prompt| prompt.join(" "));

    debug!(?user_prompt);

    // Resolve the commit stdin if it exists.
    if matches!(cli.command, Some(Commands::Commit)) {
        if stdin_str.is_empty() {
            stdin_str = CliExecutor::new().execute("git", &["diff", "--staged"]).await?;

            if stdin_str.is_empty() {
                eprintln!("Git diff is empty. Ensure you are in a repository and that the changes are staged.");
                std::process::exit(1);
            }
        }
    }

    let mut handler = CommandHandler::new(&cli, user_prompt.as_deref());
    let mut attr = handler.prepare(client).await?;

    if attr.execution_type == ExecutionType::Exit {
        std::process::exit(0);
    }

    let writer = tokio::io::stdout();

    match attr.execution_type {
        ExecutionType::Once => {
            if let Err(e) = attr
                .process_request(&cli, streamer.clone(), writer, Some(stdin_str))
                .await
            {
                eprintln!("Error: {}", e);
            }
        }
        ExecutionType::Interactive | ExecutionType::Pipe => {
            if let Err(e) = attr.process_loop(&cli, &streamer, writer, stdin_str).await {
                eprintln!("Error: {}", e);
            }
        }
        ExecutionType::Exit => {
            std::process::exit(0);
        }
    }

    Ok(())
}

fn init_logging() -> std::io::Result<()> {
    let file = std::fs::File::create("/tmp/copilot-chat.log")?;
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "copilot_chat=info".into()),
        ))
        .with(tracing_subscriber::fmt::layer().with_writer(file))
        .init();
    Ok(())
}
