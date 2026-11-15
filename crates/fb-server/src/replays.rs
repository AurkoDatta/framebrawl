//! Finished matches on disk.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use fb_host::FinishedMatch;
use serde::Serialize;

/// Replays kept before the oldest are deleted.
const KEEP: usize = 200;

#[derive(Clone)]
pub struct ReplayStore {
    dir: Option<PathBuf>,
}

#[derive(Serialize)]
pub struct ReplayEntry {
    pub name: String,
    pub bytes: u64,
    /// Seconds since the Unix epoch, taken from the file name.
    pub recorded_at: u64,
}

impl ReplayStore {
    /// Without a directory, replays are discarded.
    pub fn new(dir: Option<PathBuf>) -> Self {
        ReplayStore { dir }
    }

    /// Writes the replay in the background; a failure is logged, not fatal.
    pub fn save(&self, finished: FinishedMatch) {
        let Some(dir) = self.dir.clone() else { return };
        tokio::task::spawn_blocking(move || {
            let seconds = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_secs());
            // Zero-padded so names sort by time.
            let name = format!("{seconds:012}-{:016x}.fbr", finished.id);
            let result = std::fs::create_dir_all(&dir)
                .and_then(|()| std::fs::write(dir.join(&name), finished.replay.encode()))
                .and_then(|()| prune(&dir));
            match result {
                Ok(()) => tracing::info!(%name, "replay saved"),
                Err(error) => tracing::warn!(%name, %error, "could not save replay"),
            }
        });
    }

    /// Newest first.
    pub fn list(&self) -> Vec<ReplayEntry> {
        let Some(dir) = &self.dir else {
            return Vec::new();
        };
        let mut entries: Vec<ReplayEntry> = replay_files(dir)
            .into_iter()
            .filter_map(|path| {
                let name = path.file_stem()?.to_str()?.to_owned();
                Some(ReplayEntry {
                    recorded_at: name.split('-').next()?.parse().ok()?,
                    bytes: path.metadata().ok()?.len(),
                    name,
                })
            })
            .collect();
        entries.sort_by(|a, b| b.name.cmp(&a.name));
        entries
    }

    pub fn read(&self, name: &str) -> Option<Vec<u8>> {
        // Names are generated here; anything else is not a replay and must
        // not be turned into a path.
        let valid = name.len() == 29 && name.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-');
        if !valid {
            return None;
        }
        std::fs::read(self.dir.as_ref()?.join(format!("{name}.fbr"))).ok()
    }
}

fn replay_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(read) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    read.filter_map(|entry| Some(entry.ok()?.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "fbr"))
        .collect()
}

fn prune(dir: &Path) -> std::io::Result<()> {
    let mut files = replay_files(dir);
    files.sort();
    let excess = files.len().saturating_sub(KEEP);
    for path in &files[..excess] {
        std::fs::remove_file(path)?;
    }
    Ok(())
}
