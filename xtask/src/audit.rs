use crate::Result;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::io::{BufRead, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use wmbd_core::AuditCoverage;

const MAX_FILES: usize = 4096;
const MAX_BLOB: usize = 1024 * 1024;
const MAX_CORPUS: usize = 64 * 1024 * 1024;
const MAX_TREE: usize = 4 * 1024 * 1024;
const SHA1_HEX_BYTES: usize = 40;
const MAX_PATH_COMPONENTS: usize = 8;
const MAX_DIRECTORY_DEPTH: usize = 8;
const MAX_INVENTORY_ENTRIES: usize = MAX_FILES * (MAX_DIRECTORY_DEPTH + 1);
const REVISION_OUTPUT_BYTES: usize = 128;
const BATCH_HEADER_BYTES: usize = 128;
const INFORMATIONAL_WARNINGS: [&str; 3] = ["unmaintained", "unsound", "notice"];
const AUDIT_SETTINGS_COUNT: usize = 5;
#[cfg(unix)]
const PRIVATE_FILE_MODE: u32 = 0o600;

#[derive(Debug)]
struct Entry {
    path: String,
    object: String,
    size: usize,
}

struct PreparedSnapshot {
    directory: tempfile::TempDir,
    entries: Vec<Entry>,
    digest: String,
}

pub fn run(root: &Path, repository: &Path, revision: &str) -> Result<Value> {
    let snapshot = prepare(repository, revision)?;
    let digest = snapshot_digest(&snapshot)?;
    if digest != snapshot.digest {
        return Err("advisory snapshot changed before auditing".into());
    }
    eprintln!("Auditing the immutable advisory snapshot");
    let report = bounded(
        crate::cargo(
            root,
            &["audit", "--no-fetch", "--no-yanked", "--json", "--db"],
        )
        .arg(snapshot.directory.path().join("database"))
        .arg("--file")
        .arg(root.join("Cargo.lock"))
        .stdin(Stdio::null()),
        MAX_CORPUS,
    )?;
    let report: Value = serde_json::from_slice(&report)?;
    validate_report(&report)?;
    if snapshot_digest(&snapshot)? != snapshot.digest {
        return Err("advisory snapshot changed during auditing".into());
    }
    Ok(json!({
        "advisory_revision": revision,
        "snapshot_sha256": snapshot.digest,
        "freshness": "not_evaluated",
        "result": report,
    }))
}

fn object_id(value: &str) -> bool {
    value.len() == SHA1_HEX_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn git(repository: &Path) -> Command {
    let mut command = Command::new("git");
    command
        .args([
            "--no-replace-objects",
            "--no-lazy-fetch",
            "--literal-pathspecs",
            "-C",
        ])
        .arg(repository)
        .stdin(Stdio::null());
    for name in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_COMMON_DIR",
        "GIT_INDEX_FILE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_NAMESPACE",
        "GIT_REPLACE_REF_BASE",
        "GIT_CONFIG",
        "GIT_CONFIG_COUNT",
        "GIT_CONFIG_PARAMETERS",
    ] {
        command.env_remove(name);
    }
    command.env("GIT_OPTIONAL_LOCKS", "0");
    command.env("GIT_NO_LAZY_FETCH", "1");
    command
}

fn bounded(command: &mut Command, limit: usize) -> Result<Vec<u8>> {
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()?;
    let mut bytes = Vec::new();
    let read = child
        .stdout
        .take()
        .ok_or("missing verification output pipe")?
        .take(u64::try_from(
            limit.checked_add(1).ok_or("output bound overflow")?,
        )?)
        .read_to_end(&mut bytes);
    if read.is_err() || bytes.len() > limit {
        let stopped = child.kill();
        let reaped = child.wait();
        read?;
        reaped?;
        if let Err(error) = stopped {
            eprintln!("Output-bound stop request returned {error}; child was reaped");
        }
        return Err("verification output exceeded its configured input bound".into());
    }
    let status = child.wait()?;
    if !status.success() {
        if !bytes.is_empty() {
            eprintln!("{}", String::from_utf8_lossy(&bytes));
        }
        return Err(format!("verification command exited with {status}").into());
    }
    Ok(bytes)
}

fn parse_tree(bytes: &[u8]) -> Result<Vec<Entry>> {
    let bytes = bytes
        .strip_suffix(&[0])
        .ok_or("empty or unterminated Git tree")?;
    let mut entries = Vec::new();
    let mut paths = BTreeSet::new();
    let mut total = 0_usize;
    for record in bytes.split(|byte| *byte == 0) {
        let record = std::str::from_utf8(record)?;
        let (header, path) = record.split_once('\t').ok_or("invalid Git tree record")?;
        let fields: Vec<_> = header.split_ascii_whitespace().collect();
        let [mode, kind, object, size] = fields.as_slice() else {
            return Err("invalid Git tree header".into());
        };
        if !matches!(*mode, "100644" | "100755") || *kind != "blob" || !object_id(object) {
            return Err("advisory tree contains a nonregular blob or invalid object".into());
        }
        let components: Vec<_> = path.split('/').collect();
        if !matches!(components.first(), Some(&"crates" | &"rust"))
            || !(2..=MAX_PATH_COMPONENTS).contains(&components.len())
            || path.contains('\\')
            || components.iter().any(|part| {
                part.is_empty()
                    || matches!(*part, "." | "..")
                    || part.contains('\n')
                    || part.contains('\r')
            })
            || !paths.insert(path.to_owned())
        {
            return Err("advisory tree contains an invalid or repeated path".into());
        }
        let size: usize = size.parse()?;
        total = total
            .checked_add(size)
            .ok_or("advisory corpus size overflow")?;
        if entries.len() == MAX_FILES || size > MAX_BLOB || total > MAX_CORPUS {
            return Err("advisory tree exceeded its configured input bound".into());
        }
        entries.push(Entry {
            path: path.to_owned(),
            object: (*object).to_owned(),
            size,
        });
    }
    if entries.is_empty() {
        return Err("advisory tree is empty".into());
    }
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(entries)
}

fn write_batch(directory: &Path, entries: &[Entry], bytes: &[u8]) -> Result<String> {
    let database = directory.join("database");
    let mut cursor = std::io::Cursor::new(bytes);
    let mut digest = Sha256::new();
    for entry in entries {
        let mut header = Vec::new();
        cursor.read_until(b'\n', &mut header)?;
        if header != format!("{} blob {}\n", entry.object, entry.size).as_bytes() {
            return Err("Git batch returned an unexpected object header".into());
        }
        let mut content = vec![0; entry.size];
        cursor.read_exact(&mut content)?;
        let mut separator = [0];
        cursor.read_exact(&mut separator)?;
        if separator != *b"\n" {
            return Err("Git batch returned an invalid object terminator".into());
        }
        let destination = database.join(&entry.path);
        std::fs::create_dir_all(destination.parent().ok_or("missing advisory parent")?)?;
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(PRIVATE_FILE_MODE);
        }
        options.open(destination)?.write_all(&content)?;
        hash_entry(&mut digest, &entry.path, &content)?;
    }
    if cursor.position() != u64::try_from(bytes.len())? {
        return Err("Git batch returned trailing objects or bytes".into());
    }
    Ok(hex(digest.finalize()))
}

fn prepare(repository: &Path, revision: &str) -> Result<PreparedSnapshot> {
    if !object_id(revision) {
        return Err("advisory revision must be 40 lowercase hexadecimal characters".into());
    }
    let actual = bounded(
        git(repository)
            .args(["rev-parse", "--verify", "--end-of-options"])
            .arg(format!("{revision}^{{commit}}")),
        REVISION_OUTPUT_BYTES,
    )?;
    if actual != format!("{revision}\n").as_bytes() {
        return Err("advisory revision did not resolve to the pinned commit".into());
    }
    let entries = parse_tree(&bounded(
        git(repository).args([
            "ls-tree",
            "-r",
            "-z",
            "-l",
            "--full-tree",
            revision,
            "crates",
            "rust",
        ]),
        MAX_TREE,
    )?)?;
    let directory = tempfile::Builder::new()
        .prefix("wmbd-advisories-")
        .tempdir()?;
    let request = directory.path().join("objects");
    let mut input = std::fs::File::create(&request)?;
    let mut limit = 0_usize;
    for entry in &entries {
        writeln!(input, "{}", entry.object)?;
        limit = limit
            .checked_add(entry.size)
            .and_then(|size| size.checked_add(BATCH_HEADER_BYTES))
            .ok_or("batch output size overflow")?;
    }
    input.flush()?;
    drop(input);
    eprintln!("Reading {} committed advisory files", entries.len());
    let bytes = bounded(
        git(repository)
            .args(["cat-file", "--batch"])
            .stdin(std::fs::File::open(request)?),
        limit,
    )?;
    let digest = write_batch(directory.path(), &entries, &bytes)?;
    Ok(PreparedSnapshot {
        directory,
        entries,
        digest,
    })
}

fn inventory(
    directory: &Path,
    depth: usize,
    visited: &mut usize,
    paths: &mut BTreeSet<PathBuf>,
) -> Result<()> {
    if depth > MAX_DIRECTORY_DEPTH {
        return Err("advisory snapshot directory nesting exceeded its bound".into());
    }
    for entry in std::fs::read_dir(directory)? {
        *visited = visited
            .checked_add(1)
            .ok_or("snapshot entry count overflow")?;
        if *visited > MAX_INVENTORY_ENTRIES {
            return Err("advisory snapshot entry count exceeded its bound".into());
        }
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_dir() {
            inventory(&entry.path(), depth + 1, visited, paths)?;
        } else if !kind.is_file() || !paths.insert(entry.path()) || paths.len() > MAX_FILES {
            return Err("advisory snapshot contains unexpected filesystem entries".into());
        }
    }
    Ok(())
}

fn snapshot_digest(snapshot: &PreparedSnapshot) -> Result<String> {
    let database = snapshot.directory.path().join("database");
    let mut paths = BTreeSet::new();
    inventory(&database, 0, &mut 0, &mut paths)?;
    let expected: BTreeSet<_> = snapshot
        .entries
        .iter()
        .map(|e| database.join(&e.path))
        .collect();
    if paths != expected {
        return Err("advisory snapshot file inventory changed".into());
    }
    let mut digest = Sha256::new();
    for entry in &snapshot.entries {
        let path = database.join(&entry.path);
        if std::fs::metadata(&path)?.len() != u64::try_from(entry.size)? {
            return Err("advisory snapshot file size changed".into());
        }
        let mut bytes = Vec::new();
        std::fs::File::open(&path)?
            .take(u64::try_from(entry.size)? + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() != entry.size {
            return Err("advisory snapshot file size changed while reading".into());
        }
        hash_entry(&mut digest, &entry.path, &bytes)?;
    }
    Ok(hex(digest.finalize()))
}

fn hash_entry(digest: &mut Sha256, path: &str, content: &[u8]) -> Result<()> {
    digest.update(u64::try_from(path.len())?.to_be_bytes());
    digest.update(path.as_bytes());
    digest.update(u64::try_from(content.len())?.to_be_bytes());
    digest.update(content);
    Ok(())
}

fn hex(bytes: impl AsRef<[u8]>) -> String {
    bytes
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn validate_report(report: &Value) -> Result<()> {
    let empty_array = |pointer| {
        report
            .pointer(pointer)
            .and_then(Value::as_array)
            .is_some_and(Vec::is_empty)
    };
    let warnings = report
        .pointer("/settings/informational_warnings")
        .and_then(Value::as_array);
    let complete_warnings = warnings.is_some_and(|items| {
        items.len() == INFORMATIONAL_WARNINGS.len()
            && INFORMATIONAL_WARNINGS.iter().all(|required| {
                items
                    .iter()
                    .filter(|item| item.as_str() == Some(*required))
                    .count()
                    == 1
            })
    });
    let coverage = AuditCoverage {
        advisories: report
            .pointer("/database/advisory-count")
            .and_then(Value::as_u64)
            .ok_or("missing advisory count")?,
        dependencies: report
            .pointer("/lockfile/dependency-count")
            .and_then(Value::as_u64)
            .ok_or("missing dependency count")?,
        vulnerabilities: report
            .pointer("/vulnerabilities/count")
            .and_then(Value::as_u64)
            .ok_or("missing vulnerability count")?,
        settings_complete: empty_array("/settings/ignore")
            && empty_array("/settings/target_arch")
            && empty_array("/settings/target_os")
            && report.pointer("/settings/severity") == Some(&Value::Null)
            && report
                .get("settings")
                .and_then(Value::as_object)
                .is_some_and(|settings| settings.len() == AUDIT_SETTINGS_COUNT)
            && complete_warnings,
        findings_empty: report
            .pointer("/vulnerabilities/found")
            .and_then(Value::as_bool)
            == Some(false)
            && empty_array("/vulnerabilities/list")
            && report
                .get("warnings")
                .and_then(Value::as_object)
                .is_some_and(serde_json::Map::is_empty),
    };
    if !coverage.accepted() {
        return Err("dependency audit is empty, filtered, or contains findings".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clean_report() -> Value {
        json!({
            "database": {"advisory-count": 1},
            "lockfile": {"dependency-count": 1},
            "settings": {
                "ignore": [], "severity": null, "target_arch": [], "target_os": [],
                "informational_warnings": ["unmaintained", "unsound", "notice"]
            },
            "vulnerabilities": {"count": 0, "found": false, "list": []},
            "warnings": {}
        })
    }

    #[test]
    fn audit_report_requires_complete_unfiltered_empty_findings() -> Result<()> {
        assert!(validate_report(&clean_report()).is_ok());
        for (pointer, value) in [
            ("/database/advisory-count", json!(0)),
            ("/database/advisory-count", json!(-1)),
            ("/lockfile/dependency-count", json!(0)),
            ("/lockfile/dependency-count", Value::Null),
            ("/vulnerabilities/count", json!(1)),
            ("/vulnerabilities/count", json!("0")),
            ("/vulnerabilities/found", json!(true)),
            ("/vulnerabilities/found", Value::Null),
            ("/vulnerabilities/list", json!([{}])),
            ("/vulnerabilities/list", Value::Null),
            ("/warnings", json!({"unsound": [{}]})),
            ("/warnings", json!([])),
            ("/settings/ignore", json!(["RUSTSEC-2026-0001"])),
            ("/settings/ignore", Value::Null),
            ("/settings/severity", json!("low")),
            ("/settings/target_arch", json!(["x86_64"])),
            ("/settings/target_os", json!(["macos"])),
            ("/settings/informational_warnings", json!([])),
            (
                "/settings/informational_warnings",
                json!(["notice", "notice", "unsound"]),
            ),
            (
                "/settings/informational_warnings",
                json!(["notice", "unsound", "unknown"]),
            ),
        ] {
            let mut report = clean_report();
            *report
                .pointer_mut(pointer)
                .ok_or("missing report fixture field")? = value;
            assert!(validate_report(&report).is_err(), "{pointer}: {report}");
        }
        for key in [
            "ignore",
            "severity",
            "target_arch",
            "target_os",
            "informational_warnings",
        ] {
            let mut report = clean_report();
            report
                .get_mut("settings")
                .and_then(Value::as_object_mut)
                .ok_or("missing settings")?
                .remove(key);
            assert!(validate_report(&report).is_err(), "missing {key}");
        }
        let mut extra = clean_report();
        extra["settings"]["unknown_filter"] = json!(true);
        assert!(validate_report(&extra).is_err());
        let mut reordered = clean_report();
        reordered["settings"]["informational_warnings"] =
            json!(["notice", "unsound", "unmaintained"]);
        assert!(validate_report(&reordered).is_ok());
        Ok(())
    }

    #[test]
    fn tree_parser_rejects_nonregular_ambiguous_and_oversized_inputs() {
        let object = "1".repeat(40);
        let good = format!("100644 blob {object} 3\tcrates/demo/advisory.md\0");
        assert_eq!(parse_tree(good.as_bytes()).map(|e| e.len()).ok(), Some(1));
        for malformed in [
            String::new(),
            good.trim_end_matches('\0').to_owned(),
            format!("{good}{good}"),
            good.replace("100644", "120000"),
            good.replace("blob", "commit"),
            good.replace(&object, &"A".repeat(40)),
            good.replace("crates/demo/advisory.md", "/crates/demo/advisory.md"),
            good.replace("crates/demo/advisory.md", "crates/../advisory.md"),
            good.replace("crates/demo/advisory.md", "crates/./advisory.md"),
            good.replace("crates/demo/advisory.md", "crates//advisory.md"),
            good.replace("crates/demo/advisory.md", "crates/demo\\advisory.md"),
            good.replace("crates/demo/advisory.md", "elsewhere/advisory.md"),
            good.replace(" 3\t", &format!(" {}\t", MAX_BLOB + 1)),
        ] {
            assert!(parse_tree(malformed.as_bytes()).is_err(), "{malformed:?}");
        }
    }

    #[test]
    fn batch_requires_exact_headers_sizes_terminators_and_inventory() -> Result<()> {
        let entries =
            parse_tree(format!("100644 blob {} 3\tcrates/demo/a.md\0", "1".repeat(40)).as_bytes())?;
        let good = format!("{} blob 3\na\0b\n", "1".repeat(40)).into_bytes();
        let directory = tempfile::tempdir()?;
        let digest = write_batch(directory.path(), &entries, &good)?;
        assert_eq!(
            std::fs::read(directory.path().join("database/crates/demo/a.md"))?,
            b"a\0b"
        );
        let mut canonical = Vec::new();
        canonical.extend_from_slice(&u64::try_from("crates/demo/a.md".len())?.to_be_bytes());
        canonical.extend_from_slice(b"crates/demo/a.md");
        canonical.extend_from_slice(&3_u64.to_be_bytes());
        canonical.extend_from_slice(b"a\0b");
        assert_eq!(digest, hex(Sha256::digest(canonical)));
        for malformed in [
            Vec::new(),
            good[..good.len() - 1].to_vec(),
            good[..good.len() - 3].to_vec(),
            [good.as_slice(), b"extra"].concat(),
            String::from_utf8(good.clone())?
                .replace("blob 3", "blob 4")
                .into_bytes(),
            String::from_utf8(good.clone())?
                .replace(&"1".repeat(40), &"2".repeat(40))
                .into_bytes(),
        ] {
            let refused = tempfile::tempdir()?;
            assert!(write_batch(refused.path(), &entries, &malformed).is_err());
        }
        Ok(())
    }

    struct Fixture {
        directory: tempfile::TempDir,
        repository: PathBuf,
        revision: String,
        blob: String,
    }

    impl Fixture {
        fn object(&self, kind: &str, bytes: &[u8]) -> Result<String> {
            let mut input = tempfile::NamedTempFile::new_in(self.directory.path())?;
            input.write_all(bytes)?;
            input.flush()?;
            let bytes = bounded(
                git(&self.repository)
                    .args(["hash-object", "-w", "-t", kind, "--stdin"])
                    .stdin(std::fs::File::open(input.path())?),
                128,
            )?;
            Ok(String::from_utf8(bytes)?.trim_end_matches('\n').to_owned())
        }

        fn tree(&self, records: &str) -> Result<String> {
            let mut input = tempfile::NamedTempFile::new_in(self.directory.path())?;
            input.write_all(records.as_bytes())?;
            input.flush()?;
            Ok(String::from_utf8(bounded(
                git(&self.repository)
                    .arg("mktree")
                    .stdin(std::fs::File::open(input.path())?),
                128,
            )?)?
            .trim_end_matches('\n')
            .to_owned())
        }

        fn new() -> Result<Self> {
            let directory = tempfile::tempdir()?;
            let repository = directory.path().join("advisories.git");
            bounded(
                git(directory.path())
                    .args(["init", "--bare", "--template=", "--object-format=sha1"])
                    .arg(&repository),
                MAX_TREE,
            )?;
            let mut fixture = Self {
                directory,
                repository,
                revision: String::new(),
                blob: String::new(),
            };
            fixture.blob = fixture.object("blob", b"committed advisory\n")?;
            let package = fixture.tree(&format!(
                "100644 blob {}\tRUSTSEC-2026-0001.md\n",
                fixture.blob
            ))?;
            let crates = fixture.tree(&format!("040000 tree {package}\tdemo\n"))?;
            let tree = fixture.tree(&format!("040000 tree {crates}\tcrates\n"))?;
            fixture.revision = fixture.object("commit", format!(
                "tree {tree}\nauthor Fixture <fixture@example.invalid> 1 +0000\ncommitter Fixture <fixture@example.invalid> 1 +0000\n\nFixture\n"
            ).as_bytes())?;
            Ok(fixture)
        }
    }

    #[test]
    fn committed_snapshot_ignores_loose_files_export_attributes_and_replacements() -> Result<()> {
        let fixture = Fixture::new()?;
        std::fs::create_dir_all(fixture.repository.join("crates/attacker"))?;
        std::fs::write(
            fixture.repository.join("crates/attacker/untracked.md"),
            "untracked",
        )?;
        std::fs::create_dir_all(fixture.repository.join("info"))?;
        std::fs::write(
            fixture.repository.join("info/attributes"),
            "* export-ignore\n*.md export-subst\n",
        )?;
        let replacement = fixture.object("blob", b"replacement contents\n")?;
        bounded(
            git(&fixture.repository).args(["replace", &fixture.blob, &replacement]),
            128,
        )?;
        let snapshot = prepare(&fixture.repository, &fixture.revision)?;
        assert_eq!(snapshot.entries.len(), 1);
        assert_eq!(
            std::fs::read(
                snapshot
                    .directory
                    .path()
                    .join("database/crates/demo/RUSTSEC-2026-0001.md")
            )?,
            b"committed advisory\n"
        );
        assert_eq!(snapshot_digest(&snapshot)?, snapshot.digest);
        let error = bounded(
            git(&fixture.repository).args(["cat-file", "-p", &fixture.blob]),
            1,
        )
        .err()
        .ok_or("missing output bound refusal")?;
        assert!(
            error
                .to_string()
                .contains("exceeded its configured input bound")
        );
        for revision in ["main", "", &"A".repeat(40), &"f".repeat(40)] {
            assert!(prepare(&fixture.repository, revision).is_err());
        }
        Ok(())
    }

    #[test]
    fn snapshot_refuses_content_inventory_and_size_changes() -> Result<()> {
        let fixture = Fixture::new()?;
        let snapshot = prepare(&fixture.repository, &fixture.revision)?;
        let path = snapshot
            .directory
            .path()
            .join("database/crates/demo/RUSTSEC-2026-0001.md");
        std::fs::write(&path, b"altered!! advisory\n")?;
        assert_ne!(snapshot_digest(&snapshot)?, snapshot.digest);
        std::fs::write(&path, b"committed advisory\n")?;
        let extra = snapshot
            .directory
            .path()
            .join("database/crates/demo/extra.md");
        std::fs::write(&extra, "unexpected")?;
        assert!(snapshot_digest(&snapshot).is_err());
        std::fs::remove_file(extra)?;
        std::fs::write(&path, "different size")?;
        assert!(snapshot_digest(&snapshot).is_err());
        Ok(())
    }

    #[test]
    fn git_commands_disable_lazy_fetch_and_replacements_without_protocol_bypasses() {
        let command = git(Path::new("explicit-repository"));
        let args: Vec<_> = command
            .get_args()
            .map(|value| value.to_string_lossy().into_owned())
            .collect();
        assert!(args.iter().any(|value| value == "--no-lazy-fetch"));
        assert!(args.iter().any(|value| value == "--no-replace-objects"));
        for name in [
            "GIT_ALLOW_PROTOCOL",
            "GIT_PROTOCOL_FROM_USER",
            "GIT_CONFIG_NOSYSTEM",
            "GIT_CONFIG_GLOBAL",
            "GIT_CONFIG_SYSTEM",
        ] {
            assert!(!command.get_envs().any(|(key, _)| key == name));
        }
    }
}
