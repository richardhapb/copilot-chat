/// Any readable resource
pub trait Readable {
    /// Returns the location of the readable
    /// e.g. An absolute file path
    fn location(&self) -> &str;
}

/// Read a resource
pub trait ReaderTool {
    /// Read the content of a readable
    async fn read(&mut self, readable: &impl Readable) -> anyhow::Result<()>;
}
