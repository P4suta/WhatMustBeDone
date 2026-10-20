use crate::api::{
    AddInput, ArchiveOutput, InitOutput, Note, NoteInput, Snapshot, Status, Ticket, UpdateInput,
    text,
};
use crate::error::{Code, Error, Result};
use redb::{
    Database, DatabaseError, Durability, Key, ReadOnlyDatabase, ReadTransaction, ReadableDatabase,
    ReadableTable, ReadableTableMetadata, TableDefinition, TableHandle, Value, WriteTransaction,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::path::Path;
use std::time::{Duration, Instant};

const META: TableDefinition<&str, u64> = TableDefinition::new("meta");
const TICKETS: TableDefinition<u64, &str> = TableDefinition::new("tickets");
const NOTES: TableDefinition<u64, &str> = TableDefinition::new("notes");
const REQUESTS: TableDefinition<&str, &str> = TableDefinition::new("requests");
const SCHEMA_VERSION: u64 = 1;
const LOCK_RETRY_INTERVAL: Duration = Duration::from_millis(10);

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
enum Operation {
    Add,
    Update { ticket_id: u64 },
    Note { ticket_id: u64 },
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SavedRequest {
    operation: Operation,
    payload: String,
    response: serde_json::Value,
}

trait MutationResult {
    fn check(&self) -> Result<()>;
}

impl MutationResult for Ticket {
    fn check(&self) -> Result<()> {
        self.validate()
    }
}
impl MutationResult for Note {
    fn check(&self) -> Result<()> {
        crate::api::checked_id(self.id, "note.id")?;
        crate::api::checked_id(self.ticket_id, "note.ticket_id")?;
        text(&self.body, "body")
    }
}

pub struct Store {
    database: Database,
}

impl Store {
    pub fn open(path: &Path, wait: Duration, initialize: bool) -> Result<Self> {
        if initialize {
            if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                std::fs::create_dir_all(parent).map_err(Error::storage)?;
            }
        } else {
            require_existing(path)?;
        }
        let database = open_database(wait, || {
            if initialize {
                Database::create(path)
            } else {
                Database::open(path)
            }
        })?;
        Ok(Self { database })
    }

    pub fn initialize(&self) -> Result<InitOutput> {
        let mut transaction = self.database.begin_write().map_err(Error::storage)?;
        transaction
            .set_durability(Durability::Immediate)
            .map_err(Error::storage)?;
        check_multimaps(
            transaction
                .list_multimap_tables()
                .map_err(Error::corrupt)?
                .next()
                .is_some(),
        )?;
        {
            let mut meta = transaction.open_table(META).map_err(Error::storage)?;
            let version = meta
                .get("schema_version")
                .map_err(Error::storage)?
                .map(|v| v.value());
            match version {
                Some(version) => check_version(version)?,
                None => {
                    if meta.len().map_err(Error::storage)? != 0
                        || transaction.list_tables().map_err(Error::storage)?.count() != 1
                    {
                        return Err(Error::new(
                            Code::CorruptData,
                            "Unrecognized database; initialization refused",
                        ));
                    }
                    meta.insert("schema_version", SCHEMA_VERSION)
                        .map_err(Error::storage)?;
                    meta.insert("last_ticket", 0).map_err(Error::storage)?;
                    meta.insert("last_note", 0).map_err(Error::storage)?;
                    transaction.open_table(TICKETS).map_err(Error::storage)?;
                    transaction.open_table(NOTES).map_err(Error::storage)?;
                    transaction.open_table(REQUESTS).map_err(Error::storage)?;
                }
            }
        }
        load_write(&transaction)?;
        transaction.commit().map_err(Error::storage)?;
        Ok(InitOutput { schema_version: 1 })
    }

    fn mutate<T, F>(
        &self,
        operation: Operation,
        request_id: Option<&str>,
        payload: &impl Serialize,
        apply: F,
    ) -> Result<T>
    where
        T: Serialize + DeserializeOwned + MutationResult,
        F: FnOnce(&WriteTransaction, &mut Snapshot) -> Result<T>,
    {
        if let Some(id) = request_id {
            text(id, "request_id")?;
        }
        let payload = serde_json::to_string(payload).map_err(Error::storage)?;
        let mut transaction = self.database.begin_write().map_err(Error::storage)?;
        transaction
            .set_durability(Durability::Immediate)
            .map_err(Error::storage)?;
        let mut snapshot = load_write(&transaction)?;
        if let Some(id) = request_id {
            let table = transaction.open_table(REQUESTS).map_err(Error::corrupt)?;
            if let Some(value) = table.get(id).map_err(Error::storage)? {
                let saved: SavedRequest =
                    serde_json::from_str(value.value()).map_err(Error::corrupt)?;
                if saved.operation != operation || saved.payload != payload {
                    return Err(Error::new(
                        Code::RequestConflict,
                        "request_id was already used for different content",
                    ));
                }
                let response: T = serde_json::from_value(saved.response).map_err(Error::corrupt)?;
                response.check().map_err(Error::corrupt)?;
                return Ok(response);
            }
        }
        let result = apply(&transaction, &mut snapshot)?;
        snapshot.validate()?;
        result.check()?;
        if let Some(id) = request_id {
            let saved = SavedRequest {
                operation,
                payload,
                response: serde_json::to_value(&result).map_err(Error::storage)?,
            };
            let encoded = serde_json::to_string(&saved).map_err(Error::storage)?;
            transaction
                .open_table(REQUESTS)
                .map_err(Error::storage)?
                .insert(id, encoded.as_str())
                .map_err(Error::storage)?;
        }
        transaction.commit().map_err(Error::storage)?;
        Ok(result)
    }

    pub fn add(&self, input: AddInput) -> Result<Ticket> {
        self.mutate(
            Operation::Add,
            input.request_id.as_deref(),
            &input,
            |transaction, snapshot| {
                let id = sequence(transaction, "last_ticket")?;
                let mut ticket = Ticket {
                    id,
                    revision: 1,
                    title: input.title.clone(),
                    body: input.body.clone(),
                    status: Status::Open,
                    repos: input.repos.clone(),
                    priority: input.priority,
                    parent: input.parent,
                    depends_on: input.depends_on.clone(),
                };
                normalize(&mut ticket);
                snapshot.tickets.push(ticket.clone());
                put(transaction, TICKETS, id, &ticket)?;
                Ok(ticket)
            },
        )
    }

    pub fn update(&self, id: u64, input: UpdateInput) -> Result<Ticket> {
        self.mutate(
            Operation::Update { ticket_id: id },
            None,
            &input,
            |transaction, snapshot| {
                let current = snapshot.ticket(id)?;
                let revision =
                    wmbd_core::MatchedRevision::new(input.revision.get(), current.revision)
                        .ok_or_else(|| {
                            let mut error = Error::new(
                                Code::Conflict,
                                "Read the current ticket before updating it",
                            );
                            error.current_revision = Some(current.revision);
                            error
                        })?;
                let mut ticket = current.clone();
                if input.clear_parent && input.parent.is_some() {
                    return Err(Error::input(
                        "parent",
                        "parent and clear_parent are mutually exclusive",
                    ));
                }
                if let Some(value) = &input.title {
                    ticket.title.clone_from(value);
                }
                if let Some(value) = &input.body {
                    ticket.body.clone_from(value);
                }
                if let Some(value) = input.status {
                    ticket.status = value;
                }
                if let Some(value) = &input.repos {
                    ticket.repos.clone_from(value);
                }
                if let Some(value) = input.priority {
                    ticket.priority = value;
                }
                if let Some(value) = input.parent {
                    ticket.parent = Some(value);
                }
                if input.clear_parent {
                    ticket.parent = None;
                }
                if let Some(value) = &input.depends_on {
                    ticket.depends_on.clone_from(value);
                }
                normalize(&mut ticket);
                match revision
                    .update(ticket != *current)
                    .ok_or_else(|| Error::new(Code::Exhausted, "Sequence number exhausted"))?
                {
                    wmbd_core::RevisionChange::Unchanged => return Ok(ticket),
                    wmbd_core::RevisionChange::Changed(next) => ticket.revision = next,
                }
                let existing = snapshot
                    .tickets
                    .iter_mut()
                    .find(|t| t.id == id)
                    .ok_or_else(|| Error::new(Code::NotFound, "Ticket disappeared"))?;
                *existing = ticket.clone();
                put(transaction, TICKETS, id, &ticket)?;
                Ok(ticket)
            },
        )
    }

    pub fn note(&self, id: u64, input: NoteInput) -> Result<Note> {
        self.mutate(
            Operation::Note { ticket_id: id },
            input.request_id.as_deref(),
            &input,
            |transaction, snapshot| {
                snapshot.ticket(id)?;
                let note = Note {
                    id: sequence(transaction, "last_note")?,
                    ticket_id: id,
                    body: input.body.clone(),
                };
                snapshot.notes.push(note.clone());
                put(transaction, NOTES, note.id, &note)?;
                Ok(note)
            },
        )
    }
}

enum QueryDatabase {
    Shared(ReadOnlyDatabase),
    Recovered(Database),
}

pub(crate) fn query_snapshot(path: &Path, wait: Duration) -> Result<Snapshot> {
    require_existing(path)?;
    let database = open_database(wait, || match ReadOnlyDatabase::open(path) {
        Ok(database) => Ok(QueryDatabase::Shared(database)),
        Err(DatabaseError::RepairAborted) => Database::open(path).map(QueryDatabase::Recovered),
        Err(error) => Err(error),
    })?;
    let transaction = match &database {
        QueryDatabase::Shared(database) => database.begin_read(),
        QueryDatabase::Recovered(database) => database.begin_read(),
    }
    .map_err(Error::storage)?;
    load_read(&transaction)
}

pub fn copy_archive(source: &Path, destination: &Path, wait: Duration) -> Result<ArchiveOutput> {
    require_existing(source)?;
    let database = open_database(wait, || ReadOnlyDatabase::open(source))?;
    let transaction = database.begin_read().map_err(Error::storage)?;
    prepare_archive(&transaction, destination)?.publish()
}

struct PreparedArchive {
    file: tempfile::NamedTempFile,
    directory: std::fs::File,
    destination: std::path::PathBuf,
    output: ArchiveOutput,
}

impl PreparedArchive {
    fn publish(self) -> Result<ArchiveOutput> {
        self.publish_with_sync(std::fs::File::sync_all)
    }

    fn publish_with_sync(
        self,
        synchronize: impl FnOnce(&std::fs::File) -> std::io::Result<()>,
    ) -> Result<ArchiveOutput> {
        self.file
            .persist_noclobber(&self.destination)
            .map_err(|error| {
                if error.error.kind() == std::io::ErrorKind::AlreadyExists {
                    Error::new(
                        Code::DestinationExists,
                        "Destination already exists; choose a new path",
                    )
                } else {
                    Error::storage(error.error)
                }
            })?;
        synchronize(&self.directory).map_err(|error| {
            Error::storage(format!(
                "Archive was published, but synchronizing its directory failed: {error}"
            ))
        })?;
        Ok(self.output)
    }
}

fn prepare_archive(source: &ReadTransaction, destination: &Path) -> Result<PreparedArchive> {
    load_read(source)?;
    let destination = std::path::absolute(destination).map_err(Error::storage)?;
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let directory = prepare_directory(parent)?;
    let file = tempfile::Builder::new()
        .prefix(".wmbd-archive-")
        .tempfile_in(parent)
        .map_err(Error::storage)?;
    let database = Database::builder()
        .create_file(file.as_file().try_clone().map_err(Error::storage)?)
        .map_err(Error::storage)?;
    let mut transaction = database.begin_write().map_err(Error::storage)?;
    transaction
        .set_durability(Durability::Immediate)
        .map_err(Error::storage)?;
    copy_table(source, &transaction, META)?;
    let output = ArchiveOutput {
        schema_version: 1,
        tickets: copy_table(source, &transaction, TICKETS)?,
        notes: copy_table(source, &transaction, NOTES)?,
        requests: copy_table(source, &transaction, REQUESTS)?,
    };
    transaction.commit().map_err(Error::storage)?;
    drop(database);
    {
        let copied = ReadOnlyDatabase::open(file.path()).map_err(Error::storage)?;
        load_read(&copied.begin_read().map_err(Error::storage)?)?;
    }
    file.as_file().sync_all().map_err(Error::storage)?;
    Ok(PreparedArchive {
        file,
        directory,
        destination,
        output,
    })
}

fn prepare_directory(mut path: &Path) -> Result<std::fs::File> {
    let mut missing = Vec::new();
    let mut directory = loop {
        match open_directory(path) {
            Ok(directory) => break directory,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                missing.push(path);
                path = path
                    .parent()
                    .ok_or_else(|| Error::storage("No existing destination ancestor"))?;
            }
            Err(error) => return Err(Error::storage(error)),
        }
    };
    for path in missing.into_iter().rev() {
        match std::fs::create_dir(path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(Error::storage(error)),
        }
        let created = open_directory(path).map_err(Error::storage)?;
        created.sync_all().map_err(Error::storage)?;
        directory.sync_all().map_err(Error::storage)?;
        directory = created;
    }
    Ok(directory)
}

fn open_directory(path: &Path) -> std::io::Result<std::fs::File> {
    let directory = std::fs::File::open(path)?;
    if directory.metadata()?.is_dir() {
        Ok(directory)
    } else {
        Err(std::io::ErrorKind::NotADirectory.into())
    }
}

fn copy_table<K: Key + 'static, V: Value + 'static>(
    source: &ReadTransaction,
    destination: &WriteTransaction,
    definition: TableDefinition<K, V>,
) -> Result<u64> {
    let input = source.open_table(definition).map_err(Error::corrupt)?;
    let mut output = destination.open_table(definition).map_err(Error::storage)?;
    for entry in input.iter().map_err(Error::corrupt)? {
        let (key, value) = entry.map_err(Error::corrupt)?;
        output
            .insert(key.value(), value.value())
            .map_err(Error::storage)?;
    }
    input.len().map_err(Error::corrupt)
}

fn require_existing(path: &Path) -> Result<()> {
    match std::fs::metadata(path) {
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Err(Error::new(Code::NotInitialized, "Run wmbd init first"))
        }
        Err(error) => Err(Error::storage(error)),
    }
}

fn open_database<T>(
    wait: Duration,
    mut open: impl FnMut() -> std::result::Result<T, DatabaseError>,
) -> Result<T> {
    let start = Instant::now();
    loop {
        match open() {
            Ok(database) => return Ok(database),
            Err(DatabaseError::DatabaseAlreadyOpen) => {
                let elapsed = start.elapsed();
                if elapsed >= wait {
                    let mut error =
                        Error::new(Code::DatabaseBusy, "Database is in use; retry the command");
                    error.retryable = true;
                    return Err(error);
                }
                std::thread::sleep(LOCK_RETRY_INTERVAL.min(wait.saturating_sub(elapsed)));
            }
            Err(DatabaseError::UpgradeRequired(_)) => {
                return Err(Error::new(
                    Code::UnsupportedVersion,
                    "The redb file format needs an explicit upgrade",
                ));
            }
            Err(DatabaseError::RepairAborted) => {
                return Err(Error::new(
                    Code::RecoveryRequired,
                    "Database needs recovery; run wmbd --db PATH list on the writable source before retrying",
                ));
            }
            Err(error) => return Err(Error::storage(error)),
        }
    }
}

fn normalize(ticket: &mut Ticket) {
    ticket.repos.sort();
    ticket.repos.dedup();
    ticket.depends_on.sort_unstable();
    ticket.depends_on.dedup();
}

fn increment(value: u64) -> Result<u64> {
    wmbd_core::next_sequence(value)
        .ok_or_else(|| Error::new(Code::Exhausted, "Sequence number exhausted"))
}

fn sequence(transaction: &WriteTransaction, name: &str) -> Result<u64> {
    let mut meta = transaction.open_table(META).map_err(Error::corrupt)?;
    let current = meta
        .get(name)
        .map_err(Error::storage)?
        .ok_or_else(|| Error::new(Code::CorruptData, "Missing sequence"))?
        .value();
    let next = increment(current)?;
    meta.insert(name, next).map_err(Error::storage)?;
    Ok(next)
}

fn check_version(version: u64) -> Result<()> {
    if version == SCHEMA_VERSION {
        Ok(())
    } else {
        Err(Error::new(
            Code::UnsupportedVersion,
            format!("Unsupported application schema version {version}"),
        ))
    }
}

fn check_metadata(table: &impl ReadableTable<&'static str, u64>) -> Result<()> {
    let version = table
        .get("schema_version")
        .map_err(Error::corrupt)?
        .ok_or_else(|| Error::new(Code::CorruptData, "Missing schema version"))?
        .value();
    check_version(version)?;
    for name in ["last_ticket", "last_note"] {
        if table.get(name).map_err(Error::corrupt)?.is_none() {
            return Err(Error::new(Code::CorruptData, "Missing sequence"));
        }
    }
    Ok(())
}

fn records<T: DeserializeOwned>(
    table: &impl ReadableTable<u64, &'static str>,
) -> Result<Vec<(u64, T)>> {
    table
        .iter()
        .map_err(Error::storage)?
        .map(|entry| {
            let (key, value) = entry.map_err(Error::storage)?;
            Ok((
                key.value(),
                serde_json::from_str(value.value()).map_err(Error::corrupt)?,
            ))
        })
        .collect()
}

fn validate_snapshot(tickets: Vec<(u64, Ticket)>, notes: Vec<(u64, Note)>) -> Result<Snapshot> {
    if tickets.iter().any(|(key, t)| *key != t.id) || notes.iter().any(|(key, n)| *key != n.id) {
        return Err(Error::new(
            Code::CorruptData,
            "Record key does not match its ID",
        ));
    }
    let snapshot = Snapshot {
        tickets: tickets.into_iter().map(|(_, t)| t).collect(),
        notes: notes.into_iter().map(|(_, n)| n).collect(),
    };
    snapshot.validate().map_err(Error::corrupt)?;
    Ok(snapshot)
}

fn load_read(transaction: &ReadTransaction) -> Result<Snapshot> {
    check_tables(
        transaction
            .list_tables()
            .map_err(Error::corrupt)?
            .map(|table| table.name().to_owned())
            .collect(),
    )?;
    check_multimaps(
        transaction
            .list_multimap_tables()
            .map_err(Error::corrupt)?
            .next()
            .is_some(),
    )?;
    transaction.open_table(REQUESTS).map_err(Error::corrupt)?;
    let meta = transaction.open_table(META).map_err(Error::corrupt)?;
    check_metadata(&meta)?;
    let tickets = records(&transaction.open_table(TICKETS).map_err(Error::corrupt)?)?;
    let notes = records(&transaction.open_table(NOTES).map_err(Error::corrupt)?)?;
    let state = validate_snapshot(tickets, notes)?;
    check_counters(&meta, &state)?;
    Ok(state)
}

fn load_write(transaction: &WriteTransaction) -> Result<Snapshot> {
    check_tables(
        transaction
            .list_tables()
            .map_err(Error::corrupt)?
            .map(|table| table.name().to_owned())
            .collect(),
    )?;
    check_multimaps(
        transaction
            .list_multimap_tables()
            .map_err(Error::corrupt)?
            .next()
            .is_some(),
    )?;
    transaction.open_table(REQUESTS).map_err(Error::corrupt)?;
    check_metadata(&transaction.open_table(META).map_err(Error::corrupt)?)?;
    let tickets = records(&transaction.open_table(TICKETS).map_err(Error::corrupt)?)?;
    let notes = records(&transaction.open_table(NOTES).map_err(Error::corrupt)?)?;
    let state = validate_snapshot(tickets, notes)?;
    let meta = transaction.open_table(META).map_err(Error::corrupt)?;
    check_counters(&meta, &state)?;
    Ok(state)
}

fn check_tables(mut names: Vec<String>) -> Result<()> {
    names.sort();
    if names != ["meta", "notes", "requests", "tickets"] {
        return Err(Error::new(
            Code::CorruptData,
            "Database tables are missing or unrecognized",
        ));
    }
    Ok(())
}

fn check_multimaps(present: bool) -> Result<()> {
    if present {
        Err(Error::new(Code::CorruptData, "Unexpected multimap table"))
    } else {
        Ok(())
    }
}

fn check_counters(meta: &impl ReadableTable<&'static str, u64>, state: &Snapshot) -> Result<()> {
    for (name, max_id, count) in [
        (
            "last_ticket",
            state.tickets.iter().map(|t| t.id).max(),
            state.tickets.len(),
        ),
        (
            "last_note",
            state.notes.iter().map(|n| n.id).max(),
            state.notes.len(),
        ),
    ] {
        let last = meta
            .get(name)
            .map_err(Error::corrupt)?
            .ok_or_else(|| Error::new(Code::CorruptData, "Missing sequence"))?
            .value();
        if max_id.unwrap_or(0) != last || u64::try_from(count).map_err(Error::corrupt)? != last {
            return Err(Error::new(
                Code::CorruptData,
                "Stored records do not match their sequence",
            ));
        }
    }
    Ok(())
}

fn put(
    transaction: &WriteTransaction,
    table: TableDefinition<u64, &str>,
    id: u64,
    value: &impl Serialize,
) -> Result<()> {
    let encoded = serde_json::to_string(value).map_err(Error::storage)?;
    transaction
        .open_table(table)
        .map_err(Error::storage)?
        .insert(id, encoded.as_str())
        .map_err(Error::storage)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, Read, Write};
    use std::process::{Command, Stdio};

    type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;

    fn source(path: &Path) -> TestResult {
        let store = Store::open(path, Duration::ZERO, true)?;
        store.initialize()?;
        store.add(AddInput {
            title: "Keep this work".into(),
            ..AddInput::default()
        })?;
        Ok(())
    }

    #[test]
    fn staged_archives_clean_up_and_preserve_a_destination_created_before_publication() -> TestResult
    {
        let temp = tempfile::tempdir()?;
        let source_path = temp.path().join("source.redb");
        source(&source_path)?;
        let reader = ReadOnlyDatabase::open(&source_path)?;
        let transaction = reader.begin_read()?;
        let destination = temp.path().join("new/archive.redb");
        let prepared = prepare_archive(&transaction, &destination)?;
        let staging = prepared.file.path().to_owned();
        assert!(staging.exists());
        assert!(!destination.exists());
        drop(prepared);
        assert!(!staging.exists());
        assert!(!destination.exists());
        let prepared = prepare_archive(&transaction, &destination)?;
        let staging = prepared.file.path().to_owned();
        std::fs::write(&destination, b"another publisher's file")?;
        assert!(matches!(
            prepared.publish(),
            Err(Error {
                code: Code::DestinationExists,
                ..
            })
        ));
        assert_eq!(std::fs::read(&destination)?, b"another publisher's file");
        assert!(!staging.exists());
        Ok(())
    }

    #[test]
    fn archive_stage_child() -> TestResult {
        let Some(root) = std::env::var_os("WMBD_ARCHIVE_CHILD_ROOT") else {
            return Ok(());
        };
        let root = std::path::PathBuf::from(root);
        let reader = ReadOnlyDatabase::open(root.join("source.redb"))?;
        let transaction = reader.begin_read()?;
        let _prepared = prepare_archive(&transaction, &root.join("archive.redb"))?;
        println!("archive-staged");
        std::io::stdout().flush()?;
        std::io::stdin().read_exact(&mut [0_u8; 1])?;
        std::process::exit(31);
    }

    #[test]
    fn failed_directory_sync_reports_the_already_published_archive() -> TestResult {
        let temp = tempfile::tempdir()?;
        let source_path = temp.path().join("source.redb");
        source(&source_path)?;
        let reader = ReadOnlyDatabase::open(&source_path)?;
        let transaction = reader.begin_read()?;
        let destination = temp.path().join("archive.redb");
        let prepared = prepare_archive(&transaction, &destination)?;
        let error = prepared
            .publish_with_sync(|_| Err(std::io::Error::other("injected directory sync failure")))
            .err()
            .ok_or("missing synchronization error")?;
        assert!(matches!(error.code, Code::Storage));
        assert!(error.message.contains("Archive was published"));
        assert!(error.message.contains("injected directory sync failure"));
        let copied = ReadOnlyDatabase::open(&destination)?;
        assert_eq!(
            load_read(&copied.begin_read()?)?.ticket(1)?.title,
            "Keep this work"
        );
        Ok(())
    }

    #[test]
    fn abrupt_exit_before_publication_never_exposes_the_archive() -> TestResult {
        let temp = tempfile::tempdir()?;
        let source_path = temp.path().join("source.redb");
        source(&source_path)?;
        let before = std::fs::read(&source_path)?;
        let destination = temp.path().join("archive.redb");
        let mut child = Command::new(std::env::current_exe()?)
            .args([
                "--exact",
                "store::tests::archive_stage_child",
                "--nocapture",
            ])
            .env("WMBD_ARCHIVE_CHILD_ROOT", temp.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()?;
        let mut output =
            std::io::BufReader::new(child.stdout.take().ok_or("missing child stdout")?);
        loop {
            let mut line = String::new();
            if output.read_line(&mut line)? == 0 {
                return Err("child exited before staging the archive".into());
            }
            if line.trim() == "archive-staged" {
                break;
            }
        }
        assert!(!destination.exists());
        child
            .stdin
            .take()
            .ok_or("missing child stdin")?
            .write_all(b"x")?;
        assert_eq!(child.wait()?.code(), Some(31));
        assert!(!destination.exists());
        assert_eq!(std::fs::read(&source_path)?, before);
        copy_archive(&source_path, &destination, Duration::ZERO)?;
        assert!(destination.exists());
        Ok(())
    }
}
