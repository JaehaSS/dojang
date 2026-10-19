//! Initial bounded indexing plus changed-directory discovery. Never rereads cached log headers
//! just because a child appended output; active date directories get priority on every refresh.
use super::{file_identity, session_meta, Meta, MAX_ENTRIES};
use chrono::Datelike;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

const ENTRIES_PER_STEP: usize = 512;
const HEADER_BYTES_PER_STEP: usize = 1024 * 1024;
const MAX_DIRECTORIES: usize = 2048;

#[derive(Clone, PartialEq, Eq)]
struct Revision {
    identity: Option<(u64, u64)>,
    modified: Option<SystemTime>,
    len: u64,
}
impl Revision {
    fn of(metadata: &fs::Metadata) -> Self {
        Self {
            identity: file_identity(metadata),
            modified: metadata.modified().ok(),
            len: metadata.len(),
        }
    }
}

struct FileRecord {
    revision: Revision,
    complete: bool,
}
struct Scan {
    path: PathBuf,
    entries: fs::ReadDir,
}

pub(super) struct Discovery {
    root: PathBuf,
    scans: VecDeque<Scan>,
    directories: BTreeMap<PathBuf, Revision>,
    watched: BTreeSet<PathBuf>,
    files: BTreeMap<PathBuf, FileRecord>,
    pending: VecDeque<(PathBuf, Revision)>,
    queued: BTreeSet<PathBuf>,
    refresh_at: Instant,
    started_at: SystemTime,
    retry_index: usize,
    pub(super) limited: bool,
}
impl Discovery {
    pub(super) fn new(root: PathBuf) -> Self {
        let mut out = Self {
            root,
            scans: VecDeque::new(),
            directories: BTreeMap::new(),
            watched: BTreeSet::new(),
            files: BTreeMap::new(),
            pending: VecDeque::new(),
            queued: BTreeSet::new(),
            refresh_at: Instant::now(),
            started_at: SystemTime::now(),
            retry_index: 0,
            limited: false,
        };
        out.watch(out.root.clone());
        out.refresh_dates();
        out
    }

    pub(super) fn watch(&mut self, path: PathBuf) {
        if self.watched.len() >= MAX_DIRECTORIES && !self.watched.contains(&path) {
            self.limited = true;
            return;
        }
        self.watched.insert(path.clone());
        self.queue_directory(&path, true);
    }

    fn refresh_dates(&mut self) {
        // Codex date shards can follow local time. Include UTC too at the rollover boundary.
        let dates = [
            chrono::Utc::now().date_naive(),
            chrono::Local::now().date_naive(),
        ];
        for date in dates {
            let year = self.root.join(format!("{:04}", date.year()));
            let month = year.join(format!("{:02}", date.month()));
            let day = month.join(format!("{:02}", date.day()));
            for path in [year, month, day] {
                self.watch(path);
            }
        }
    }

    fn queue_directory(&mut self, path: &Path, priority: bool) {
        if self.scans.iter().any(|scan| scan.path == path) {
            return;
        }
        let Ok(metadata) = fs::metadata(path) else {
            return;
        };
        let revision = Revision::of(&metadata);
        if self.directories.get(path) == Some(&revision) {
            return;
        }
        if self.directories.len() >= MAX_DIRECTORIES && !self.directories.contains_key(path) {
            // A historical cache cannot deny an active date directory a scan forever.
            if let Some(evict) = self
                .directories
                .keys()
                .find(|known| *known != &self.root && !self.watched.contains(*known))
                .cloned()
            {
                self.directories.remove(&evict);
            } else {
                self.limited = true;
                return;
            }
        }
        let Ok(entries) = fs::read_dir(path) else {
            return;
        };
        self.directories.insert(path.to_path_buf(), revision);
        let scan = Scan {
            path: path.to_path_buf(),
            entries,
        };
        if priority {
            self.scans.push_front(scan);
        } else {
            self.scans.push_back(scan);
        }
    }

    pub(super) fn step(&mut self, force_refresh: bool) -> Vec<Meta> {
        if force_refresh || Instant::now() >= self.refresh_at {
            self.refresh_at = Instant::now() + Duration::from_secs(2);
            let watched: Vec<_> = self.watched.iter().cloned().collect();
            for path in watched {
                self.queue_directory(&path, false);
            }
            self.refresh_dates();
        }
        // Active date shards are cheap to stat and must discover progress before short idle
        // deadlines. Historical watched directories retain the slower refresh cadence.
        self.refresh_dates();
        // Retry changed incomplete headers every bounded discovery tick, independently of the
        // directory refresh cadence. Rotate the probe budget so partial historical files cannot
        // indefinitely hide a newly created child's completed header.
        let incomplete: Vec<_> = self
            .files
            .iter()
            .filter(|(_, record)| !record.complete)
            .map(|(path, record)| (path.clone(), record.revision.clone()))
            .collect();
        if !incomplete.is_empty() {
            let start = self.retry_index % incomplete.len();
            for n in 0..incomplete.len().min(128) {
                let (path, old) = &incomplete[(start + n) % incomplete.len()];
                if let Ok(metadata) = fs::metadata(path) {
                    let revision = Revision::of(&metadata);
                    if force_refresh || revision != *old {
                        self.enqueue_file(path.clone(), revision, true);
                    }
                }
            }
            self.retry_index = (start + 128) % incomplete.len();
        }
        for _ in 0..ENTRIES_PER_STEP {
            // One entry per directory keeps a huge history shard from starving the active date.
            let Some(mut scan) = self.scans.pop_front() else {
                break;
            };
            let entry = scan.entries.next();
            if entry.is_some() {
                self.scans.push_back(scan);
            }
            let Some(entry) = entry else {
                continue;
            };
            let Ok(entry) = entry else {
                continue;
            };
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                self.queue_directory(&entry.path(), false);
            } else if kind.is_file() && entry.path().extension().is_some_and(|ext| ext == "jsonl") {
                let path = entry.path();
                let Ok(metadata) = entry.metadata() else {
                    continue;
                };
                let revision = Revision::of(&metadata);
                if let Some(record) = self.files.get(&path) {
                    // Appends cannot change a complete immutable session_meta header.
                    if record.revision.identity == revision.identity
                        && record.complete
                        && revision.len >= record.revision.len
                    {
                        continue;
                    }
                    if record.complete && record.revision == revision {
                        continue;
                    }
                }
                self.enqueue_file(path, revision, false);
            }
        }
        let mut budget = HEADER_BYTES_PER_STEP;
        let mut found = Vec::new();
        while budget > 0 {
            let Some((path, revision)) = self.pending.pop_front() else {
                break;
            };
            self.queued.remove(&path);
            let (meta, complete, bytes) = session_meta(&path, budget);
            budget = budget.saturating_sub(bytes.max(1));
            self.files
                .insert(path.clone(), FileRecord { revision, complete });
            if let Some(meta) = meta {
                found.push(meta.with_path(path));
            }
        }
        found
    }

    fn enqueue_file(&mut self, path: PathBuf, revision: Revision, retry: bool) {
        if self.queued.len() >= MAX_ENTRIES && !self.queued.contains(&path) {
            self.limited = true;
            return;
        }
        if !self.files.contains_key(&path) && self.files.len() >= MAX_ENTRIES {
            // Do not turn a full historical cache into a permanent active-log blind spot.
            if let Some(evict) = self
                .files
                .keys()
                .find(|known| !self.queued.contains(*known))
                .cloned()
            {
                self.files.remove(&evict);
            } else {
                self.limited = true;
                return;
            }
        }
        if !self.queued.insert(path.clone()) {
            return;
        }
        let active = retry
            || revision
                .modified
                .is_some_and(|time| time >= self.started_at);
        if active {
            self.pending.push_front((path, revision));
        } else {
            self.pending.push_back((path, revision));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (PathBuf, PathBuf, String) {
        let nanos = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "praxis-codex-discovery-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("rollout-fixture.jsonl");
        let header = serde_json::json!({"timestamp":"2026-10-07T00:00:00Z","type":"session_meta","payload":{"id":"20000000-0000-0000-0000-000000000002","parent_thread_id":"10000000-0000-0000-0000-000000000001"}}).to_string()+"\n";
        (root, path, header)
    }
    #[test]
    fn partial_header_retries_without_directory_revision_changes() {
        let (root, path, header) = fixture();
        fs::write(&path, &header[..header.len() / 2]).unwrap();
        let mut discovery = Discovery::new(root.clone());
        assert!(discovery.step(true).is_empty());
        assert!(!discovery.files[&path].complete);
        let unchanged = Revision::of(&fs::metadata(&root).unwrap());
        fs::write(&path, &header).unwrap();
        assert!(unchanged == Revision::of(&fs::metadata(&root).unwrap()));
        let found = discovery.step(true);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, "20000000-0000-0000-0000-000000000002");
        assert!(discovery.files[&path].complete);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn append_does_not_reparse_cached_complete_headers_on_directory_rescan() {
        let (root, path, header) = fixture();
        fs::write(&path, &header).unwrap();
        let mut discovery = Discovery::new(root.clone());
        assert_eq!(discovery.step(true).len(), 1);
        fs::write(&path, header + "{\"public\":\"appended\"}\n").unwrap();
        fs::write(root.join("new-unrelated-file"), "directory changed").unwrap();
        assert!(discovery.step(true).is_empty());
        assert_eq!(discovery.files.len(), 1);
        fs::remove_dir_all(root).unwrap();
    }
}
