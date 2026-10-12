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

    pub fn toggle_favorite(&mut self, connection: Connection, table: &str) {
        let favorites = match connection {
            Connection::Local => &mut self.local_favorites,
            Connection::Terminal => &mut self.terminal_favorites,
        };
        if !favorites.remove(table) {
            favorites.insert(table.to_owned());
        }
    }
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
    preferences: Preferences,
    reply: oneshot::Sender<Result<(), PreferenceError>>,
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
            .join("gpui-demo")
            .join("preferences.json");
        Self::open_at(path)
    }

    fn open_at(path: PathBuf) -> Result<(Self, Preferences), PreferenceError> {
        let preferences = match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice::<Preferences>(&bytes).map_err(|error| {
                log::warn!("Couldn't parse preferences at {}: {error}", path.display());
                PreferenceError::Load
            })?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => Preferences::default(),
            Err(error) => {
                log::warn!("Couldn't read preferences at {}: {error}", path.display());
                return Err(PreferenceError::Load);
            }
        };
        if preferences.version != 1 {
            log::warn!(
                "Unsupported preferences version {} at {}",
                preferences.version,
                path.display()
            );
            return Err(PreferenceError::Load);
        }
        let (sender, receiver) = mpsc::channel::<SaveJob>();
        // One writer preserves click order; replacing a UI task cannot cancel a save.
        let worker = thread::Builder::new()
            .name("preferences".into())
            .spawn(move || {
                for job in receiver {
                    let result = save_atomic(&path, &job.preferences).map_err(|error| {
                        log::warn!("Couldn't save preferences at {}: {error}", path.display());
                        PreferenceError::Save
                    });
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

    pub fn save(&self, preferences: Preferences) -> oneshot::Receiver<Result<(), PreferenceError>> {
        let (reply, result) = oneshot::channel();
        if self
            .sender
            .as_ref()
            .expect("store is open")
            .send(SaveJob { preferences, reply })
            .is_err()
        {
            log::warn!("Preferences writer stopped unexpectedly");
        }
        result
    }
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

    #[test]
    fn favorites_round_trip_and_remain_separate_for_each_connection() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("nested/preferences.json");
        let (store, mut preferences) = PreferenceStore::open_at(path.clone()).unwrap();
        preferences.toggle_favorite(Connection::Local, "Events");
        preferences.toggle_favorite(Connection::Terminal, "Books");
        drop(store.save(preferences));
        drop(store);
        let (store, mut reloaded) = PreferenceStore::open_at(path.clone()).unwrap();
        assert!(reloaded.favorites(Connection::Local).contains("Events"));
        assert!(!reloaded.favorites(Connection::Terminal).contains("Events"));
        assert!(reloaded.favorites(Connection::Terminal).contains("Books"));
        reloaded.toggle_favorite(Connection::Local, "Events");
        drop(store.save(reloaded));
        drop(store);
        let (_, reloaded) = PreferenceStore::open_at(path).unwrap();
        assert!(reloaded.favorites(Connection::Local).is_empty());
        assert!(reloaded.favorites(Connection::Terminal).contains("Books"));
    }

    #[test]
    fn rapid_updates_flush_in_order_even_when_receivers_are_dropped() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("preferences.json");
        let (store, mut preferences) = PreferenceStore::open_at(path.clone()).unwrap();
        for _ in 0..21 {
            preferences.toggle_favorite(Connection::Local, "Events");
            drop(store.save(preferences.clone()));
        }
        drop(store);
        let (_, reloaded) = PreferenceStore::open_at(path).unwrap();
        assert!(reloaded.favorites(Connection::Local).contains("Events"));
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
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
