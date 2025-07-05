use std::fmt::Display;
use std::{collections::HashMap, ops::Index, str::Lines};

/// The type of a difference and its content owns the String because the read data is not owned
/// for any other element.
#[derive(Debug, PartialEq)]
pub enum Diff {
    Match((usize, String)),
    Insert((usize, String)),
    Delete((usize, String)),
}

impl Display for Diff {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let line = match self {
            Self::Match((line_number, line)) => format!("{line_number} {line}"),
            Self::Insert((line_number, line)) => format!("+ {line_number} {line}"),
            Self::Delete((line_number, line)) => format!("- {line_number} {line}"),
        };

        write!(f, "{line}")
    }
}

/// Manage all the differences between two readable resources. Uses `Myers' Algorithm` to
/// calculate the shortest path for retrieving the differences, storing all the results and their types.
#[derive(Debug)]
pub struct DiffsManager {
    pub diffs: Vec<Diff>,
}

impl DiffsManager {
    pub fn from_myers_algorithm(seq1: LineSequence, seq2: LineSequence) -> DiffsManager {
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
                edits.push(Diff::Match((x, seq1.lines[x - 1].to_string())));
                x -= 1;
                y -= 1;
            }

            if x == prev_x {
                edits.push(Diff::Insert((y, seq2.lines[y - 1].to_string())));
                y -= 1;
            } else {
                edits.push(Diff::Delete((x, seq1.lines[x - 1].to_string())));
                x -= 1;
            }
        }

        edits.reverse();
        Self {
            diffs: edits,
        }
    }
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

/// A sequence of string lines with their hash for quick comparison, unifying identical content using
/// the same hash for both represents an ordered sequence of lines of strings.
pub struct LineSequence<'a> {
    hashes: Vec<usize>,
    lines: Vec<&'a str>,
}

impl<'a> LineSequence<'a> {
    /// Create structs that match common lines with the same hash and generate the rest of the hashes
    /// for all the other lines
    pub fn from_lines(lines1: Lines<'a>, lines2: Lines<'a>) -> (Self, Self) {
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
        &self.hashes[index]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_myers() {
        let str1 = r#"hello, this is a test
and this is a Myers' Algorithm
hello world
Bye world
"#
        .to_string();

        let str2 = r#"bye, this is a test
and this is a Myers' Algorithm
hello earth
bye world
additional line
"#
        .to_string();

        let lines1 = str1.lines();
        let lines2 = str2.lines();

        let (seq1, seq2) = LineSequence::from_lines(lines1, lines2);
        let diffs = DiffsManager::from_myers_algorithm(seq1, seq2);

        let expected = vec![
            Diff::Delete((1, "hello, this is a test".to_string())),
            Diff::Insert((1, "bye, this is a test".to_string())),
            Diff::Match((2, "and this is a Myers' Algorithm".to_string())),
            Diff::Delete((3, "hello world".to_string())),
            Diff::Delete((4, "Bye world".to_string())),
            Diff::Insert((3, "hello earth".to_string())),
            Diff::Insert((4, "bye world".to_string())),
            Diff::Insert((5, "additional line".to_string())),
        ];

        assert_eq!(diffs.diffs.len(), expected.len());

        for (i, edit) in diffs.diffs.iter().enumerate() {
            assert_eq!(*edit, expected[i]);
        }
    }
}
