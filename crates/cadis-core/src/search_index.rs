//! Lightweight file search index using trigram matching.
//! Inspired by QMD's FTS5 approach but without SQLite dependency.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

const SKIP_DIRS: &[&str] = &[".git", "target", "node_modules", ".cadis"];
const MAX_FILE_SIZE: u64 = 1024 * 1024;

pub struct SearchIndex {
    entries: Vec<IndexEntry>,
}

struct IndexEntry {
    path: PathBuf,
    trigrams: HashSet<[u8; 3]>,
}

/// A file match with its relevance score.
pub struct SearchHit {
    pub path: PathBuf,
    pub score: f64,
}

fn extract_trigrams(text: &[u8]) -> HashSet<[u8; 3]> {
    let lower: Vec<u8> = text.iter().map(|b| b.to_ascii_lowercase()).collect();
    let mut set = HashSet::new();
    if lower.len() >= 3 {
        for w in lower.windows(3) {
            set.insert([w[0], w[1], w[2]]);
        }
    }
    set
}

fn is_binary(buf: &[u8]) -> bool {
    let check = if buf.len() > 512 { &buf[..512] } else { buf };
    check.contains(&0)
}

impl SearchIndex {
    /// Build index from a workspace root (walks files, extracts trigrams).
    pub fn build(root: &Path) -> Self {
        let mut entries = Vec::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let Ok(rd) = fs::read_dir(&dir) else { continue };
            for entry in rd.flatten() {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                if SKIP_DIRS.iter().any(|s| *s == name_str.as_ref()) {
                    continue;
                }
                let path = entry.path();
                let Ok(meta) = entry.metadata() else { continue };
                if meta.is_dir() {
                    stack.push(path);
                } else if meta.is_file() && meta.len() <= MAX_FILE_SIZE {
                    let Ok(content) = fs::read(&path) else {
                        continue;
                    };
                    if is_binary(&content) {
                        continue;
                    }
                    let trigrams = extract_trigrams(&content);
                    entries.push(IndexEntry { path, trigrams });
                }
            }
        }
        Self { entries }
    }

    /// Number of indexed files.
    pub fn file_count(&self) -> usize {
        self.entries.len()
    }

    /// Search for query, return matching file paths ranked by trigram overlap.
    pub fn search(&self, query: &str, max_results: usize) -> Vec<SearchHit> {
        let qt = extract_trigrams(query.as_bytes());
        if qt.is_empty() {
            return Vec::new();
        }
        let mut hits: Vec<SearchHit> = self
            .entries
            .iter()
            .filter_map(|e| {
                let inter = qt.intersection(&e.trigrams).count();
                if inter == 0 {
                    return None;
                }
                let union = qt.len() + e.trigrams.len() - inter;
                let score = inter as f64 / union as f64;
                Some(SearchHit {
                    path: e.path.clone(),
                    score,
                })
            })
            .collect();
        hits.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        hits.truncate(max_results);
        hits
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_build_and_search() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("hello.rs"), "fn hello_world() {}").unwrap();
        fs::write(dir.path().join("bye.rs"), "fn goodbye_world() {}").unwrap();
        fs::create_dir(dir.path().join(".git")).unwrap();
        fs::write(dir.path().join(".git/config"), "should be skipped").unwrap();

        let idx = SearchIndex::build(dir.path());
        assert_eq!(idx.file_count(), 2);

        let hits = idx.search("hello_world", 10);
        assert!(!hits.is_empty());
        assert!(hits[0].path.to_string_lossy().contains("hello.rs"));
    }

    #[test]
    fn test_skips_binary_and_large_files() {
        let dir = tempfile::tempdir().unwrap();
        // Binary file (contains null bytes)
        fs::write(dir.path().join("bin.dat"), b"abc\x00def").unwrap();
        // Normal text file
        fs::write(dir.path().join("text.txt"), "some text content").unwrap();

        let idx = SearchIndex::build(dir.path());
        assert_eq!(idx.file_count(), 1);
        assert!(idx.entries[0].path.to_string_lossy().contains("text.txt"));
    }
}
