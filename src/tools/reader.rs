use chrono::Local;

/// Any readable resource
pub trait Readable {
    /// Returns the location of the readable
    /// e.g. An absolute file path
    fn location(&self) -> &str;
    fn modified_time(&self) -> &chrono::DateTime<Local>;
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
}
