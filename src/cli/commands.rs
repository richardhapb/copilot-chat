use clap::{Parser, Subcommand};

/// Application that provides Copilot Chat in the CLI, offering amazing speed and maximum flexibility.
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,
    /// Token path
    #[arg(short, long)]
    token_path: Option<String>,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Write the commit message for the current directory
    Commit,
}
