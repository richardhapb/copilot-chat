use std::{collections::HashMap, ops::Index, str::Lines, time::SystemTime};

use crate::tools::diff::Diff;

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
    fn get_diffs(&self, readable: &impl Readable) -> anyhow::Result<Vec<Diff>> {
        let mut diffs = vec![];
        let meta = std::fs::metadata(readable.location())?;
        if *readable.modified_time() >= meta.modified()? {
            return Ok(diffs);
        }

        let memory_content = readable.content();
        let file_content = std::fs::read_to_string(readable.location())?;
        let (seq1, seq2) = LineSequence::from_lines(memory_content.lines(), file_content.lines());

        let edits = EditPath::from_myers_algorithm(seq1, seq2);

        return Ok(diffs);
    }
}

/// Array that suppors negative indexes
#[derive(Clone)]
struct SignedArray {
    pos_arr: Vec<i32>,
    neg_arr: Vec<i32>,
}

impl SignedArray {
    fn new(length: usize) -> Self {
        Self {
            pos_arr: vec![-1; length + 1],
            neg_arr: vec![-1; length + 1],
        }
    }

    fn set(&mut self, idx: i32, value: i32) {
        if idx < 0 {
            self.neg_arr[-idx as usize] = value;
        } else {
            self.pos_arr[idx as usize] = value;
        }
    }

    fn get(&self, idx: i32) -> i32 {
        if idx < 0 {
            self.neg_arr[-idx as usize]
        } else {
            self.pos_arr[idx as usize]
        }
    }
}

struct LineSequence<'a> {
    hashes: Vec<usize>,
    lines: Vec<&'a str>,
}

impl<'a> LineSequence<'a> {
    /// Create structs that match common lines with the same hash and generate the rest of the hashes
    /// for all the other lines
    fn from_lines(lines1: Lines<'a>, lines2: Lines<'a>) -> (Self, Self) {
        let mut map: HashMap<&'a str, usize> = HashMap::new();
        let mut next_hash = 0;

        let (hashes1, lines_vec1): (Vec<_>, Vec<_>) = lines1
            .map(|line| {
                let entry = map.entry(line).or_insert_with(|| {
                    let h = next_hash;
                    next_hash += 1;
                    h
                });
                (*entry, line)
            })
            .unzip();

        let (hashes2, lines_vec2): (Vec<_>, Vec<_>) = lines2
            .map(|line| {
                let entry = map.entry(line).or_insert_with(|| {
                    let h = next_hash;
                    next_hash += 1;
                    h
                });
                (*entry, line)
            })
            .unzip();

        (
            Self {
                hashes: hashes1,
                lines: lines_vec1,
            },
            Self {
                hashes: hashes2,
                lines: lines_vec2,
            },
        )
    }

    fn len(&self) -> usize {
        self.hashes.len()
    }
}

impl Index<usize> for LineSequence<'_> {
    type Output = usize;
    fn index(&self, index: usize) -> &Self::Output {
        &self.hashes[index as usize]
    }
}

#[derive(Debug, Clone)]
struct SnakePath {
    prev_path: Option<Box<SnakePath>>,
    x: i32,
    y: i32,
    length: usize,
}

#[derive(Debug, PartialEq)]
enum Edit<'a> {
    Match(&'a str),
    Insert(&'a str),
    Delete(&'a str),
}

#[derive(Debug)]
struct EditPath<'a> {
    edits: Vec<Edit<'a>>,
}

impl<'a> EditPath<'a> {
    fn from_myers_algorithm(seq1: LineSequence<'a>, seq2: LineSequence<'a>) -> EditPath<'a> {
        let (x_axis_len, y_axis_len) = (seq1.len(), seq2.len());
        let max = (x_axis_len + y_axis_len) as i32;

        // Store states for backtracking
        let mut trace: Vec<SignedArray> = vec![];

        let mut v = SignedArray::new(max as usize);
        v.set(1, 0);

        let mut final_d = 0;

        'outer: for d in 0..=max {
            let mut v_current = SignedArray::new(max as usize);

            for k in (-d..=d).step_by(2) {
                let x_start = if k == -d || (k != d && v.get(k - 1) < v.get(k + 1)) {
                    v.get(k + 1) // insertion (vertical)
                } else {
                    v.get(k - 1) + 1 // deletion (horizontal)
                };
                let y_start = x_start - k;

                let mut x = x_start as usize;
                let mut y = y_start as usize;

                while x < x_axis_len && y < y_axis_len && seq1[x] == seq2[y] {
                    x += 1;
                    y += 1;
                }

                v_current.set(k, x as i32);

                if x >= x_axis_len && y >= y_axis_len {
                    final_d = d;
                    break 'outer;
                }
            }

            trace.push(v_current.clone());
            v = v_current; // update current state
        }

        // Backtrack from (seq1.len(), seq2.len()) to (0,0)
        let mut x = x_axis_len;
        let mut y = y_axis_len;
        let mut edits = vec![];

        for d in (0..final_d).rev() {
            let v = &trace[d as usize];
            let k = x as i32 - y as i32;

            let prev_k = if k == -d || (k != d && v.get(k - 1) < v.get(k + 1)) {
                k + 1 // insertion
            } else {
                k - 1 // deletion
            };

            let prev_x = v.get(prev_k) as usize;
            let prev_y = (prev_x as i32 - prev_k) as usize;

            while x > prev_x && y > prev_y {
                edits.push(Edit::Match(seq1.lines[x - 1]));
                x -= 1;
                y -= 1;
            }

            if x == prev_x {
                edits.push(Edit::Insert(seq2.lines[y - 1]));
                y -= 1;
            } else {
                edits.push(Edit::Delete(seq1.lines[x - 1]));
                x -= 1;
            }
        }

        edits.reverse();
        Self { edits }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_myers() {
        let str1 = r#"
hello, this is a test
and this is a Myers' Algorithm
hello world
Bye world
"#
        .to_string();

        let str2 = r#"
bye, this is a test
and this is a Myers' Algorithm
hello earth
bye world
"#
        .to_string();

        let lines1 = str1.lines();
        let lines2 = str2.lines();

        let (seq1, seq2) = LineSequence::from_lines(lines1, lines2);
        let diffs = EditPath::from_myers_algorithm(seq1, seq2);

        let expected = vec![
            Edit::Delete("hello, this is a test"),
            Edit::Insert("bye, this is a test"),
            Edit::Match("and this is a Myers' Algorithm"),
            Edit::Delete("hello world"),
            Edit::Delete("Bye world"),
            Edit::Insert("hello earth"),
            Edit::Insert("bye world"),
        ];

        assert_eq!(diffs.edits.len(), expected.len());

        for (i, edit) in diffs.edits.iter().enumerate() {
            assert_eq!(*edit, expected[i]);
        }
    }
}
