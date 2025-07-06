use clap::{Parser, Subcommand};

/// Application that provides Copilot Chat in the CLI, offering amazing speed and maximum flexibility.
#[derive(Parser, Debug)]
#[command(name="copilot-chat", version, about, long_about = None, author="richardhapb")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,

    /// File path for being read by Copilot
    #[arg(short, long)]
    pub files: Option<Vec<String>>,

    /// Token path
    #[arg(short, long)]
    token_path: Option<String>,

    /// Prompt to send to Copilot
    #[arg(trailing_var_arg = true, global = true)]
    pub prompt: Option<Vec<String>>,

    /// Prompt to send to Copilot
    #[arg(short, long, global = true)]
    pub model: Option<String>,
}

#[derive(Debug, Subcommand, PartialEq)]
pub enum Commands {
    /// Write the commit message for the current directory
    Commit,
    /// List all the available models
    Models,
    /// Clear the chat history for the current directory
    Clear,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_prompt_args() {
        let req = vec!["copilot-chat", "hello", "there,", "tell", "me", "something"];
        let cli = Cli::parse_from(req);

        assert!(cli.command.is_none());
        assert!(cli.prompt.is_some());

        assert_eq!(
            cli.prompt.expect("prompt args"),
            vec!["hello", "there,", "tell", "me", "something"]
        );
    }

    #[test]
    fn test_prompt_commit_args() {
        let req = vec!["copilot-chat", "commit", "write", "a", "cool", "message"];
        let cli = Cli::parse_from(req);

        assert_eq!(cli.command.expect("commit command"), Commands::Commit);
        assert!(cli.prompt.is_some());

        assert_eq!(cli.prompt.expect("prompt args"), vec!["write", "a", "cool", "message"]);
    }
}
