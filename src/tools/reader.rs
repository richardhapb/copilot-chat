use super::diff::LineSequence;
use std::time::SystemTime;

use crate::tools::diff::DiffsManager;

/// Any readable resource
pub trait Readable: std::fmt::Debug {
    /// Returns the location of the readable
    /// e.g. An absolute file path
    fn location(&self) -> &str;
    fn modified_time(&self) -> &SystemTime;
    fn set_modified_time(&mut self, new_time: SystemTime);
    fn set_content(&mut self, content: String);
    fn content(&self) -> &str;

    /// Add the line number to each line
    ///
    /// Example:
    ///     ```rust
    ///     let reader = TrackedFile::new(Some("Hello\nWelcome to Copilot\nTell me something".to_string()));
    ///     let numered = reader.add_line_numbers();
    ///
    ///     assert_eq!(numered, "1: Hello\n2: Welcome to Copilot\n3: Tell me something\n")
    ///     ````
    fn add_line_numbers(&self) -> String {
        let mut new_content = String::new();
        for (i, line) in self.content().lines().enumerate() {
            let numered = format!("{}: {}\n", i + 1, line);
            new_content.push_str(&numered);
        }

        new_content
    }
}

/// Read a resource
pub trait ReaderTool {
    /// Read the content of a readable
    async fn read<'a>(&self, readable: &'a mut impl Readable) -> anyhow::Result<&'a str>;

    /// Calculate the line-level difference between the in-memory content of a [`Readable`] and the
    /// corresponding file on the filesystem, but only if the file's modified timestamp is more recent
    /// than the last known modification of the [`Readable`].
    ///
    /// The diff is computed using **Myers' Algorithm**, which has a worst-case time complexity of
    /// O(ND), where:
    /// - **N** is the total number of lines across both the [`Readable`] and the file,
    /// - **D** is the number of differences (i.e., insertions and deletions) between them.
    ///
    /// This method is efficient for practical use cases and provides minimal diffs even in large files.
    fn get_diffs(
        &self,
        readable: &impl Readable,
    ) -> anyhow::Result<Option<DiffsManager>> {
        let meta = std::fs::metadata(readable.location());

        // If there is not metadata, probably the file doesn't exist anymore
        if let Ok(meta) = meta {
            if *readable.modified_time() >= meta.modified()? {
                return Ok(None);
            }

            let memory_content = readable.content();
            let file_content = std::fs::read_to_string(readable.location())?;
            let (seq1, seq2) =
                LineSequence::from_lines(memory_content.lines(), file_content.lines());

            let diffs = DiffsManager::from_myers_algorithm(seq1, seq2);

            Ok(Some(diffs))
        } else {
            // TODO: Consider return all the file as a diff
            Ok(None)
        }
    }
}
