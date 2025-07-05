use super::reader::Readable;
use std::fmt::Display;

/// Diff type for a line
#[derive(Debug)]
pub enum DiffType {
    Add,
    Sub,
    Change
}

/// A difference in one resource
#[derive(Debug)]
pub struct Diff<'a> {
    range: Range,
    readable: &'a dyn Readable,
    diff_type: DiffType
}

/// The lines range of the file
#[derive(Debug, Clone)]
pub struct Range {
    pub start: usize,
    pub end: usize,
}

impl Display for Range {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, ":{}-{}", self.start, self.end)
    }
}

impl Default for Range {
    fn default() -> Self {
        Self { start: 1, end: 0 }
    }
}

impl Range {
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
