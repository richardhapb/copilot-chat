use std::fmt::Display;

use tokio::{fs::File, io::AsyncReadExt};

use super::reader::{Readable, ReaderTool};

/// The lines range of the file
#[derive(Debug, Clone)]
pub struct FileRange {
    start: usize,
    end: usize,
}

impl Display for FileRange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, ":{}-{}", self.start, self.end)
    }
}

impl Default for FileRange {
    fn default() -> Self {
        Self { start: 1, end: 0 }
    }
}

impl FileRange {
    pub fn from_file_arg(arg: &str) -> Option<Self> {
        if let Some((_, range)) = arg.split_once(":") {
            if let Some((start, end)) = range.split_once("-") {
                let start = start.parse().unwrap_or(1);
                // 0 means at the end of the file
                let end = end.parse().unwrap_or(0);
                Some(Self { start, end })
            } else {
                None
            }
        } else {
            None
        }
    }
}

pub struct FileReadable {
    path: String,
}

impl Readable for FileReadable {
    fn location(&self) -> &str {
        &self.path
    }
}

/// Read a file content and handle all file-related context
#[derive(Debug, Default, Clone)]
pub struct FileReader {
    path: String,
    content: String,
}

impl ReaderTool for FileReader {
    async fn read(&mut self, readable: &impl Readable) -> anyhow::Result<()> {
        let file_path = readable.location();
        let mut file = File::open(file_path).await?;

        file.read_to_string(&mut self.content).await?;

        Ok(())
    }
}

impl FileReader {
    /// Get a new `FileReader`
    #[allow(dead_code)]
    pub fn new(path: Option<String>) -> Self {
        if let Some(path) = path {
            Self {
                path,
                content: String::new(),
            }
        } else {
            Self::default()
        }
    }

    /// Get the clean file path by removing the range if it exists; if there is no range,
    /// returns the argument itself. e.g. /path/to/file:10-20 -> /path/to/file
    pub fn from_file_arg(arg: &str) -> Self {
        if let Some((path, _)) = arg.split_once(":") {
            Self {
                path: path.to_string(),
                content: String::new(),
            }
        } else {
            Self {
                path: arg.to_string(),
                content: String::new(),
            }
        }
    }

    pub fn get_readable(&self) -> FileReadable {
        FileReadable {
            path: self.path.clone(),
        }
    }

    /// Add the line number to each line
    ///
    /// Example:
    ///     ```rust
    ///     let reader = FileReader::new(Some("Hello\nWelcome to Copilot\nTell me something".to_string()));
    ///     let numered = reader.add_line_numbers();
    ///
    ///     assert_eq!(numered, "1: Hello\n2: Welcome to Copilot\n3: Tell me something\n")
    ///     ````
    pub fn add_line_numbers(&self) -> String {
        let mut new_content = String::new();
        for (i, line) in self.content.lines().enumerate() {
            let numered = format!("{}: {}\n", i + 1, line);
            new_content.push_str(&numered);
        }

        new_content
    }

    /// Prepare all the necesary data for copilot
    /// - Read the file
    /// - Add the line number for each line
    /// - Add the file name and indicate the range selected by the user
    pub async fn prepare_for_copilot(
        &mut self,
        readable: &impl Readable,
        range: Option<&FileRange>,
    ) -> anyhow::Result<String> {
        self.read(readable).await?;
        let numbered = self.add_line_numbers();
        if let Some(range) = range {
            let mut range_str = range.to_string();
            if range.end == 0 {
                range_str = range_str
                    .split_once("-")
                    .unwrap_or((&range_str, ""))
                    .0
                    .to_string();
            }
            Ok(format!(
                "File: {}{}\n\n{}",
                readable.location(),
                range_str,
                numbered
            ))
        } else {
            Ok(format!("File: {}\n\n{}", readable.location(), numbered))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;

    struct MockFile;
    impl Readable for MockFile {
        fn location(&self) -> &str {
            let temp = "/tmp/copilot-test";
            let mut file = File::create(temp).expect("create the file");

            file.write("Hello\nWelcome to Copilot\nTell me something\n".as_bytes())
                .expect("write to the file");

            temp
        }
    }

    #[tokio::test]
    async fn numbered_lines() {
        let mut reader = FileReader::new(None);
        let readable = MockFile;
        reader.read(&readable).await.expect("read the file");
        let numbered = reader.add_line_numbers();

        assert_eq!(
            numbered,
            "1: Hello\n2: Welcome to Copilot\n3: Tell me something\n"
        )
    }

    #[test]
    fn extract_range() {
        let range = FileRange::from_file_arg("/path/to/file:20-30");

        assert!(range.is_some());
        let range = range.expect("valid range");
        assert_eq!(range.start, 20);
        assert_eq!(range.end, 30);
    }

    #[tokio::test]
    async fn prepare() {
        let readable = MockFile;
        let mut reader = FileReader::new(None);
        reader.read(&readable).await.expect("read the file");
        let range = FileRange::from_file_arg(&format!("{}:1-2", readable.location()));
        let prepared = reader
            .prepare_for_copilot(&readable, range.as_ref())
            .await
            .expect("prepare the request");

        assert_eq!(
            prepared,
            "File: /tmp/copilot-test:1-2\n\n1: Hello\n2: Welcome to Copilot\n3: Tell me something\n4: Hello\n5: Welcome to Copilot\n6: Tell me something\n"
        )
    }
}
