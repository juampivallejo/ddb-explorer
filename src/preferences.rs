//! Local preferences only; never stores AWS credentials or item contents.
use crate::dynamodb::Connection;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs, io,
    path::{Path, PathBuf},
    sync::mpsc,
    thread::{self, JoinHandle},
};
use tokio::sync::oneshot;

#[derive(Clone, Serialize, Deserialize)]
pub struct Preferences {
    version: u32,
    #[serde(default)]
    local_favorites: BTreeSet<String>,
    #[serde(default)]
    terminal_favorites: BTreeSet<String>,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            version: 1,
            local_favorites: BTreeSet::new(),
            terminal_favorites: BTreeSet::new(),
        }
    }
}

impl Preferences {
    pub fn favorites(&self, connection: Connection) -> &BTreeSet<String> {
        match connection {
            Connection::Local => &self.local_favorites,
            Connection::Terminal => &self.terminal_favorites,
        }
    }

    pub fn apply(&mut self, change: &FavoriteChange) {
        let favorites = match change.connection {
            Connection::Local => &mut self.local_favorites,
            Connection::Terminal => &mut self.terminal_favorites,
        };
        if change.favorite {
            favorites.insert(change.table.clone());
        } else {
            favorites.remove(&change.table);
        }
    }
}

/// An explicit desired state is safe to retry; toggling the on-disk state isn't.
#[derive(Clone)]
pub struct FavoriteChange {
    pub connection: Connection,
    pub table: String,
    pub favorite: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreferenceError {
    Load,
    Save,
}

impl PreferenceError {
    pub fn message(self) -> &'static str {
        match self {
            Self::Load => {
                "Favorites couldn't be loaded. Check preferences.json and restart. The existing file has been kept."
            }
            Self::Save => {
                "Favorites couldn't be saved. Check access to the preferences folder and retry."
            }
        }
    }
}

struct SaveJob {
    change: Option<FavoriteChange>,
    reply: oneshot::Sender<Result<Preferences, PreferenceError>>,
}

pub struct PreferenceStore {
    sender: Option<mpsc::Sender<SaveJob>>,
    worker: Option<JoinHandle<()>>,
}

impl PreferenceStore {
    pub fn open() -> Result<(Self, Preferences), PreferenceError> {
        let path = dirs::config_dir()
            .ok_or_else(|| {
                log::warn!("Couldn't locate the preferences directory");
                PreferenceError::Load
            })?
            .join("ddb-explorer")
            .join("preferences.json");
        Self::open_at(path)
    }

    fn open_at(path: PathBuf) -> Result<(Self, Preferences), PreferenceError> {
        let preferences = load(&path).map_err(|error| {
            log::warn!("Couldn't load preferences at {}: {error}", path.display());
            PreferenceError::Load
        })?;
        let (sender, receiver) = mpsc::channel::<SaveJob>();
        // One writer preserves click order; replacing a UI task cannot cancel a save.
        let worker = thread::Builder::new()
            .name("preferences".into())
            .spawn(move || {
                let mut pending = Vec::new();
                for job in receiver {
                    if let Some(change) = job.change {
                        pending.push(change);
                    }
                    let result = save_changes(&path, &pending).map_err(|error| {
                        log::warn!("Couldn't save preferences at {}: {error}", path.display());
                        PreferenceError::Save
                    });
                    // Retain failed actions so Retry (or a later click) saves them
                    // without overwriting unrelated favorites from other processes.
                    if result.is_ok() {
                        pending.clear();
                    }
                    let _ = job.reply.send(result);
                }
            })
            .map_err(|error| {
                log::warn!("Couldn't start preferences writer: {error}");
                PreferenceError::Load
            })?;
        Ok((
            Self {
                sender: Some(sender),
                worker: Some(worker),
            },
            preferences,
        ))
    }

    pub fn save(
        &self,
        change: FavoriteChange,
    ) -> oneshot::Receiver<Result<Preferences, PreferenceError>> {
        self.send(Some(change))
    }

    pub fn retry(&self) -> oneshot::Receiver<Result<Preferences, PreferenceError>> {
        self.send(None)
    }

    fn send(
        &self,
        change: Option<FavoriteChange>,
    ) -> oneshot::Receiver<Result<Preferences, PreferenceError>> {
        let (reply, result) = oneshot::channel();
        if self
            .sender
            .as_ref()
            .expect("store is open")
            .send(SaveJob { change, reply })
            .is_err()
        {
            log::warn!("Preferences writer stopped unexpectedly");
        }
        result
    }
}

fn load(path: &Path) -> io::Result<Preferences> {
    let preferences = match fs::read(path) {
        Ok(bytes) => serde_json::from_slice::<Preferences>(&bytes)?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => Preferences::default(),
        Err(error) => return Err(error),
    };
    if preferences.version != 1 {
        return Err(io::Error::other("Unsupported preferences version"));
    }
    Ok(preferences)
}

fn save_changes(path: &Path, changes: &[FavoriteChange]) -> io::Result<Preferences> {
    let directory = path
        .parent()
        .ok_or_else(|| io::Error::other("Missing preferences directory"))?;
    fs::create_dir_all(directory)?;
    // Lock a stable file: preferences.json is replaced by atomic saves. Keep the
    // lock file on disk so every process continues locking the same file.
    let lock = fs::File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path.with_extension("lock"))?;
    lock.lock()?;
    let mut preferences = load(path)?;
    for change in changes {
        preferences.apply(change);
    }
    save_atomic(path, &preferences)?;
    // Closing the handle releases the lock, including on any earlier error.
    drop(lock);
    Ok(preferences)
}

impl Drop for PreferenceStore {
    fn drop(&mut self) {
        // Flush queued favorites before exit, even if the window closes immediately.
        self.sender.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn save_atomic(path: &Path, preferences: &Preferences) -> io::Result<()> {
    let directory = path
        .parent()
        .ok_or_else(|| io::Error::other("Missing preferences directory"))?;
    fs::create_dir_all(directory)?;
    let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
    serde_json::to_writer_pretty(&mut temporary, preferences)?;
    temporary.as_file().sync_all()?;
    temporary.persist(path).map_err(|error| error.error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change(connection: Connection, table: &str, favorite: bool) -> FavoriteChange {
        FavoriteChange {
            connection,
            table: table.into(),
            favorite,
        }
    }

    fn saved(
        receiver: oneshot::Receiver<Result<Preferences, PreferenceError>>,
    ) -> Result<Preferences, PreferenceError> {
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(receiver)
            .unwrap()
    }

    #[test]
    fn favorites_round_trip_and_remain_separate_for_each_connection() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("nested/preferences.json");
        let (store, _) = PreferenceStore::open_at(path.clone()).unwrap();
        drop(store.save(change(Connection::Local, "Events", true)));
        drop(store.save(change(Connection::Terminal, "Books", true)));
        drop(store);
        let (store, reloaded) = PreferenceStore::open_at(path.clone()).unwrap();
        assert!(reloaded.favorites(Connection::Local).contains("Events"));
        assert!(!reloaded.favorites(Connection::Terminal).contains("Events"));
        assert!(reloaded.favorites(Connection::Terminal).contains("Books"));
        drop(store.save(change(Connection::Local, "Events", false)));
        drop(store);
        let (_, reloaded) = PreferenceStore::open_at(path).unwrap();
        assert!(reloaded.favorites(Connection::Local).is_empty());
        assert!(reloaded.favorites(Connection::Terminal).contains("Books"));
    }

    #[test]
    fn rapid_updates_flush_in_order_even_when_receivers_are_dropped() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("preferences.json");
        let (store, _) = PreferenceStore::open_at(path.clone()).unwrap();
        for index in 0..21 {
            drop(store.save(change(Connection::Local, "Events", index % 2 == 0)));
        }
        drop(store);
        let (_, reloaded) = PreferenceStore::open_at(path).unwrap();
        assert!(reloaded.favorites(Connection::Local).contains("Events"));
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 2);
    }

    #[test]
    fn stale_instances_merge_actions_and_repeated_favoriting_is_idempotent() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("preferences.json");
        let (first, _) = PreferenceStore::open_at(path.clone()).unwrap();
        let (second, stale) = PreferenceStore::open_at(path.clone()).unwrap();
        assert!(stale.favorites(Connection::Local).is_empty());
        saved(first.save(change(Connection::Local, "Books", true))).unwrap();
        let merged = saved(second.save(change(Connection::Local, "Events", true))).unwrap();
        assert!(merged.favorites(Connection::Local).contains("Books"));
        assert!(merged.favorites(Connection::Local).contains("Events"));
        let merged = saved(second.save(change(Connection::Local, "Books", true))).unwrap();
        assert!(merged.favorites(Connection::Local).contains("Books"));
        saved(first.save(change(Connection::Terminal, "Accounts", true))).unwrap();
        let merged = saved(second.save(change(Connection::Local, "Books", false))).unwrap();
        assert!(!merged.favorites(Connection::Local).contains("Books"));
        assert!(merged.favorites(Connection::Local).contains("Events"));
        assert!(merged.favorites(Connection::Terminal).contains("Accounts"));
    }

    #[test]
    fn failed_actions_retry_against_current_disk_contents() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("preferences.json");
        let (store, _) = PreferenceStore::open_at(path.clone()).unwrap();
        // Simulate a file becoming invalid after startup; never replace it.
        fs::write(&path, "invalid").unwrap();
        assert!(matches!(
            saved(store.save(change(Connection::Local, "Books", true))),
            Err(PreferenceError::Save)
        ));
        assert_eq!(fs::read_to_string(&path).unwrap(), "invalid");
        let mut repaired = Preferences::default();
        repaired.apply(&change(Connection::Terminal, "Other instance", true));
        save_atomic(&path, &repaired).unwrap();
        let merged = saved(store.retry()).unwrap();
        assert!(merged.favorites(Connection::Local).contains("Books"));
        assert!(
            merged
                .favorites(Connection::Terminal)
                .contains("Other instance")
        );

        // A later action also retries previous failures in click order.
        fs::write(&path, "invalid").unwrap();
        assert!(saved(store.save(change(Connection::Local, "Events", true))).is_err());
        save_atomic(&path, &merged).unwrap();
        let merged = saved(store.save(change(Connection::Local, "Books", false))).unwrap();
        assert!(merged.favorites(Connection::Local).contains("Events"));
        assert!(!merged.favorites(Connection::Local).contains("Books"));
        assert!(
            merged
                .favorites(Connection::Terminal)
                .contains("Other instance")
        );
    }

    #[test]
    #[ignore = "subprocess helper invoked by concurrent_processes_preserve_all_favorites"]
    fn process_writer() {
        let Some(path) = std::env::var_os("DDB_EXPLORER_TEST_PREF_PATH") else {
            return;
        };
        let path = PathBuf::from(path);
        let prefix = std::env::var("DDB_EXPLORER_TEST_PREF_PREFIX").expect("subprocess prefix");
        let (store, _) = PreferenceStore::open_at(path).unwrap();
        for index in 0..25 {
            drop(store.save(change(
                Connection::Local,
                &format!("{prefix}-{index}"),
                true,
            )));
        }
        // Observe any write failure, then flush the queue before the process exits.
        saved(store.retry()).unwrap();
        drop(store);
    }

    #[test]
    fn concurrent_processes_preserve_all_favorites() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("preferences.json");
        let spawn = |prefix: &str| {
            std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "preferences::tests::process_writer", "--ignored"])
                .env("DDB_EXPLORER_TEST_PREF_PATH", &path)
                .env("DDB_EXPLORER_TEST_PREF_PREFIX", prefix)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .unwrap()
        };
        let first = spawn("first");
        let second = spawn("second");
        for child in [first, second] {
            let output = child.wait_with_output().unwrap();
            assert!(
                output.status.success(),
                "Child failed: {}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        let preferences = load(&path).unwrap();
        assert_eq!(preferences.favorites(Connection::Local).len(), 50);
        for prefix in ["first", "second"] {
            for index in 0..25 {
                assert!(
                    preferences
                        .favorites(Connection::Local)
                        .contains(&format!("{prefix}-{index}"))
                );
            }
        }
    }

    #[test]
    fn invalid_or_newer_files_are_preserved() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("preferences.json");
        for content in ["invalid json", r#"{"version":2}"#] {
            fs::write(&path, content).unwrap();
            assert!(matches!(
                PreferenceStore::open_at(path.clone()),
                Err(PreferenceError::Load)
            ));
            assert_eq!(fs::read_to_string(&path).unwrap(), content);
        }
    }

    #[test]
    fn write_failures_are_reported_without_removing_the_existing_target() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("preferences.json");
        fs::create_dir(&target).unwrap();
        let marker = target.join("keep");
        fs::write(&marker, "preserve").unwrap();
        assert!(save_atomic(&target, &Preferences::default()).is_err());
        assert_eq!(fs::read_to_string(marker).unwrap(), "preserve");
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }
}
