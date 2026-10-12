//! A single worker serializes file operations and keeps disk work off the UI.
use super::models::Entry;
use serde_json::Value;
use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::mpsc,
    thread::{self, JoinHandle},
};
use tokio::sync::oneshot;

pub enum Command {
    List,
    Save {
        entry: Box<Entry>,
        payload: Option<Value>,
        create: bool,
    },
    Delete {
        ids: Vec<String>,
    },
    Payload(String),
}

pub struct Listing {
    pub entries: Vec<Entry>,
    pub skipped_files: usize,
}

pub enum Reply {
    Entries(Listing),
    Payload(Value),
}
pub type Result = std::result::Result<Reply, String>;
type Job = (Command, oneshot::Sender<Result>);

pub struct HistoryStore {
    sender: Option<mpsc::Sender<Job>>,
    worker: Option<JoinHandle<()>>,
}

impl HistoryStore {
    pub fn open() -> io::Result<Self> {
        let path = dirs::data_local_dir()
            .ok_or_else(|| io::Error::other("History directory unavailable"))?
            .join("ddb-explorer/history");
        Self::open_at(path)
    }

    pub(super) fn open_at(path: PathBuf) -> io::Result<Self> {
        let (sender, receiver) = mpsc::channel::<Job>();
        let worker = thread::Builder::new().name("history".into()).spawn(move || {
            for (command, reply) in receiver {
                let result = execute(&path, command).map_err(|error| {
                    log::warn!("History operation failed: {error}");
                    "History couldn't be read or saved. Check access to the app's data folder and retry.".to_owned()
                });
                let _ = reply.send(result);
            }
        })?;
        Ok(Self {
            sender: Some(sender),
            worker: Some(worker),
        })
    }

    pub fn send(&self, command: Command) -> oneshot::Receiver<Result> {
        let (reply, receiver) = oneshot::channel();
        let _ = self
            .sender
            .as_ref()
            .expect("open store")
            .send((command, reply));
        receiver
    }
}

impl Drop for HistoryStore {
    fn drop(&mut self) {
        self.sender.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn file(path: &Path, id: &str, suffix: &str) -> io::Result<PathBuf> {
    // IDs originate in files too; never allow a corrupt entry to escape this folder.
    if id.is_empty() || !id.bytes().all(|byte| byte.is_ascii_digit() || byte == b'-') {
        return Err(io::Error::other("Invalid history ID"));
    }
    Ok(path.join(format!("{id}.{suffix}.json")))
}

fn write_atomic<T: serde::Serialize>(path: &Path, value: &T) -> io::Result<()> {
    let directory = path.parent().expect("history file has parent");
    fs::create_dir_all(directory)?;
    let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
    serde_json::to_writer(&mut temporary, value)?;
    temporary.as_file().sync_all()?;
    temporary.persist(path).map_err(|error| error.error)?;
    Ok(())
}

fn read_entry(path: &Path, entry_path: &Path) -> io::Result<Entry> {
    let entry: Entry = serde_json::from_slice(&fs::read(entry_path)?)?;
    if entry.version != 1 {
        return Err(io::Error::other("Unsupported history version"));
    }
    if file(path, &entry.id, "entry")? != entry_path {
        return Err(io::Error::other("History ID mismatch"));
    }
    Ok(entry)
}

fn list(path: &Path) -> io::Result<Listing> {
    let files = match fs::read_dir(path) {
        Ok(files) => files,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(Listing {
                entries: Vec::new(),
                skipped_files: 0,
            });
        }
        Err(error) => return Err(error),
    };
    let mut entries = Vec::new();
    let mut skipped_files = 0;
    for file in files {
        let file = match file {
            Ok(file) => file,
            Err(error) => {
                log::warn!("Couldn't inspect a history file: {error}");
                skipped_files += 1;
                continue;
            }
        };
        if !file.file_name().to_string_lossy().ends_with(".entry.json") {
            continue;
        }
        match read_entry(path, &file.path()) {
            Ok(entry) => entries.push(entry),
            Err(error) => {
                log::warn!("Skipping history file {}: {error}", file.path().display());
                skipped_files += 1;
            }
        }
    }
    entries.sort_by(|a, b| {
        b.created_ms
            .cmp(&a.created_ms)
            .then_with(|| b.id.cmp(&a.id))
    });
    Ok(Listing {
        entries,
        skipped_files,
    })
}

fn remove(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}

fn execute(path: &Path, command: Command) -> io::Result<Reply> {
    match command {
        Command::List => {}
        Command::Save {
            entry,
            payload,
            create,
        } => {
            let metadata = file(path, &entry.id, "entry")?;
            // Pagination must not recreate an entry the user already deleted.
            if create || metadata.exists() {
                if let Some(payload) = payload {
                    write_atomic(&file(path, &entry.id, "item")?, &payload)?;
                }
                write_atomic(&metadata, &entry)?;
            }
        }
        Command::Delete { ids } => {
            for id in ids {
                remove(&file(path, &id, "entry")?)?;
                remove(&file(path, &id, "item")?)?;
            }
        }
        Command::Payload(id) => {
            return Ok(Reply::Payload(serde_json::from_slice(&fs::read(file(
                path, &id, "item",
            )?)?)?));
        }
    }
    Ok(Reply::Entries(list(path)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dynamodb::Connection;

    #[test]
    fn snapshots_round_trip_and_deleted_queries_cannot_be_resurrected() {
        let dir = tempfile::tempdir().unwrap();
        let mut entry = Entry::new(Connection::Local, "Books".into(), None);
        entry.snapshot = true;
        let payload = serde_json::json!({"title": "old value"});
        execute(
            dir.path(),
            Command::Save {
                entry: Box::new(entry.clone()),
                payload: Some(payload.clone()),
                create: true,
            },
        )
        .unwrap();
        assert_eq!(list(dir.path()).unwrap().entries.len(), 1);
        let Reply::Payload(loaded) =
            execute(dir.path(), Command::Payload(entry.id.clone())).unwrap()
        else {
            panic!()
        };
        assert_eq!(loaded, payload);
        execute(
            dir.path(),
            Command::Delete {
                ids: vec![entry.id.clone()],
            },
        )
        .unwrap();
        execute(
            dir.path(),
            Command::Save {
                entry: Box::new(entry),
                payload: None,
                create: false,
            },
        )
        .unwrap();
        assert!(list(dir.path()).unwrap().entries.is_empty());
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn rejects_paths_outside_history_and_preserves_corrupt_files() {
        let dir = tempfile::tempdir().unwrap();
        assert!(file(dir.path(), "../escape", "entry").is_err());
        let path = dir.path().join("1.entry.json");
        fs::write(&path, "broken").unwrap();
        let listing = list(dir.path()).unwrap();
        assert!(listing.entries.is_empty());
        assert_eq!(listing.skipped_files, 1);
        assert_eq!(fs::read_to_string(path).unwrap(), "broken");
    }

    #[test]
    fn bad_entries_do_not_block_listing_saving_or_deleting_healthy_snapshots() {
        let dir = tempfile::tempdir().unwrap();
        let mut fixtures = vec![(dir.path().join("1.entry.json"), b"broken".to_vec())];
        for (filename, id, version) in [("2", "2", 99), ("3", "../escape", 1), ("4", "5", 1)] {
            let mut entry = Entry::new(Connection::Local, "Books".into(), None);
            entry.id = id.into();
            entry.version = version;
            fixtures.push((
                dir.path().join(format!("{filename}.entry.json")),
                serde_json::to_vec(&entry).unwrap(),
            ));
        }
        for (path, bytes) in &fixtures {
            fs::write(path, bytes).unwrap();
        }
        let mut healthy = Entry::new(Connection::Local, "Books".into(), None);
        healthy.snapshot = true;
        let payload = serde_json::json!({"value": "historical"});
        let Reply::Entries(listing) = execute(
            dir.path(),
            Command::Save {
                entry: Box::new(healthy.clone()),
                payload: Some(payload.clone()),
                create: true,
            },
        )
        .unwrap() else {
            panic!()
        };
        assert_eq!(listing.entries.len(), 1);
        assert_eq!(listing.entries[0].id, healthy.id);
        assert_eq!(listing.skipped_files, fixtures.len());
        let Reply::Payload(loaded) =
            execute(dir.path(), Command::Payload(healthy.id.clone())).unwrap()
        else {
            panic!()
        };
        assert_eq!(loaded, payload);
        let Reply::Entries(listing) = execute(
            dir.path(),
            Command::Delete {
                ids: vec![healthy.id.clone()],
            },
        )
        .unwrap() else {
            panic!()
        };
        assert!(listing.entries.is_empty());
        assert_eq!(listing.skipped_files, fixtures.len());
        assert!(!file(dir.path(), &healthy.id, "item").unwrap().exists());
        for (path, bytes) in &fixtures {
            assert_eq!(&fs::read(path).unwrap(), bytes);
        }
    }

    #[test]
    fn worker_flushes_on_exit_and_queries_remain_independent_of_snapshots() {
        let dir = tempfile::tempdir().unwrap();
        let query = Entry::new(Connection::Local, "Books".into(), None);
        let mut snapshot = Entry::new(Connection::Local, "Books".into(), None);
        snapshot.snapshot = true;
        let store = HistoryStore::open_at(dir.path().to_owned()).unwrap();
        drop(store.send(Command::Save {
            entry: Box::new(query.clone()),
            payload: None,
            create: true,
        }));
        drop(store.send(Command::Save {
            entry: Box::new(snapshot.clone()),
            payload: Some(serde_json::json!({"id": "saved"})),
            create: true,
        }));
        drop(store.send(Command::Delete {
            ids: vec![query.id],
        }));
        drop(store);
        let entries = list(dir.path()).unwrap().entries;
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, snapshot.id);
        assert!(file(dir.path(), &snapshot.id, "item").unwrap().exists());
    }
}
