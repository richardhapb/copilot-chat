use chrono::Local;
use std::fmt::Display;

use super::reader::{Readable, ReaderTool};
use serde::{Deserialize, Serialize};

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

impl Readable for TrackedFile {
    fn location(&self) -> &str {
        &self.path
    }

    fn content(&self) -> &str {
        &self.content
    }

    fn modified_time(&self) -> &chrono::DateTime<Local> {
        &self.last_modification
    }

    fn set_content(&mut self, content: String) {
        self.content = content
    }
}

/// Read a file content and handle all file-related context
#[derive(Debug, Deserialize, Serialize, PartialEq, Default)]
pub struct TrackedFile {
    pub path: String,
    content: String,
    last_modification: chrono::DateTime<Local>,
}

pub struct FileReader;

impl ReaderTool for FileReader {
    async fn read<'a>(&self, readable: &'a mut impl Readable) -> anyhow::Result<&'a str> {
        let file_path = readable.location();
        let content = std::fs::read_to_string(file_path)?;

        readable.set_content(content);

        Ok(readable.content())
    }
}

impl TrackedFile {
    /// Get a new `FileReader`
    #[allow(dead_code)]
    pub fn new(path: Option<String>) -> Self {
        if let Some(path) = path {
            Self {
                path,
                content: String::new(),
                last_modification: chrono::Local::now(),
            }
        } else {
            Self::default()
        }
    }

    /// Get the clean file path by removing the range if it exists; if there is no range,
    /// returns the argument itself. e.g. /path/to/file:10-20 -> /path/to/file
    pub fn from_file_arg(arg: &str) -> Self {
        let last_modification = chrono::Local::now();
        if let Some((path, _)) = arg.split_once(":") {
            Self {
                path: path.to_string(),
                content: String::new(),
                last_modification,
            }
        } else {
            Self {
                path: arg.to_string(),
                content: String::new(),
                last_modification,
            }
        }
    }

    /// Prepare all the necesary data for copilot
    /// - Read the file
    /// - Add the line number for each line
    /// - Add the file name and indicate the range selected by the user
    pub async fn prepare_load_once(&self) -> anyhow::Result<String> {
        let numbered = self.add_line_numbers();
        Ok(format!("File: {} [load-once]\n\n{}", self.path, numbered))
    }

    /// Prepare the necesary data for copilot
    /// - Add the file name and indicate the range selected by the user
    pub async fn prepare_for_copilot(
        &mut self,
        range: Option<&FileRange>,
    ) -> anyhow::Result<String> {
        if let Some(range) = range {
            let mut range_str = range.to_string();
            if range.end == 0 {
                range_str = range_str
                    .split_once("-")
                    .unwrap_or((&range_str, ""))
                    .0
                    .to_string();
            }
            Ok(format!("File: {}{}", self.path, range_str,))
        } else {
            Ok(format!("File: {}", self.path))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;

    #[derive(Default)]
    struct MockFile {
        content: String,
        _timemod: chrono::DateTime<Local>,
    }

    impl Readable for MockFile {
        fn location(&self) -> &str {
            let temp = "/tmp/copilot-test";
            let mut file = File::create(temp).expect("create the file");

            file.write("Hello\nWelcome to Copilot\nTell me something\n".as_bytes())
                .expect("write to the file");

            temp
        }

        fn set_content(&mut self, content: String) {
            self.content = content
        }

        fn modified_time(&self) -> &chrono::DateTime<Local> {
            &self._timemod
        }

        fn content<'a>(&'a self) -> &'a str {
            &self.content
        }
    }

    #[tokio::test]
    async fn numbered_lines() {
        let reader = FileReader;
        let mut readable = MockFile::default();
        let _ = reader.read(&mut readable).await.expect("read the file");
        let numbered = readable.add_line_numbers();

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
    async fn prepare_once() {
        let mut readable = MockFile::default();
        let mut file_tracked = TrackedFile::new(None);
        let reader = FileReader;
        reader.read(&mut readable).await.expect("read the file");

        file_tracked.set_content(readable.content.clone());
        file_tracked.path = readable.location().into();

        let prepared = file_tracked
            .prepare_load_once()
            .await
            .expect("prepare the request");

        assert_eq!(
            prepared,
            "File: /tmp/copilot-test [load-once]\n\n1: Hello\n2: Welcome to Copilot\n3: Tell me something\n"
        )
    }

    #[tokio::test]
    async fn prepare_copilot() {
        let mut readable = MockFile::default();
        let mut file_tracked = TrackedFile::new(None);
        let reader = FileReader;

        reader.read(&mut readable).await.expect("read the file");

        file_tracked.content = readable.content().into();
        file_tracked.path = readable.location().into();

        let range = FileRange::from_file_arg(&format!("{}:1-2", readable.location()));
        let prepared = file_tracked
            .prepare_for_copilot(range.as_ref())
            .await
            .expect("prepare the request");

        assert_eq!(
            prepared,
            "File: /tmp/copilot-test:1-2"
        )
    }
}
