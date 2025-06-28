use tokio::process::Command;

/// Execute and handle command line executions
pub struct CliExecutor;

impl CliExecutor {
    /// A new Executor instance
    pub fn new() -> Self {
        Self
    }

    /// Execute a CLI command and returns the output
    pub async fn execute(&self, command: &str, args: &[&str]) -> anyhow::Result<String> {
        let output = Command::new(command).args(args).output().await?;
        if !output.status.success() {
            return Err(anyhow::anyhow!("Error executing command"));
        }

        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }
}
