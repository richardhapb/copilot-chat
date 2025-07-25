# Copilot Chat CLI

## Overview

Copilot Chat CLI is a command-line application designed to provide seamless interaction with GitHub Copilot models. It offers a fast, flexible, and interactive way to communicate with Copilot, enabling users to send prompts, process files, and manage chat history directly from the terminal.

This project is built with Rust and leverages asynchronous programming for high performance. It includes features such as file tracking, diff computation, and interactive chat sessions, making it ideal for developers who want to integrate AI-powered assistance into their workflows.

---

## Features

- **Interactive Chat**: Engage in real-time conversations with Copilot using the CLI.
- **File Tracking**: Send specific file content or ranges to Copilot for analysis.
- **Diff Computation**: Automatically compute and send differences between file versions to avoid sending duplicate file information.
- **Commit Message Generation**: Generate commit messages based on staged changes in your Git repository.
- **Model Management**: List available Copilot models and switch between them.
- **Chat History Management**: Save, load, and clear chat history for the current directory.
- **Socket-Based Input**: Accept input via TCP sockets for advanced integrations.

---

## Installation

### Prerequisites

- Rust (latest stable version)
- Cargo (Rust's package manager)
- GitHub Copilot API credentials

### Steps

1. Clone the repository:
   ```bash
   git clone https://github.com/richardhapb/copilot-chat.git
   cd copilot-chat
   ```

2. Build the project:
   ```bash
   cargo build --release
   ```

3. Run the binary:
   ```bash
   ./target/release/copilot-chat
   ```

4. Copy to PATH (Unix):
   ```bash
   sudo ln -sf $(pwd)/target/release/copilot-chat /usr/local/bin
   ```

---

## Usage

### Basic Commands

#### Interactive Mode
Start an interactive chat session:
```bash
copilot-chat
```

#### Send a Prompt
Send a one-time prompt to Copilot:
```bash
copilot-chat --prompt "Tell me a joke"
```

Or use piping directly
```bash
cat myfile.rs | copilot-chat give me feedback on this file
```

#### File Analysis
Send a file or file range to Copilot:
```bash
copilot-chat --files "/path/to/file:10-20,/path/to/another:30-50,/path/to/some"
```

#### Generate Commit Message
Generate a commit message based on staged changes:
```bash
copilot-chat commit
```

#### List Models
List all available Copilot models:
```bash
copilot-chat models
```

#### Clear Chat History
Clear the saved chat history for the current directory:
```bash
copilot-chat clear
```

---

### Advanced Features

#### Socket-Based Input
Send input via TCP socket:
1. Start the CLI in interactive mode.
2. Connect to the socket at `127.0.0.1:4000` and send data.

#### Custom Model Selection
Specify a model for Copilot:
```bash
copilot-chat --model "gpt-4o"
```
---

## Development

### Run Tests
Execute the test suite:
```bash
cargo test
```

### Code Structure
- **`src/main.rs`**: Entry point for the application.
- **`src/chat`**: Core chat logic and streaming implementation.
- **`src/tools`**: Utilities for file handling, diff computation, and CLI execution.
- **`src/cli`**: Command-line interface and subcommand handling.
- **`src/client`**: Copilot API client and authentication.

---

## Contributing

Contributions are welcome! Please follow these steps:

1. Fork the repository.
2. Create a feature branch.
3. Submit a pull request with a detailed description of your changes.

---

## License

This project is licensed under the MIT License.
