//! Persistent memory subsystem for C.A.D.I.S.

use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;

use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};

// ── Types ──────────────────────────────────────────────────────────

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value")]
pub enum MemoryScope {
    Agent(String),
    Project(String),
    Global,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum MemoryKind {
    UserPreference,
    ProjectFact,
    Decision,
    Procedure,
    BugPattern,
    ToolConvention,
    TaskSummary,
    Correction,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum MemoryStatus {
    Candidate,
    Confirmed,
    Superseded,
    Rejected,
    Archived,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MemoryRecord {
    pub id: String,
    pub scope: MemoryScope,
    pub kind: MemoryKind,
    pub status: MemoryStatus,
    pub content: String,
    pub keywords: Vec<String>,
    pub source_session_id: Option<String>,
    pub source_agent_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug)]
pub struct MemoryHit {
    pub record: MemoryRecord,
    pub score: f64,
}

#[derive(Clone, Debug)]
pub struct MemoryCapsule {
    pub entries: Vec<String>,
    pub total_chars: usize,
    pub truncated: bool,
}

#[derive(Debug)]
pub enum MemoryError {
    Io(std::io::Error),
    Parse(String),
    NotFound,
}

impl std::fmt::Display for MemoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "io: {e}"),
            Self::Parse(msg) => write!(f, "parse: {msg}"),
            Self::NotFound => write!(f, "not found"),
        }
    }
}

impl From<std::io::Error> for MemoryError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<serde_json::Error> for MemoryError {
    fn from(e: serde_json::Error) -> Self {
        Self::Parse(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, MemoryError>;

// ── Store ──────────────────────────────────────────────────────────

pub struct MemoryStore {
    profile_root: PathBuf,
}

impl MemoryStore {
    pub fn new(profile_root: impl Into<PathBuf>) -> Self {
        Self {
            profile_root: profile_root.into(),
        }
    }

    fn ledger_path(&self) -> PathBuf {
        self.profile_root.join("memory").join("ledger.jsonl")
    }

    fn ensure_dir(&self) -> Result<()> {
        let dir = self.profile_root.join("memory");
        if !dir.exists() {
            fs::create_dir_all(&dir)?;
        }
        Ok(())
    }

    fn read_all(&self) -> Result<Vec<MemoryRecord>> {
        let path = self.ledger_path();
        if !path.exists() {
            return Ok(Vec::new());
        }
        let file = fs::File::open(&path)?;
        let reader = BufReader::new(file);
        let mut records = Vec::new();
        for line in reader.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let record: MemoryRecord = serde_json::from_str(&line)?;
            records.push(record);
        }
        Ok(records)
    }

    fn write_all(&self, records: &[MemoryRecord]) -> Result<()> {
        self.ensure_dir()?;
        let path = self.ledger_path();
        let mut file = fs::File::create(&path)?;
        for record in records {
            let line = serde_json::to_string(record)?;
            writeln!(file, "{line}")?;
        }
        Ok(())
    }

    pub fn propose(&self, mut record: MemoryRecord) -> Result<MemoryRecord> {
        self.ensure_dir()?;
        let now = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);
        if record.id.is_empty() {
            let existing = self.read_all().unwrap_or_default();
            record.id = format!("mem_{:06}", existing.len() + 1);
        }
        record.status = MemoryStatus::Candidate;
        record.created_at = now.clone();
        record.updated_at = now;
        if record.keywords.is_empty() {
            record.keywords = extract_keywords(&record.content);
        }
        let path = self.ledger_path();
        let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
        let line = serde_json::to_string(&record)?;
        writeln!(file, "{line}")?;
        Ok(record)
    }

    pub fn promote(&self, id: &str) -> Result<()> {
        self.set_status(id, MemoryStatus::Confirmed)
    }

    pub fn reject(&self, id: &str) -> Result<()> {
        self.set_status(id, MemoryStatus::Rejected)
    }

    fn set_status(&self, id: &str, status: MemoryStatus) -> Result<()> {
        let mut records = self.read_all()?;
        let now = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);
        let mut found = false;
        for record in &mut records {
            if record.id == id {
                record.status = status.clone();
                record.updated_at = now.clone();
                found = true;
                break;
            }
        }
        if !found {
            return Err(MemoryError::NotFound);
        }
        self.write_all(&records)
    }

    pub fn search(
        &self,
        query: &str,
        scope: Option<&MemoryScope>,
        limit: usize,
    ) -> Result<Vec<MemoryHit>> {
        let records = self.read_all()?;
        let query_words = extract_keywords(query);
        let mut hits: Vec<MemoryHit> = records
            .into_iter()
            .filter(|r| scope.is_none_or(|s| &r.scope == s))
            .filter_map(|r| {
                let score = keyword_score(&r, &query_words);
                if score > 0.0 {
                    Some(MemoryHit { record: r, score })
                } else {
                    None
                }
            })
            .collect();
        hits.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        hits.truncate(limit);
        Ok(hits)
    }

    pub fn compile_capsule(
        &self,
        scope: Option<&MemoryScope>,
        max_chars: usize,
    ) -> Result<MemoryCapsule> {
        let records = self.read_all()?;
        let confirmed: Vec<&MemoryRecord> = records
            .iter()
            .filter(|r| r.status == MemoryStatus::Confirmed)
            .filter(|r| scope.is_none_or(|s| &r.scope == s))
            .collect();
        let mut entries = Vec::new();
        let mut total_chars = 0;
        let mut truncated = false;
        for record in confirmed {
            let entry = format!("[{:?}] {}", record.kind, record.content);
            if total_chars + entry.len() > max_chars {
                truncated = true;
                break;
            }
            total_chars += entry.len();
            entries.push(entry);
        }
        Ok(MemoryCapsule {
            entries,
            total_chars,
            truncated,
        })
    }
}

// ── Helpers ────────────────────────────────────────────────────────

const STOPWORDS: &[&str] = &[
    "a", "an", "the", "is", "it", "in", "on", "of", "to", "and", "or", "for", "with", "at", "by",
    "from", "as", "be", "was", "are", "this", "that",
];

pub fn extract_keywords(content: &str) -> Vec<String> {
    let stop: HashSet<&str> = STOPWORDS.iter().copied().collect();
    let mut seen = HashSet::new();
    content
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .map(|w| w.to_lowercase())
        .filter(|w| w.len() > 1 && !stop.contains(w.as_str()))
        .filter(|w| seen.insert(w.clone()))
        .collect()
}

pub fn keyword_score(record: &MemoryRecord, query_words: &[String]) -> f64 {
    if query_words.is_empty() {
        return 0.0;
    }
    let content_lower = record.content.to_lowercase();
    let mut score = 0.0;
    for qw in query_words {
        if record.keywords.iter().any(|k| k == qw) {
            score += 2.0;
        }
        if content_lower.contains(qw.as_str()) {
            score += 1.0;
        }
    }
    score / query_words.len() as f64
}

// ── Tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store() -> (tempfile::TempDir, MemoryStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::new(dir.path());
        (dir, store)
    }

    fn sample_record(content: &str) -> MemoryRecord {
        MemoryRecord {
            id: String::new(),
            scope: MemoryScope::Global,
            kind: MemoryKind::ProjectFact,
            status: MemoryStatus::Candidate,
            content: content.to_owned(),
            keywords: Vec::new(),
            source_session_id: None,
            source_agent_id: None,
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    #[test]
    fn propose_and_search() {
        let (_dir, store) = temp_store();
        let record = store
            .propose(sample_record("Rust workspace uses cargo"))
            .unwrap();
        assert!(!record.id.is_empty());
        assert_eq!(record.status, MemoryStatus::Candidate);

        let hits = store.search("cargo workspace", None, 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].score > 0.0);
    }

    #[test]
    fn promote_and_capsule() {
        let (_dir, store) = temp_store();
        let r = store
            .propose(sample_record("Always run tests before commit"))
            .unwrap();
        store.promote(&r.id).unwrap();

        let capsule = store.compile_capsule(None, 4096).unwrap();
        assert_eq!(capsule.entries.len(), 1);
        assert!(!capsule.truncated);
        assert!(capsule.entries[0].contains("Always run tests"));
    }

    #[test]
    fn compile_capsule_truncation() {
        let (_dir, store) = temp_store();
        let r1 = store
            .propose(sample_record("First fact about the project"))
            .unwrap();
        let r2 = store
            .propose(sample_record("Second fact about the project"))
            .unwrap();
        store.promote(&r1.id).unwrap();
        store.promote(&r2.id).unwrap();

        let capsule = store.compile_capsule(None, 50).unwrap();
        assert_eq!(capsule.entries.len(), 1);
        assert!(capsule.truncated);
    }
}
