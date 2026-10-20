use serde_json::{Value, json};
use std::io::{BufRead, Read, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn run(
    db: &Path,
    args: &[&str],
    input: Option<Value>,
) -> Result<(bool, Value), Box<dyn std::error::Error>> {
    finish(spawn_input(db, args, input)?)
}

fn spawn_input(
    db: &Path,
    args: &[&str],
    input: Option<Value>,
) -> Result<Child, Box<dyn std::error::Error>> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_wmbd"))
        .arg("--db")
        .arg(db)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    if let Some(input) = input {
        child
            .stdin
            .take()
            .ok_or("missing stdin")?
            .write_all(&serde_json::to_vec(&input)?)?;
    }
    Ok(child)
}

fn finish(child: Child) -> Result<(bool, Value), Box<dyn std::error::Error>> {
    let output = child.wait_with_output()?;
    let bytes = if output.status.success() {
        &output.stdout
    } else {
        &output.stderr
    };
    let value = serde_json::from_slice(bytes)
        .map_err(|error| format!("{error}: {}", String::from_utf8_lossy(bytes)))?;
    Ok((output.status.success(), value))
}

fn success(
    db: &Path,
    args: &[&str],
    input: Option<Value>,
) -> Result<Value, Box<dyn std::error::Error>> {
    let (ok, value) = run(db, args, input)?;
    assert!(ok, "{value}");
    Ok(value)
}

fn failure(
    db: &Path,
    args: &[&str],
    input: Option<Value>,
    code: &str,
) -> Result<Value, Box<dyn std::error::Error>> {
    let (ok, value) = run(db, args, input)?;
    assert!(!ok, "{value}");
    assert_eq!(value["error"]["code"], code);
    Ok(value)
}

#[test]
fn output_failures_return_failure_without_panicking() -> TestResult {
    let diagnostic = Command::new(env!("CARGO_BIN_EXE_wmbd"))
        .arg("--unrecognized-argument")
        .output()?;
    assert_eq!(diagnostic.status.code(), Some(2));
    let error: Value = serde_json::from_slice(&diagnostic.stderr)?;
    assert_eq!(error["error"]["path"], "arguments");
    let temp = tempfile::tempdir()?;
    let missing = temp.path().join("missing.redb");
    for args in [["--unrecognized-argument"], ["list"]] {
        let (reader, writer) = std::io::pipe()?;
        drop(reader);
        let output = Command::new(env!("CARGO_BIN_EXE_wmbd"))
            .arg("--db")
            .arg(&missing)
            .args(args)
            .stdout(Stdio::piped())
            .stderr(writer)
            .output()?;
        assert_eq!(output.status.code(), Some(1), "{args:?}");
        assert!(output.stdout.is_empty());
    }
    let (reader, writer) = std::io::pipe()?;
    drop(reader);
    let output = Command::new(env!("CARGO_BIN_EXE_wmbd"))
        .args(["schema", "note"])
        .stdout(writer)
        .stderr(Stdio::piped())
        .output()?;
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stderr.is_empty());
    assert!(!missing.exists());
    Ok(())
}

#[test]
fn literal_note_preserves_arguments_and_ignores_json_stdin() -> TestResult {
    let temp = tempfile::tempdir()?;
    let db = temp.path().join("literal-note.redb");
    success(&db, &["init"], None)?;
    success(&db, &["add"], Some(json!({"title":"Resume shared work"})))?;
    let body = "次は共通 runner を接続する\nKeep \"quoted\" details and 🦀";
    let stdin = temp.path().join("ignored.json");
    std::fs::write(
        &stdin,
        serde_json::to_vec(&json!({"body":"Not the literal note"}))?,
    )?;
    let child = Command::new(env!("CARGO_BIN_EXE_wmbd"))
        .arg("--db")
        .arg(&db)
        .args(["note", "1", body])
        .stdin(std::fs::File::open(stdin)?)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let (ok, note) = finish(child)?;
    assert!(ok, "{note}");
    assert_eq!(
        note,
        json!({"api_version":1,"data":{"id":1,"ticket_id":1,"body":body}})
    );
    assert_eq!(
        success(&db, &["show", "1"], None)?["data"]["notes"]["items"][0],
        note["data"]
    );
    let json_text = r#"{"body":"Treat this as literal text","request_id":"not-a-request"}"#;
    let literal = success(&db, &["note", "1", json_text], None)?;
    assert_eq!(literal["data"]["id"], 2);
    assert_eq!(literal["data"]["body"], json_text);
    let leading_dash = success(&db, &["note", "1", "--", "--next-action"], None)?;
    assert_eq!(leading_dash["data"]["id"], 3);
    assert_eq!(leading_dash["data"]["body"], "--next-action");
    assert_eq!(success(&db, &["notes"], None)?["data"]["total"], 3);
    Ok(())
}

#[test]
fn literal_and_json_notes_share_request_replays_and_ticket_identity() -> TestResult {
    let temp = tempfile::tempdir()?;
    let db = temp.path().join("literal-note-replay.redb");
    success(&db, &["init"], None)?;
    for title in ["Shared foundation", "Dependent integration"] {
        success(&db, &["add"], Some(json!({"title":title})))?;
    }
    let body = "Resume by connecting the runner\nKeep \"quoted\" context and 🦀";
    let args = ["note", "1", body, "--request-id", "literal-first"];
    let input = json!({"body":body,"request_id":"literal-first"});
    let original = success(&db, &args, None)?;
    assert_eq!(original["data"], json!({"id":1,"ticket_id":1,"body":body}));
    assert_eq!(success(&db, &args, None)?, original);
    assert_eq!(success(&db, &["note", "1"], Some(input.clone()))?, original);
    assert_eq!(
        success(&db, &["note", "1", "--input", "-"], Some(input.clone()))?,
        original
    );
    let updated = success(
        &db,
        &["update", "1"],
        Some(json!({"revision":1,"title":"Completed shared foundation","status":"done"})),
    )?;
    assert_eq!(success(&db, &args, None)?, original);
    assert_eq!(success(&db, &["note", "1"], Some(input))?, original);
    assert_eq!(
        success(&db, &["show", "1"], None)?["data"]["ticket"],
        updated["data"]
    );
    let from_json = success(
        &db,
        &["note", "2"],
        Some(json!({"body":"Discovered through JSON","request_id":"json-first"})),
    )?;
    assert_eq!(from_json["data"]["id"], 2);
    assert_eq!(
        success(
            &db,
            &[
                "note",
                "2",
                "Discovered through JSON",
                "--request-id",
                "json-first"
            ],
            None
        )?,
        from_json
    );
    failure(
        &db,
        &["note", "1", "Changed body", "--request-id", "literal-first"],
        None,
        "request_conflict",
    )?;
    failure(
        &db,
        &["note", "2", body, "--request-id", "literal-first"],
        None,
        "request_conflict",
    )?;
    failure(
        &db,
        &["note", "2"],
        Some(json!({"body":body,"request_id":"literal-first"})),
        "request_conflict",
    )?;
    failure(
        &db,
        &["add"],
        Some(json!({"title":"Different operation","request_id":"literal-first"})),
        "request_conflict",
    )?;
    let added = success(
        &db,
        &["add"],
        Some(json!({"title":"Operation identity","request_id":"ticket-first"})),
    )?;
    assert_eq!(added["data"]["id"], 3);
    failure(
        &db,
        &[
            "note",
            "1",
            "Different operation",
            "--request-id",
            "ticket-first",
        ],
        None,
        "request_conflict",
    )?;
    let no_request = success(
        &db,
        &["note", "1"],
        Some(json!({"body":"No request identity","request_id":null})),
    )?;
    assert_eq!(no_request["data"]["id"], 3);
    let repeated = success(&db, &["note", "1", "No request identity"], None)?;
    assert_eq!(repeated["data"]["id"], 4);
    let recent = success(&db, &["notes"], None)?;
    assert_eq!(recent["data"]["total"], 4);
    assert_eq!(recent["data"]["items"][3]["note"], original["data"]);
    assert_eq!(
        recent["data"]["items"][3]["ticket"],
        json!({"id":1,"revision":2,"title":"Completed shared foundation","status":"done","repos":[],"priority":"normal","parent":null,"blocked_by":[]})
    );
    Ok(())
}

#[test]
fn literal_note_validation_preserves_json_selection_and_atomic_allocation() -> TestResult {
    let temp = tempfile::tempdir()?;
    let parent = temp.path().join("missing").join("nested");
    let missing = parent.join("literal-note.redb");
    let arguments: &[&[&str]] = &[
        &["note"],
        &["note", "0", "Body"],
        &["note", "not-an-id", "Body"],
        &["note", "1", "First body", "Second body"],
        &["note", "1", "Body", "--input", "-"],
        &["note", "1", "Body", "--input", "missing.json"],
        &["note", "1", "--input", "-", "Body"],
        &["note", "1", "--request-id", "requires-body"],
        &["note", "1", "--input", "-", "--request-id", "requires-body"],
        &[
            "note",
            "1",
            "--input",
            "missing.json",
            "--request-id",
            "requires-body",
        ],
        &[
            "note",
            "1",
            "Body",
            "--request-id",
            "conflicting-input",
            "--input",
            "-",
        ],
        &[
            "note",
            "1",
            "Body",
            "--request-id",
            "first",
            "--request-id",
            "second",
        ],
    ];
    for args in arguments {
        let error = failure(&missing, args, None, "invalid_input")?;
        assert_eq!(error["error"]["path"], "arguments", "{args:?}");
        assert_eq!(error["error"]["retryable"], false);
        assert!(!parent.exists(), "{args:?}");
    }
    failure(
        &missing,
        &["note", "1", "Remember this discovery"],
        None,
        "not_initialized",
    )?;
    assert!(!parent.exists());
    let db = temp.path().join("literal-note-errors.redb");
    success(&db, &["init"], None)?;
    let ticket = success(&db, &["add"], Some(json!({"title":"Preserve this work"})))?;
    for body in ["", " \n\t"] {
        let error = failure(
            &db,
            &["note", "1", body, "--request-id", "recovered"],
            None,
            "invalid_input",
        )?;
        assert_eq!(error["error"]["path"], "note.body");
    }
    for request_id in ["", " \n\t"] {
        let error = failure(
            &db,
            &["note", "1", "Valid body", "--request-id", request_id],
            None,
            "invalid_input",
        )?;
        assert_eq!(error["error"]["path"], "request_id");
    }
    failure(
        &db,
        &["note", "99", "Missing owner", "--request-id", "recovered"],
        None,
        "not_found",
    )?;
    assert_eq!(success(&db, &["notes"], None)?["data"]["total"], 0);
    assert_eq!(
        success(&db, &["show", "1"], None)?["data"]["ticket"],
        ticket["data"]
    );
    let first = success(
        &db,
        &[
            "note",
            "1",
            "Recovered discovery",
            "--request-id",
            "recovered",
        ],
        None,
    )?;
    assert_eq!(first["data"]["id"], 1);
    let held = redb::Database::open(&db)?;
    let args = [
        "note",
        "1",
        "After the writer releases",
        "--request-id",
        "after-lock",
    ];
    let blocked = failure(
        &db,
        &[
            "--lock-timeout-ms",
            "0",
            "note",
            "1",
            "After the writer releases",
            "--request-id",
            "after-lock",
        ],
        None,
        "database_busy",
    )?;
    assert_eq!(blocked["error"]["retryable"], true);
    drop(held);
    let second = success(&db, &args, None)?;
    assert_eq!(second["data"]["id"], 2);
    assert_eq!(success(&db, &args, None)?, second);
    assert_eq!(success(&db, &["notes"], None)?["data"]["total"], 2);
    assert_eq!(
        success(&db, &["show", "1"], None)?["data"]["ticket"],
        ticket["data"]
    );
    Ok(())
}

#[test]
fn literal_notes_keep_json_sources_and_canonical_schemas_compatible() -> TestResult {
    let temp = tempfile::tempdir()?;
    let parent = temp.path().join("missing");
    let db = parent.join("schema.redb");
    let schema = success(&db, &["schema", "note"], None)?;
    assert_eq!(schema["data"]["command"], "note");
    assert!(!parent.exists());
    let input = jsonschema::validator_for(&schema["data"]["input"])?;
    for value in [
        json!({"body":"Resume here"}),
        json!({"body":"Resume here","request_id":null}),
        json!({"body":"Resume here","request_id":"typed"}),
    ] {
        assert!(input.is_valid(&value), "{value}");
    }
    for value in [
        json!({}),
        json!({"body":42}),
        json!({"body":"Resume here","request_id":42}),
        json!({"body":"Resume here","input":"-"}),
        json!({"body":"Resume here","id":1}),
    ] {
        assert!(!input.is_valid(&value), "{value}");
    }
    success(&db, &["init"], None)?;
    success(&db, &["add"], Some(json!({"title":"Typed notes"})))?;
    let file = temp.path().join("note.json");
    let file_input = json!({"body":"Read JSON from a file","request_id":"file-note"});
    std::fs::write(&file, serde_json::to_vec(&file_input)?)?;
    let file = file.to_str().ok_or("non-UTF-8 fixture path")?;
    let from_file = success(&db, &["note", "1", "--input", file], None)?;
    assert_eq!(
        from_file["data"],
        json!({"id":1,"ticket_id":1,"body":"Read JSON from a file"})
    );
    assert_eq!(
        success(
            &db,
            &[
                "note",
                "1",
                "Read JSON from a file",
                "--request-id",
                "file-note"
            ],
            None
        )?,
        from_file
    );
    let default_stdin = success(
        &db,
        &["note", "1"],
        Some(json!({"body":"Read default standard input"})),
    )?;
    assert_eq!(default_stdin["data"]["id"], 2);
    assert_eq!(default_stdin["data"]["body"], "Read default standard input");
    let explicit_stdin = success(
        &db,
        &["note", "1", "--input", "-"],
        Some(json!({"body":"Read explicit standard input"})),
    )?;
    assert_eq!(explicit_stdin["data"]["id"], 3);
    assert_eq!(
        explicit_stdin["data"]["body"],
        "Read explicit standard input"
    );
    let literal = success(&db, &["note", "1", "Read a literal body"], None)?;
    let output = jsonschema::validator_for(&schema["data"]["output"])?;
    for response in [from_file, default_stdin, explicit_stdin, literal] {
        assert!(output.is_valid(&response), "{response}");
    }
    let error_schema = jsonschema::validator_for(&schema["data"]["error"])?;
    let syntax = failure(
        &db,
        &["note", "1", "Body", "--input", "-"],
        None,
        "invalid_input",
    )?;
    assert!(error_schema.is_valid(&syntax));
    let runtime = failure(&db, &["note", "1", ""], None, "invalid_input")?;
    assert!(error_schema.is_valid(&runtime));
    failure(
        &db,
        &["note", "1"],
        Some(json!({"body":"Unknown JSON field","unknown":true})),
        "invalid_input",
    )?;
    assert_eq!(success(&db, &["notes"], None)?["data"]["total"], 4);
    Ok(())
}

#[test]
fn recent_notes_recover_latest_body_with_current_cross_repository_context() -> TestResult {
    let temp = tempfile::tempdir()?;
    let db = temp.path().join("recent-notes.redb");
    success(&db, &["init"], None)?;
    success(
        &db,
        &["add"],
        Some(json!({"title":"Shared foundation","repos":["owner/foundation"],"priority":"high"})),
    )?;
    success(
        &db,
        &["add"],
        Some(
            json!({"title":"Resume integration","repos":["owner/app"],"priority":"low","depends_on":[1]}),
        ),
    )?;
    success(
        &db,
        &["note", "1"],
        Some(json!({"body":"An earlier discovery"})),
    )?;
    let body = "次は共通 runner を接続する\nKeep \"quoted\" details and 🦀";
    let input = json!({"body":body,"request_id":"resume-note"});
    let latest = success(&db, &["note", "2"], Some(input.clone()))?;
    let page = success(&db, &["notes", "--limit", "1"], None)?;
    assert_eq!(page["data"]["total"], 2);
    assert_eq!(page["data"]["next_offset"], 1);
    assert_eq!(
        page["data"]["items"]
            .as_array()
            .ok_or("missing notes")?
            .len(),
        1
    );
    assert_eq!(page["data"]["items"][0]["note"], latest["data"]);
    assert_eq!(page["data"]["items"][0]["note"]["body"], body);
    assert_eq!(
        page["data"]["items"][0]["ticket"],
        json!({
            "id":2,"revision":1,"title":"Resume integration","status":"open",
            "repos":["owner/app"],"priority":"low","parent":null,"blocked_by":[1]
        })
    );
    success(
        &db,
        &["update", "2"],
        Some(
            json!({"revision":1,"title":"Integration in progress","repos":["owner/new-app"],"priority":"high","status":"in_progress","parent":1}),
        ),
    )?;
    success(
        &db,
        &["update", "1"],
        Some(json!({"revision":1,"status":"done"})),
    )?;
    let refreshed = success(&db, &["notes", "--limit", "1"], None)?;
    assert_eq!(refreshed["data"]["items"][0]["note"], latest["data"]);
    assert_eq!(
        refreshed["data"]["items"][0]["ticket"],
        json!({
            "id":2,"revision":2,"title":"Integration in progress","status":"in_progress",
            "repos":["owner/new-app"],"priority":"high","parent":1,"blocked_by":[]
        })
    );
    let closed = success(
        &db,
        &["note", "1"],
        Some(json!({"body":"The completed foundation remains useful elsewhere"})),
    )?;
    assert_eq!(success(&db, &["note", "2"], Some(input))?, latest);
    success(
        &db,
        &["update", "2"],
        Some(json!({"revision":2,"status":"cancelled"})),
    )?;
    let all = success(&db, &["notes"], None)?;
    assert_eq!(all["data"]["total"], 3);
    assert!(all["data"]["next_offset"].is_null());
    assert_eq!(all["data"]["items"][0]["note"], closed["data"]);
    assert_eq!(all["data"]["items"][0]["ticket"]["status"], "done");
    assert_eq!(all["data"]["items"][1]["note"], latest["data"]);
    assert_eq!(all["data"]["items"][1]["ticket"]["status"], "cancelled");
    assert_eq!(all["data"]["items"][1]["ticket"]["revision"], 3);
    assert_eq!(all["data"]["items"][2]["note"]["id"], 1);
    Ok(())
}

#[test]
fn recent_notes_paginate_complete_global_history_after_ordering() -> TestResult {
    fn ids(page: &Value) -> Result<Vec<u64>, Box<dyn std::error::Error>> {
        page["data"]["items"]
            .as_array()
            .ok_or("missing notes")?
            .iter()
            .map(|item| {
                item["note"]["id"]
                    .as_u64()
                    .ok_or_else(|| "missing note ID".into())
            })
            .collect()
    }

    let temp = tempfile::tempdir()?;
    let db = temp.path().join("note-pages.redb");
    success(&db, &["init"], None)?;
    assert_eq!(
        success(&db, &["notes"], None)?["data"],
        json!({"items":[],"total":0,"next_offset":null})
    );
    for input in [
        json!({"title":"First repo","repos":["owner/alpha"],"priority":"low"}),
        json!({"title":"Second repo","repos":["owner/beta"],"priority":"high"}),
        json!({"title":"Unassigned completion"}),
        json!({"title":"Cancelled shared work","repos":["owner/alpha","owner/beta"]}),
    ] {
        success(&db, &["add"], Some(input))?;
    }
    for (id, status) in [("2", "in_progress"), ("3", "done"), ("4", "cancelled")] {
        success(
            &db,
            &["update", id],
            Some(json!({"revision":1,"status":status})),
        )?;
    }
    for id in 1..=53_u64 {
        let owner = ((id - 1) % 4 + 1).to_string();
        let note = success(
            &db,
            &["note", &owner],
            Some(
                json!({"body":format!("Saved next action {id}\nAcross repositories and statuses")}),
            ),
        )?;
        assert_eq!(note["data"]["id"], id);
    }
    let first = success(&db, &["notes"], None)?;
    assert_eq!(first["data"]["total"], 53);
    assert_eq!(first["data"]["next_offset"], 50);
    assert_eq!(ids(&first)?, (4..=53).rev().collect::<Vec<_>>());
    assert_eq!(first["data"]["items"][0]["ticket"]["id"], 1);
    assert_eq!(first["data"]["items"][1]["ticket"]["status"], "cancelled");
    assert_eq!(first["data"]["items"][2]["ticket"]["status"], "done");
    assert_eq!(first["data"]["items"][2]["ticket"]["repos"], json!([]));
    assert_eq!(first["data"]["items"][3]["ticket"]["status"], "in_progress");
    let last = success(&db, &["notes", "--offset", "50"], None)?;
    assert_eq!(last["data"]["total"], 53);
    assert_eq!(ids(&last)?, vec![3, 2, 1]);
    assert!(last["data"]["next_offset"].is_null());
    let middle = success(&db, &["notes", "--limit", "2", "--offset", "49"], None)?;
    assert_eq!(middle["data"]["total"], 53);
    assert_eq!(middle["data"]["next_offset"], 51);
    assert_eq!(ids(&middle)?, vec![4, 3]);
    let complete = success(&db, &["notes", "--limit", "500"], None)?;
    assert_eq!(complete["data"]["total"], 53);
    assert_eq!(ids(&complete)?, (1..=53).rev().collect::<Vec<_>>());
    assert!(complete["data"]["next_offset"].is_null());
    for offset in ["53".to_owned(), usize::MAX.to_string()] {
        assert_eq!(
            success(&db, &["notes", "--offset", &offset], None)?["data"],
            json!({"items":[],"total":53,"next_offset":null})
        );
    }
    Ok(())
}

#[test]
fn recent_notes_schema_and_ranges_require_no_database() -> TestResult {
    let temp = tempfile::tempdir()?;
    let parent = temp.path().join("missing").join("nested");
    let db = parent.join("notes.redb");
    let schema = success(&db, &["schema", "notes"], None)?;
    assert_eq!(schema["data"]["command"], "notes");
    assert!(!parent.exists());
    let input = jsonschema::validator_for(&schema["data"]["input"])?;
    for value in [
        json!({}),
        json!({"limit":1}),
        json!({"limit":500,"offset":usize::MAX}),
    ] {
        assert!(input.is_valid(&value), "{value}");
    }
    for value in [
        json!({"limit":0}),
        json!({"limit":501}),
        json!({"offset":-1}),
        json!({"query":"next"}),
        json!({"repo":"owner/repo"}),
    ] {
        assert!(!input.is_valid(&value), "{value}");
    }
    for args in [
        &["notes", "--limit", "0"][..],
        &["notes", "--limit", "501"][..],
        &["notes", "--offset", "-1"][..],
        &["notes", "--query", "next"][..],
        &["notes", "--repo", "owner/repo"][..],
    ] {
        failure(&db, args, None, "invalid_input")?;
        assert!(!parent.exists());
    }
    let error = failure(&db, &["notes"], None, "not_initialized")?;
    assert!(jsonschema::validator_for(&schema["data"]["error"])?.is_valid(&error));
    assert!(!parent.exists());
    success(&db, &["init"], None)?;
    let empty = success(&db, &["notes"], None)?;
    let output = jsonschema::validator_for(&schema["data"]["output"])?;
    assert!(output.is_valid(&empty));
    success(&db, &["add"], Some(json!({"title":"Typed discovery"})))?;
    success(
        &db,
        &["note", "1"],
        Some(json!({"body":"Read the saved next action"})),
    )?;
    let populated = success(&db, &["notes", "--limit", "1"], None)?;
    assert!(output.is_valid(&populated));
    for field in ["note", "ticket"] {
        let mut invalid = populated.clone();
        invalid["data"]["items"][0][field]["id"] = json!(0);
        assert!(!output.is_valid(&invalid), "{field}");
    }
    Ok(())
}

#[test]
fn capture_defaults_preserve_literal_arguments_and_ignore_json_stdin() -> TestResult {
    let temp = tempfile::tempdir()?;
    let db = temp.path().join("capture.redb");
    success(&db, &["init"], None)?;
    let first = success(&db, &["capture", "Remember the next action"], None)?;
    assert_eq!(
        first["data"],
        json!({
            "id":1,"revision":1,"title":"Remember the next action","body":"",
            "status":"open","repos":[],"priority":"normal","parent":null,"depends_on":[]
        })
    );
    let title = "Remember \"quotes\", \\ paths, $(literal), `text` and 日本語\nNext action";
    let body = "First line\nSecond line: \"quoted\" \\ literal 🦀";
    let input = temp.path().join("ignored.json");
    std::fs::write(
        &input,
        serde_json::to_vec(
            &json!({"title":"Ignore this JSON","body":"Do not parse standard input"}),
        )?,
    )?;
    let child = Command::new(env!("CARGO_BIN_EXE_wmbd"))
        .arg("--db")
        .arg(&db)
        .args(["capture", title, "--body", body])
        .stdin(std::fs::File::open(&input)?)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let (ok, literal) = finish(child)?;
    assert!(ok, "{literal}");
    assert_eq!(literal["data"]["id"], 2);
    assert_eq!(literal["data"]["title"], title);
    assert_eq!(literal["data"]["body"], body);
    assert_eq!(literal["data"]["status"], "open");
    assert_eq!(literal["data"]["revision"], 1);
    assert_eq!(literal["data"]["priority"], "normal");
    assert_eq!(literal["data"]["repos"], json!([]));
    assert!(literal["data"]["parent"].is_null());
    assert_eq!(literal["data"]["depends_on"], json!([]));
    assert_eq!(
        success(&db, &["show", "2"], None)?["data"]["ticket"],
        literal["data"]
    );
    assert_eq!(success(&db, &["list"], None)?["data"]["total"], 2);
    Ok(())
}

#[test]
fn capture_and_json_add_share_raw_payload_replays_and_original_responses() -> TestResult {
    let temp = tempfile::tempdir()?;
    let db = temp.path().join("capture-replay.redb");
    success(&db, &["init"], None)?;
    for title in ["Purpose", "First prerequisite", "Second prerequisite"] {
        success(&db, &["add"], Some(json!({"title":title})))?;
    }
    let args = [
        "capture",
        "Adopt the shared foundation",
        "--body",
        "Resume by connecting the runner\nKeep the previous discovery",
        "--repo",
        "owner/zeta",
        "--repo",
        "owner/alpha",
        "--repo",
        "owner/alpha",
        "--priority",
        "high",
        "--parent",
        "1",
        "--depends-on",
        "3",
        "--depends-on",
        "2",
        "--depends-on",
        "3",
        "--request-id",
        "capture-full",
    ];
    let input = json!({
        "title":"Adopt the shared foundation",
        "body":"Resume by connecting the runner\nKeep the previous discovery",
        "repos":["owner/zeta","owner/alpha","owner/alpha"],
        "priority":"high","parent":1,"depends_on":[3,2,3],"request_id":"capture-full"
    });
    let original = success(&db, &args, None)?;
    assert_eq!(
        original["data"],
        json!({
            "id":4,"revision":1,"title":"Adopt the shared foundation",
            "body":"Resume by connecting the runner\nKeep the previous discovery",
            "status":"open","repos":["owner/alpha","owner/zeta"],
            "priority":"high","parent":1,"depends_on":[2,3]
        })
    );
    assert_eq!(success(&db, &args, None)?, original);
    assert_eq!(success(&db, &["add"], Some(input.clone()))?, original);
    let updated = success(
        &db,
        &["update", "4"],
        Some(json!({"revision":1,"body":"A newer next action","status":"in_progress"})),
    )?;
    assert_eq!(updated["data"]["revision"], 2);
    assert_eq!(success(&db, &args, None)?, original);
    assert_eq!(success(&db, &["add"], Some(input.clone()))?, original);
    assert_eq!(
        success(&db, &["show", "4"], None)?["data"]["ticket"],
        updated["data"]
    );

    let mut changed_args = args;
    changed_args[1] = "Different content";
    failure(&db, &changed_args, None, "request_conflict")?;
    for (field, value) in [
        ("repos", json!(["owner/alpha", "owner/zeta", "owner/alpha"])),
        ("repos", json!(["owner/zeta", "owner/alpha"])),
        ("depends_on", json!([2, 3, 3])),
        ("depends_on", json!([3, 2])),
    ] {
        let mut changed = input.clone();
        changed[field] = value;
        failure(&db, &["add"], Some(changed), "request_conflict")?;
    }
    let from_json = success(
        &db,
        &["add"],
        Some(json!({"title":"Captured from JSON","request_id":"json-first"})),
    )?;
    assert_eq!(from_json["data"]["id"], 5);
    assert_eq!(
        success(
            &db,
            &[
                "capture",
                "Captured from JSON",
                "--body",
                "",
                "--priority",
                "normal",
                "--request-id",
                "json-first"
            ],
            None,
        )?,
        from_json
    );
    let next = success(&db, &["capture", "One new ticket"], None)?;
    assert_eq!(next["data"]["id"], 6);
    assert_eq!(success(&db, &["list", "--all"], None)?["data"]["total"], 6);
    assert_eq!(
        success(&db, &["show", "4"], None)?["data"]["ticket"],
        updated["data"]
    );
    Ok(())
}

#[test]
fn capture_rejects_invalid_arguments_without_initializing_or_partially_saving() -> TestResult {
    let temp = tempfile::tempdir()?;
    let parent = temp.path().join("missing").join("nested");
    let missing = parent.join("capture.redb");
    let arguments: &[&[&str]] = &[
        &["capture"],
        &["capture", "Ticket", "--priority", "urgent"],
        &["capture", "Ticket", "--parent", "0"],
        &["capture", "Ticket", "--depends-on", "0"],
        &["capture", "Ticket", "--status", "done"],
        &["capture", "Ticket", "--input", "-"],
        &["capture", "Ticket", "--repo", "first", "second"],
        &["capture", "Ticket", "--depends-on", "1", "2"],
    ];
    for args in arguments {
        let error = failure(&missing, args, None, "invalid_input")?;
        assert_eq!(error["error"]["retryable"], false);
        assert!(!parent.exists(), "{args:?}");
    }
    failure(
        &missing,
        &["capture", "Remember this"],
        None,
        "not_initialized",
    )?;
    assert!(!parent.exists());

    let db = temp.path().join("capture-errors.redb");
    success(&db, &["init"], None)?;
    let existing = success(&db, &["add"], Some(json!({"title":"Existing work"})))?;
    let invalid: &[&[&str]] = &[
        &["capture", " \n\t"],
        &["capture", "Ticket", "--repo", " \t"],
        &["capture", "Ticket", "--request-id", " "],
        &[
            "capture",
            "Ticket",
            "--parent",
            "99",
            "--request-id",
            "recoverable",
        ],
        &[
            "capture",
            "Ticket",
            "--depends-on",
            "99",
            "--request-id",
            "recoverable",
        ],
    ];
    for args in invalid {
        failure(&db, args, None, "invalid_input")?;
        assert_eq!(success(&db, &["list", "--all"], None)?["data"]["total"], 1);
        assert_eq!(
            success(&db, &["show", "1"], None)?["data"]["ticket"],
            existing["data"]
        );
    }
    let second = success(
        &db,
        &[
            "capture",
            "Recovered request",
            "--parent",
            "1",
            "--depends-on",
            "1",
            "--request-id",
            "recoverable",
        ],
        None,
    )?;
    assert_eq!(second["data"]["id"], 2);
    assert_eq!(second["data"]["parent"], 1);
    assert_eq!(second["data"]["depends_on"], json!([1]));
    let held = redb::Database::open(&db)?;
    let error = failure(
        &db,
        &[
            "--lock-timeout-ms",
            "0",
            "capture",
            "After the writer releases",
            "--request-id",
            "after-lock",
        ],
        None,
        "database_busy",
    )?;
    assert_eq!(error["error"]["retryable"], true);
    drop(held);
    let third = success(
        &db,
        &[
            "capture",
            "After the writer releases",
            "--request-id",
            "after-lock",
        ],
        None,
    )?;
    assert_eq!(third["data"]["id"], 3);
    assert_eq!(success(&db, &["list", "--all"], None)?["data"]["total"], 3);
    Ok(())
}

#[test]
fn capture_schemas_reuse_add_types_without_accessing_storage() -> TestResult {
    let temp = tempfile::tempdir()?;
    let parent = temp.path().join("missing");
    let db = parent.join("schema.redb");
    let add = success(&db, &["schema", "add"], None)?;
    let capture = success(&db, &["schema", "capture"], None)?;
    assert_eq!(add["data"]["command"], "add");
    assert_eq!(capture["data"]["command"], "capture");
    for field in ["input", "output", "error"] {
        assert_eq!(capture["data"][field], add["data"][field], "{field}");
    }
    let validator = jsonschema::validator_for(&capture["data"]["input"])?;
    for input in [
        json!({"title":"Typed capture"}),
        json!({"title":"Typed capture","parent":null,"request_id":null}),
        json!({"title":"Typed capture","body":"Resume here","repos":["owner/repo"],"priority":"low","parent":1,"depends_on":[1,u64::MAX],"request_id":"typed"}),
    ] {
        assert!(validator.is_valid(&input), "{input}");
    }
    for input in [
        json!({}),
        json!({"title":7}),
        json!({"title":"Typed capture","parent":0}),
        json!({"title":"Typed capture","depends_on":[0]}),
        json!({"title":"Typed capture","priority":"urgent"}),
        json!({"title":"Typed capture","status":"done"}),
    ] {
        assert!(!validator.is_valid(&input), "{input}");
    }
    assert!(!parent.exists());
    success(&db, &["init"], None)?;
    let added = success(
        &db,
        &["capture", "Typed capture", "--priority", "low"],
        None,
    )?;
    assert!(jsonschema::validator_for(&capture["data"]["output"])?.is_valid(&added));
    let error = failure(&db, &["capture"], None, "invalid_input")?;
    assert!(jsonschema::validator_for(&capture["data"]["error"])?.is_valid(&error));
    Ok(())
}

#[test]
fn backlog_survives_processes_and_preserves_work_context() -> TestResult {
    let temp = tempfile::tempdir()?;
    let db = temp.path().join("backlog.redb");
    let init = success(&db, &["init"], None)?;
    assert_eq!(init["api_version"], 1);
    let goal = success(
        &db,
        &["add"],
        Some(json!({
            "title": "Shared verification", "body": "Make every project easier to develop",
            "repos": ["P4suta/njutest", "P4suta/aozora"], "priority": "high"
        })),
    )?;
    let parent = goal["data"]["id"].as_u64().ok_or("missing ID")?;
    let prerequisite = success(
        &db,
        &["add"],
        Some(json!({"title": "Implement the runner"})),
    )?;
    let dependency = prerequisite["data"]["id"].as_u64().ok_or("missing ID")?;
    let work = success(
        &db,
        &["add"],
        Some(json!({
            "title": "Adopt the runner", "parent": parent, "depends_on": [dependency],
            "repos": ["P4suta/aozora"]
        })),
    )?;
    let id = work["data"]["id"].as_u64().ok_or("missing ID")?.to_string();
    success(
        &db,
        &["note", &id],
        Some(json!({"body": "次は既存の検証を接続する"})),
    )?;
    let context = success(&db, &["show", &id, "--context"], None)?;
    assert_eq!(
        context["data"]["context"]["ancestors"][0]["body"],
        "Make every project easier to develop"
    );
    assert_eq!(
        context["data"]["notes"]["items"][0]["body"],
        "次は既存の検証を接続する"
    );
    assert_eq!(
        context["data"]["context"]["dependencies"][0]["id"],
        dependency
    );
    let matches = success(&db, &["search", "既存"], None)?;
    assert_eq!(matches["data"]["items"][0]["id"].as_u64(), id.parse().ok());
    success(&db, &["init"], None)?;
    assert_eq!(
        success(&db, &["list", "--repo", "P4suta/aozora"], None)?["data"]["total"],
        2
    );
    Ok(())
}

#[test]
fn rejected_relations_and_stale_updates_roll_back() -> TestResult {
    let temp = tempfile::tempdir()?;
    let db = temp.path().join("db.redb");
    success(&db, &["init"], None)?;
    success(&db, &["add"], Some(json!({"title":"Purpose"})))?;
    success(&db, &["add"], Some(json!({"title":"Child", "parent":1})))?;
    failure(
        &db,
        &["update", "1"],
        Some(json!({"revision":1,"parent":2})),
        "invalid_input",
    )?;
    assert_eq!(
        success(&db, &["show", "1"], None)?["data"]["ticket"]["revision"],
        1
    );
    failure(
        &db,
        &["add"],
        Some(json!({"title":"Missing", "depends_on":[99]})),
        "invalid_input",
    )?;
    let third = success(
        &db,
        &["add"],
        Some(json!({"title":"Third", "depends_on":[2]})),
    )?;
    assert_eq!(third["data"]["id"], 3);
    failure(
        &db,
        &["update", "2"],
        Some(json!({"revision":1,"depends_on":[3]})),
        "invalid_input",
    )?;
    success(
        &db,
        &["update", "1"],
        Some(json!({"revision":1,"depends_on":[2]})),
    )?;
    let conflict = failure(
        &db,
        &["update", "1"],
        Some(json!({"revision":1,"body":"stale"})),
        "conflict",
    )?;
    assert_eq!(conflict["error"]["current_revision"], 2);
    assert_eq!(
        success(&db, &["show", "1"], None)?["data"]["ticket"]["body"],
        ""
    );
    failure(
        &db,
        &["update", "2"],
        Some(json!({"revision":1,"parent":1,"clear_parent":true})),
        "invalid_input",
    )?;
    success(
        &db,
        &["update", "2"],
        Some(json!({"revision":1,"clear_parent":true,"status":"done"})),
    )?;
    assert_eq!(
        success(&db, &["show", "2"], None)?["data"]["ticket"]["parent"],
        Value::Null
    );
    assert_eq!(
        success(&db, &["list", "--ready"], None)?["data"]["total"],
        2
    );
    Ok(())
}

#[test]
fn unchanged_updates_preserve_revisions_after_normalization_and_keep_conflicts() -> TestResult {
    let temp = tempfile::tempdir()?;
    let db = temp.path().join("unchanged.redb");
    success(&db, &["init"], None)?;
    for title in ["Purpose", "First prerequisite", "Second prerequisite"] {
        success(&db, &["add"], Some(json!({"title":title})))?;
    }
    let original = success(
        &db,
        &["add"],
        Some(json!({
            "title":"Work", "body":"Context", "priority":"high", "parent":1,
            "repos":["repo/Z","repo/A","repo/Z"], "depends_on":[3,2,2]
        })),
    )?;
    for input in [
        json!({"revision":1}),
        json!({"revision":1,"title":"Work","body":"Context","status":"open","priority":"high","parent":1,"repos":["repo/Z","repo/A","repo/A"],"depends_on":[3,2,3]}),
        json!({"revision":1,"title":null,"body":null,"status":null,"priority":null,"parent":null,"repos":null,"depends_on":null}),
    ] {
        assert_eq!(success(&db, &["update", "4"], Some(input))?, original);
    }
    let mut revision = 1_u64;
    let mut latest = original;
    for (mut patch, field, value) in [
        (json!({"title":"Renamed"}), "title", json!("Renamed")),
        (json!({"body":"Updated"}), "body", json!("Updated")),
        (
            json!({"status":"in_progress"}),
            "status",
            json!("in_progress"),
        ),
        (json!({"priority":"low"}), "priority", json!("low")),
        (json!({"repos":["repo/B"]}), "repos", json!(["repo/B"])),
        (json!({"parent":2}), "parent", json!(2)),
        (json!({"clear_parent":true}), "parent", Value::Null),
        (json!({"depends_on":[]}), "depends_on", json!([])),
    ] {
        patch["revision"] = json!(revision);
        latest = success(&db, &["update", "4"], Some(patch))?;
        revision += 1;
        assert_eq!(latest["data"]["revision"], revision);
        assert_eq!(latest["data"][field], value);
        assert_eq!(
            success(&db, &["update", "4"], Some(json!({"revision":revision})))?,
            latest
        );
    }
    for input in [
        json!({"revision":revision,"clear_parent":true}),
        json!({"revision":revision,"depends_on":[]}),
        json!({"revision":revision,"repos":["repo/B","repo/B"]}),
    ] {
        assert_eq!(success(&db, &["update", "4"], Some(input))?, latest);
    }
    for input in [
        json!({"revision":1}),
        json!({"revision":1,"body":"Updated"}),
        json!({"revision":1,"parent":1,"clear_parent":true}),
    ] {
        let stale = failure(&db, &["update", "4"], Some(input), "conflict")?;
        assert_eq!(stale["error"]["current_revision"], revision);
    }
    failure(
        &db,
        &["update", "4"],
        Some(json!({"revision":revision,"parent":1,"clear_parent":true})),
        "invalid_input",
    )?;
    assert_eq!(
        success(&db, &["show", "4"], None)?["data"]["ticket"],
        latest["data"]
    );
    let mut saturated = latest["data"].clone();
    saturated["revision"] = json!(u64::MAX);
    let encoded = serde_json::to_string(&saturated)?;
    let native = redb::Database::open(&db)?;
    let transaction = native.begin_write()?;
    transaction
        .open_table(redb::TableDefinition::<u64, &str>::new("tickets"))?
        .insert(4, encoded.as_str())?;
    transaction.commit()?;
    drop(native);
    let unchanged = success(&db, &["update", "4"], Some(json!({"revision":u64::MAX})))?;
    assert_eq!(unchanged["data"], saturated);
    failure(
        &db,
        &["update", "4"],
        Some(json!({"revision":u64::MAX,"body":"Cannot advance"})),
        "exhausted",
    )?;
    let stale = failure(
        &db,
        &["update", "4"],
        Some(json!({"revision":u64::MAX-1})),
        "conflict",
    )?;
    assert_eq!(stale["error"]["current_revision"], u64::MAX);
    assert_eq!(
        success(&db, &["show", "4"], None)?["data"]["ticket"],
        saturated
    );
    Ok(())
}

#[test]
fn update_revisions_are_positive_before_storage_access() -> TestResult {
    let temp = tempfile::tempdir()?;
    let db = temp.path().join("backlog.redb");
    let missing = temp.path().join("missing/backlog.redb");
    let input_path = temp.path().join("update.json");
    let input_name = input_path.to_str().ok_or("non-UTF-8 fixture path")?;
    let invalid = json!({"revision":0,"body":"Must not apply"});
    std::fs::write(&input_path, serde_json::to_vec(&invalid)?)?;
    success(&db, &["init"], None)?;
    success(
        &db,
        &["add"],
        Some(json!({"title":"Preserve revision","body":"Original"})),
    )?;
    let before = std::fs::read(&db)?;
    let requests = [
        (vec!["update", "1"], Some(invalid)),
        (vec!["update", "1", "--input", input_name], None),
    ];
    for path in [&db, &missing] {
        for (args, input) in &requests {
            let error = failure(path, args, input.clone(), "invalid_input")?;
            assert_eq!(error["error"]["path"], "revision");
            assert_eq!(error["error"]["retryable"], false);
            assert!(error["error"].get("current_revision").is_none());
        }
    }
    assert!(!missing.parent().ok_or("missing fixture parent")?.exists());
    assert_eq!(std::fs::read(&db)?, before);
    let writer = redb::Database::open(&db)?;
    let locked_bytes = std::fs::read(&db)?;
    for (args, input) in &requests {
        let mut locked_args = vec!["--lock-timeout-ms", "0"];
        locked_args.extend(args.iter().copied());
        let error = failure(&db, &locked_args, input.clone(), "invalid_input")?;
        assert_eq!(error["error"]["path"], "revision");
    }
    assert_eq!(std::fs::read(&db)?, locked_bytes);
    drop(writer);
    let ticket = success(&db, &["show", "1"], None)?;
    assert_eq!(ticket["data"]["ticket"]["revision"], 1);
    assert_eq!(ticket["data"]["ticket"]["body"], "Original");
    let schema = success(&db, &["schema", "update"], None)?;
    let validator = jsonschema::validator_for(&schema["data"]["input"])?;
    for (revision, accepted) in [(0, false), (1, true), (u64::MAX, true)] {
        assert_eq!(validator.is_valid(&json!({"revision":revision})), accepted);
    }
    std::fs::write(
        &input_path,
        serde_json::to_vec(&json!({"revision":1,"body":"From file"}))?,
    )?;
    let file_update = success(&db, &["update", "1", "--input", input_name], None)?;
    assert_eq!(file_update["data"]["revision"], 2);
    assert_eq!(file_update["data"]["body"], "From file");
    let stdin_update = success(
        &db,
        &["update", "1"],
        Some(json!({"revision":2,"body":"From stdin"})),
    )?;
    assert_eq!(stdin_update["data"]["revision"], 3);
    assert_eq!(stdin_update["data"]["body"], "From stdin");
    for revision in [1, u64::MAX] {
        let error = failure(
            &db,
            &["update", "1"],
            Some(json!({"revision":revision,"body":"Stale"})),
            "conflict",
        )?;
        assert_eq!(error["error"]["current_revision"], 3);
    }
    assert_eq!(
        success(&db, &["show", "1"], None)?["data"]["ticket"],
        stdin_update["data"]
    );
    Ok(())
}

#[test]
fn request_ids_prevent_duplicate_tickets_and_notes() -> TestResult {
    let temp = tempfile::tempdir()?;
    let db = temp.path().join("db.redb");
    success(&db, &["init"], None)?;
    let input = json!({"title":"Original", "request_id":"capture-1"});
    let first = success(&db, &["add"], Some(input.clone()))?;
    assert_eq!(first, success(&db, &["add"], Some(input))?);
    failure(
        &db,
        &["add"],
        Some(json!({"title":"Other", "request_id":"capture-1"})),
        "request_conflict",
    )?;
    let note = json!({"body":"Resume here", "request_id":"note-1"});
    let first = success(&db, &["note", "1"], Some(note.clone()))?;
    assert_eq!(first, success(&db, &["note", "1"], Some(note))?);
    failure(
        &db,
        &["note", "1"],
        Some(json!({"body":"Different", "request_id":"note-1"})),
        "request_conflict",
    )?;
    assert_eq!(success(&db, &["list"], None)?["data"]["total"], 1);
    assert_eq!(
        success(&db, &["show", "1"], None)?["data"]["notes"]["total"],
        1
    );
    failure(
        &db,
        &["add"],
        Some(json!({"title":" ","request_id":"bad"})),
        "invalid_input",
    )?;
    assert_eq!(
        success(
            &db,
            &["add"],
            Some(json!({"title":"Valid", "request_id":"bad"}))
        )?["data"]["id"],
        2
    );
    Ok(())
}

#[test]
fn detail_reveals_direct_dependents_across_repositories_and_statuses() -> TestResult {
    let temp = tempfile::tempdir()?;
    let db = temp.path().join("db.redb");
    success(&db, &["init"], None)?;
    for input in [
        json!({"title":"Shared foundation","repos":["owner/foundation"]}),
        json!({"title":"Child only","parent":1}),
        json!({"title":"Consumer A","repos":["owner/a"],"depends_on":[1]}),
        json!({"title":"Indirect consumer","depends_on":[3]}),
        json!({"title":"Consumer B","repos":["owner/b"],"depends_on":[1],"priority":"high"}),
    ] {
        success(&db, &["add"], Some(input))?;
    }
    let detail = success(&db, &["show", "1", "--context"], None)?;
    let dependents = detail["data"]["dependents"]
        .as_array()
        .ok_or("missing dependents")?;
    assert_eq!(dependents.len(), 2);
    assert_eq!(dependents[0]["id"], 3);
    assert_eq!(dependents[0]["repos"], json!(["owner/a"]));
    assert_eq!(dependents[0]["blocked_by"], json!([1]));
    assert_eq!(dependents[1]["id"], 5);
    assert_eq!(detail["data"]["children"][0]["id"], 2);
    success(
        &db,
        &["update", "1"],
        Some(json!({"revision":1,"status":"done"})),
    )?;
    for (id, status) in [("3", "done"), ("5", "cancelled")] {
        success(
            &db,
            &["update", id],
            Some(json!({"revision":1,"status":status})),
        )?;
    }
    let detail = success(&db, &["show", "1"], None)?;
    let dependents = detail["data"]["dependents"]
        .as_array()
        .ok_or("missing dependents")?;
    assert_eq!(dependents.len(), 2);
    assert_eq!(dependents[0]["status"], "done");
    assert_eq!(dependents[1]["status"], "cancelled");
    assert!(dependents.iter().all(|d| d["blocked_by"] == json!([])));
    assert_eq!(
        success(&db, &["show", "2"], None)?["data"]["dependents"],
        json!([])
    );
    Ok(())
}

#[test]
fn ready_filter_priority_order_and_note_pagination_are_stable() -> TestResult {
    let temp = tempfile::tempdir()?;
    let db = temp.path().join("db.redb");
    success(&db, &["init"], None)?;
    success(&db, &["add"], Some(json!({"title":"Prerequisite"})))?;
    success(
        &db,
        &["add"],
        Some(json!({"title":"Blocked","depends_on":[1],"priority":"high"})),
    )?;
    success(
        &db,
        &["add"],
        Some(json!({"title":"Ready","priority":"high"})),
    )?;
    let ready = success(&db, &["list", "--ready"], None)?;
    assert_eq!(ready["data"]["items"][0]["id"], 3);
    assert_eq!(ready["data"]["total"], 2);
    let page = success(&db, &["list", "--limit", "1"], None)?;
    assert_eq!(page["data"]["items"][0]["id"], 2);
    assert_eq!(page["data"]["next_offset"], 1);
    success(
        &db,
        &["update", "1"],
        Some(json!({"revision":1,"status":"cancelled"})),
    )?;
    assert_eq!(
        success(&db, &["list", "--ready"], None)?["data"]["total"],
        1
    );
    success(
        &db,
        &["update", "1"],
        Some(json!({"revision":2,"status":"done"})),
    )?;
    assert_eq!(
        success(&db, &["list", "--ready"], None)?["data"]["items"][0]["id"],
        2
    );
    for body in ["older", "newer"] {
        success(&db, &["note", "2"], Some(json!({"body":body})))?;
    }
    let page = success(&db, &["show", "2", "--notes-limit", "1"], None)?;
    assert_eq!(page["data"]["notes"]["items"][0]["body"], "newer");
    assert_eq!(page["data"]["notes"]["next_offset"], 1);
    assert_eq!(
        success(&db, &["show", "2", "--notes-offset", "1"], None)?["data"]["notes"]["items"][0]["body"],
        "older"
    );
    assert_eq!(
        success(&db, &["search", "Prerequisite"], None)?["data"]["total"],
        1
    );
    Ok(())
}

#[test]
fn goal_scoped_bottlenecks_keep_global_paths_and_rank_scoped_consumers() -> TestResult {
    let temp = tempfile::tempdir()?;
    let db = temp.path().join("goal-bottlenecks.redb");
    success(&db, &["init"], None)?;
    for input in [
        json!({"title":"Selected goal","repos":["repo/goal"]}),
        json!({"title":"Nested work","repos":["repo/child"]}),
        json!({"title":"Nested leaf","repos":["repo/leaf"]}),
        json!({"title":"Broad external foundation","repos":["repo/foundationA"],"priority":"high"}),
        json!({"title":"Outside intermediate","repos":["repo/bridge"]}),
        json!({"title":"Goal-specific foundation","repos":["repo/foundationB"],"priority":"low"}),
        json!({"title":"Unrelated intermediate","repos":["repo/unrelated"]}),
        json!({"title":"Unrelated consumer","repos":["repo/unrelated"]}),
        json!({"title":"Completed parent","repos":["repo/closed"]}),
        json!({"title":"Work beneath completion","repos":["repo/child"]}),
        json!({"title":"Cancelled parent","repos":["repo/closed"]}),
        json!({"title":"Work beneath cancellation","repos":["repo/child"]}),
        json!({"title":"Cancelled external prerequisite","priority":"high"}),
        json!({"title":"Consumer of cancellation","repos":["repo/child"]}),
        json!({"title":"Completed dependency bridge","repos":["repo/outside"]}),
        json!({"title":"Consumer beyond completion","repos":["repo/child"]}),
        json!({"title":"Another unrelated consumer","repos":["repo/unrelated"]}),
        json!({"title":"Independent candidate goal","repos":["repo/other"]}),
    ] {
        success(&db, &["add"], Some(input))?;
    }
    for (id, input) in [
        ("1", json!({"revision":1,"depends_on":[5]})),
        ("2", json!({"revision":1,"parent":1,"depends_on":[6]})),
        ("3", json!({"revision":1,"parent":2,"depends_on":[2,6]})),
        ("4", json!({"revision":1,"parent":18})),
        ("5", json!({"revision":1,"parent":18,"depends_on":[4]})),
        ("6", json!({"revision":1,"parent":18})),
        ("7", json!({"revision":1,"depends_on":[4]})),
        ("8", json!({"revision":1,"depends_on":[7]})),
        (
            "9",
            json!({"revision":1,"parent":1,"status":"done","depends_on":[4]}),
        ),
        ("10", json!({"revision":1,"parent":9,"depends_on":[6]})),
        (
            "11",
            json!({"revision":1,"parent":1,"status":"cancelled","depends_on":[4]}),
        ),
        ("12", json!({"revision":1,"parent":11,"depends_on":[6]})),
        (
            "13",
            json!({"revision":1,"parent":18,"status":"cancelled","depends_on":[4]}),
        ),
        ("14", json!({"revision":1,"parent":1,"depends_on":[13]})),
        ("15", json!({"revision":1,"status":"done","depends_on":[6]})),
        ("16", json!({"revision":1,"parent":1,"depends_on":[15]})),
        ("17", json!({"revision":1,"depends_on":[7]})),
    ] {
        success(&db, &["update", id], Some(input))?;
    }
    let ranked = |value: &Value| -> Result<Vec<(u64, u64)>, &'static str> {
        value["data"]["items"]
            .as_array()
            .ok_or("missing scoped bottleneck items")?
            .iter()
            .map(|item| {
                Ok((
                    item["ticket"]["id"]
                        .as_u64()
                        .ok_or("missing scoped bottleneck ID")?,
                    item["affected"]
                        .as_u64()
                        .ok_or("missing scoped affected count")?,
                ))
            })
            .collect()
    };
    let baseline = success(&db, &["bottlenecks"], None)?;
    assert_eq!(
        ranked(&baseline)?,
        vec![(4, 5), (6, 4), (7, 2), (13, 1), (2, 1), (5, 1)]
    );
    let scoped = success(&db, &["bottlenecks", "--for-goal", "1"], None)?;
    assert_eq!(
        ranked(&scoped)?,
        vec![(6, 4), (4, 1), (13, 1), (2, 1), (5, 1)]
    );
    assert_eq!(scoped["data"]["total"], 5);
    for (args, expected) in [
        (
            &["bottlenecks", "--for-goal", "1", "--ready"][..],
            vec![(6, 4), (4, 1)],
        ),
        (
            &[
                "bottlenecks",
                "--for-goal",
                "1",
                "--repo",
                "repo/foundationA",
            ][..],
            vec![(4, 1)],
        ),
        (
            &["bottlenecks", "--for-goal", "1", "--unassigned"][..],
            vec![(13, 1)],
        ),
        (
            &["bottlenecks", "--for-goal", "1", "--unassigned", "--ready"][..],
            vec![],
        ),
        (
            &["bottlenecks", "--for-goal", "1", "--under", "1"][..],
            vec![(2, 1)],
        ),
        (
            &["bottlenecks", "--for-goal", "1", "--under", "18"][..],
            vec![(6, 4), (4, 1), (13, 1), (5, 1)],
        ),
        (
            &[
                "bottlenecks",
                "--for-goal",
                "1",
                "--under",
                "18",
                "--repo",
                "repo/foundationB",
                "--ready",
            ][..],
            vec![(6, 4)],
        ),
        (
            &["bottlenecks", "--for-goal", "2"][..],
            vec![(6, 2), (2, 1)],
        ),
        (
            &["bottlenecks", "--for-goal", "3"][..],
            vec![(2, 1), (6, 1)],
        ),
        (&["bottlenecks", "--for-goal", "9"][..], vec![(6, 1)]),
        (&["bottlenecks", "--for-goal", "11"][..], vec![(6, 1)]),
        (&["bottlenecks", "--for-goal", "13"][..], vec![]),
        (
            &["bottlenecks", "--for-goal", "18", "--under", "1"][..],
            vec![],
        ),
    ] {
        let response = success(&db, args, None)?;
        assert_eq!(ranked(&response)?, expected, "{args:?}");
        assert_eq!(response["data"]["total"], expected.len());
    }
    let page = success(
        &db,
        &[
            "bottlenecks",
            "--for-goal",
            "1",
            "--limit",
            "2",
            "--offset",
            "1",
        ],
        None,
    )?;
    assert_eq!(ranked(&page)?, vec![(4, 1), (13, 1)]);
    assert_eq!(page["data"]["total"], 5);
    assert_eq!(page["data"]["next_offset"], 3);
    let maximum_offset = usize::MAX.to_string();
    assert_eq!(
        success(
            &db,
            &[
                "bottlenecks",
                "--for-goal",
                "1",
                "--offset",
                &maximum_offset
            ],
            None
        )?["data"],
        json!({"items":[],"total":5,"next_offset":null})
    );
    for args in [
        &[
            "bottlenecks",
            "--for-goal",
            "99",
            "--repo",
            "absent",
            "--ready",
        ][..],
        &[
            "bottlenecks",
            "--for-goal",
            "99",
            "--under",
            "3",
            "--unassigned",
        ][..],
        &[
            "bottlenecks",
            "--for-goal",
            "1",
            "--under",
            "99",
            "--repo",
            "absent",
        ][..],
    ] {
        failure(&db, args, None, "not_found")?;
    }
    assert_eq!(success(&db, &["bottlenecks"], None)?, baseline);
    success(
        &db,
        &["update", "3"],
        Some(json!({"revision":2,"parent":18})),
    )?;
    let moved = success(&db, &["bottlenecks", "--for-goal", "1"], None)?;
    assert_eq!(ranked(&moved)?, vec![(6, 3), (4, 1), (13, 1), (5, 1)]);
    assert_eq!(
        ranked(&success(
            &db,
            &["bottlenecks", "--for-goal", "18", "--under", "1"],
            None
        )?)?,
        vec![(2, 1)]
    );
    assert_eq!(success(&db, &["bottlenecks"], None)?, baseline);
    for (id, dependencies) in [("14", json!([13, 4])), ("16", json!([15, 2]))] {
        success(
            &db,
            &["update", id],
            Some(json!({"revision":2,"depends_on":dependencies})),
        )?;
    }
    let alternatives = success(&db, &["bottlenecks", "--for-goal", "1"], None)?;
    assert_eq!(
        ranked(&alternatives)?,
        vec![(6, 4), (4, 2), (13, 1), (2, 1), (5, 1)]
    );
    success(
        &db,
        &["update", "1"],
        Some(json!({"revision":2,"status":"done"})),
    )?;
    let completed_root = success(&db, &["bottlenecks", "--for-goal", "1"], None)?;
    assert_eq!(
        ranked(&completed_root)?,
        vec![(6, 4), (4, 1), (13, 1), (2, 1)]
    );
    let schema = success(&db, &["schema", "bottlenecks"], None)?;
    let output = jsonschema::validator_for(&schema["data"]["output"])?;
    for response in [
        &baseline,
        &scoped,
        &page,
        &moved,
        &alternatives,
        &completed_root,
    ] {
        assert!(output.is_valid(response), "{response}");
    }
    Ok(())
}

#[test]
fn goal_scoped_bottlenecks_count_consumers_beyond_impact_default_page() -> TestResult {
    let temp = tempfile::tempdir()?;
    let db = temp.path().join("many-goal-consumers.redb");
    success(&db, &["init"], None)?;
    success(&db, &["add"], Some(json!({"title":"Closed goal"})))?;
    success(&db, &["add"], Some(json!({"title":"External foundation"})))?;
    for index in 0..51 {
        success(
            &db,
            &["add"],
            Some(json!({"title":format!("Consumer {index}")})),
        )?;
    }
    success(
        &db,
        &["update", "1"],
        Some(json!({"revision":1,"status":"done"})),
    )?;
    for id in 3..=53 {
        let id = id.to_string();
        success(
            &db,
            &["update", &id],
            Some(json!({"revision":1,"parent":1,"depends_on":[2]})),
        )?;
    }
    let impact = success(&db, &["impact", "2"], None)?;
    assert_eq!(impact["data"]["total"], 51);
    assert_eq!(
        impact["data"]["items"]
            .as_array()
            .ok_or("missing impact items")?
            .len(),
        50
    );
    let scoped = success(
        &db,
        &["bottlenecks", "--for-goal", "1", "--limit", "1"],
        None,
    )?;
    assert_eq!(scoped["data"]["total"], 1);
    assert!(scoped["data"]["next_offset"].is_null());
    assert_eq!(scoped["data"]["items"][0]["ticket"]["id"], 2);
    assert_eq!(scoped["data"]["items"][0]["affected"], 51);
    let leaf = success(&db, &["bottlenecks", "--for-goal", "3"], None)?;
    assert_eq!(leaf["data"]["total"], 1);
    assert_eq!(leaf["data"]["items"][0]["ticket"]["id"], 2);
    assert_eq!(leaf["data"]["items"][0]["affected"], 1);
    assert_eq!(
        success(
            &db,
            &["bottlenecks", "--for-goal", "1", "--offset", "1"],
            None
        )?["data"],
        json!({"items":[],"total":1,"next_offset":null})
    );
    Ok(())
}

#[test]
fn goal_scoped_bottleneck_schema_requires_positive_optional_anchor() -> TestResult {
    let temp = tempfile::tempdir()?;
    let db = temp.path().join("missing/backlog.redb");
    let schema = success(&db, &["schema", "bottlenecks"], None)?;
    let input = jsonschema::validator_for(&schema["data"]["input"])?;
    for value in [
        json!({}),
        json!({"for_goal":null}),
        json!({"for_goal":1,"under":2,"ready":true,"unassigned":true}),
        json!({"for_goal":u64::MAX,"repo":"repo/A"}),
    ] {
        assert!(input.is_valid(&value), "{value}");
    }
    for value in [
        json!({"for_goal":0}),
        json!({"for_goal":-1}),
        json!({"for_goal":"1"}),
    ] {
        assert!(!input.is_valid(&value), "{value}");
    }
    failure(
        &db,
        &["bottlenecks", "--for-goal", "0"],
        None,
        "invalid_input",
    )?;
    failure(
        &db,
        &["bottlenecks", "--for-goal", "1"],
        None,
        "not_initialized",
    )?;
    assert!(!db.parent().ok_or("missing fixture parent")?.exists());
    Ok(())
}

#[test]
fn bottleneck_queries_rank_global_impact_before_candidate_filters() -> TestResult {
    let temp = tempfile::tempdir()?;
    let db = temp.path().join("bottlenecks.redb");
    success(&db, &["init"], None)?;
    let empty = success(&db, &["bottlenecks"], None)?;
    assert_eq!(
        empty["data"],
        json!({"items":[],"total":0,"next_offset":null})
    );
    assert_eq!(success(&db, &["bottlenecks", "--ready"], None)?, empty);
    failure(&db, &["bottlenecks", "--under", "99"], None, "not_found")?;
    for input in [
        json!({"title":"Shared foundation","repos":["repo/foundation"],"priority":"low"}),
        json!({"title":"Direct consumer","repos":["repo/projectA"],"priority":"high"}),
        json!({"title":"Indirect consumer","repos":["repo/projectB"],"priority":"high"}),
        json!({"title":"Second active path","repos":["repo/projectC"]}),
        json!({"title":"Shared consumer","repos":["repo/projectB"],"priority":"high"}),
        json!({"title":"Completed bridge","repos":["repo/projectA"],"priority":"high"}),
        json!({"title":"Beyond completion","repos":["repo/projectA"]}),
        json!({"title":"Cancelled bridge","repos":["repo/projectA"],"priority":"high"}),
        json!({"title":"Beyond cancellation","repos":["repo/projectA"]}),
        json!({"title":"Unblocked progress","repos":["repo/projecta"],"priority":"high"}),
        json!({"title":"Progress consumer","repos":["repo/consumer"]}),
        json!({"title":"Unassigned prerequisite","priority":"high"}),
        json!({"title":"Completed prerequisite","repos":["repo/projectA"],"priority":"high"}),
        json!({"title":"Consumer of completed prerequisite","repos":["repo/projectA"]}),
        json!({"title":"Parent scope","repos":["repo/projectA"],"priority":"high"}),
        json!({"title":"Parent's actual consumer","repos":["repo/consumer"]}),
    ] {
        success(&db, &["add"], Some(input))?;
    }
    for (id, input) in [
        ("1", json!({"revision":1,"parent":15})),
        ("2", json!({"revision":1,"parent":15,"depends_on":[1]})),
        ("3", json!({"revision":1,"depends_on":[2]})),
        (
            "4",
            json!({"revision":1,"parent":1,"status":"in_progress","depends_on":[1]}),
        ),
        ("5", json!({"revision":1,"depends_on":[2,4,12]})),
        ("6", json!({"revision":1,"status":"done","depends_on":[1]})),
        ("7", json!({"revision":1,"depends_on":[6]})),
        (
            "8",
            json!({"revision":1,"parent":15,"status":"cancelled","depends_on":[1]}),
        ),
        ("9", json!({"revision":1,"depends_on":[8]})),
        ("10", json!({"revision":1,"status":"in_progress"})),
        ("11", json!({"revision":1,"depends_on":[10]})),
        ("12", json!({"revision":1,"parent":15})),
        ("13", json!({"revision":1,"status":"done"})),
        ("14", json!({"revision":1,"depends_on":[13]})),
        ("16", json!({"revision":1,"depends_on":[15]})),
    ] {
        success(&db, &["update", id], Some(input))?;
    }
    let ranked = |value: &Value| -> Result<Vec<(u64, u64)>, &'static str> {
        value["data"]["items"]
            .as_array()
            .ok_or("missing bottleneck items")?
            .iter()
            .map(|item| {
                Ok((
                    item["ticket"]["id"]
                        .as_u64()
                        .ok_or("missing bottleneck ID")?,
                    item["affected"].as_u64().ok_or("missing affected count")?,
                ))
            })
            .collect()
    };
    let all = success(&db, &["bottlenecks"], None)?;
    assert_eq!(
        ranked(&all)?,
        vec![(1, 4), (2, 2), (8, 1), (10, 1), (12, 1), (15, 1), (4, 1)]
    );
    assert_eq!(all["data"]["total"], 7);
    assert!(all["data"]["next_offset"].is_null());
    assert_eq!(all["data"]["items"][1]["ticket"]["blocked_by"], json!([1]));
    assert_eq!(all["data"]["items"][2]["ticket"]["status"], "cancelled");
    for (id, affected) in ranked(&all)? {
        let id = id.to_string();
        assert_eq!(
            success(&db, &["impact", &id], None)?["data"]["total"],
            affected
        );
    }
    let ready = success(&db, &["bottlenecks", "--ready"], None)?;
    assert_eq!(ranked(&ready)?, vec![(1, 4), (12, 1), (15, 1)]);
    assert_eq!(ready["data"]["total"], 3);
    let repo = success(&db, &["bottlenecks", "--repo", "repo/projectA"], None)?;
    assert_eq!(ranked(&repo)?, vec![(2, 2), (8, 1), (15, 1)]);
    assert_eq!(
        ranked(&success(
            &db,
            &["bottlenecks", "--repo", "repo/projecta"],
            None
        )?)?,
        vec![(10, 1)]
    );
    assert_eq!(
        ranked(&success(
            &db,
            &["bottlenecks", "--repo", "repo/foundation"],
            None
        )?)?,
        vec![(1, 4)]
    );
    assert_eq!(
        ranked(&success(&db, &["bottlenecks", "--unassigned"], None)?)?,
        vec![(12, 1)]
    );
    let under = success(&db, &["bottlenecks", "--under", "15"], None)?;
    assert_eq!(
        ranked(&under)?,
        vec![(1, 4), (2, 2), (8, 1), (12, 1), (4, 1)]
    );
    assert_eq!(under["data"]["total"], 5);
    assert_eq!(
        ranked(&success(&db, &["bottlenecks", "--under", "1"], None)?)?,
        vec![(4, 1)]
    );
    assert_eq!(
        ranked(&success(
            &db,
            &["bottlenecks", "--under", "15", "--ready"],
            None
        )?)?,
        vec![(1, 4), (12, 1)]
    );
    assert_eq!(
        ranked(&success(
            &db,
            &["bottlenecks", "--under", "15", "--repo", "repo/projectA"],
            None
        )?)?,
        vec![(2, 2), (8, 1)]
    );
    assert_eq!(
        ranked(&success(
            &db,
            &["bottlenecks", "--under", "15", "--unassigned"],
            None
        )?)?,
        vec![(12, 1)]
    );
    let page = success(&db, &["bottlenecks", "--limit", "2", "--offset", "1"], None)?;
    assert_eq!(ranked(&page)?, vec![(2, 2), (8, 1)]);
    assert_eq!(page["data"]["total"], 7);
    assert_eq!(page["data"]["next_offset"], 3);
    let ready_page = success(
        &db,
        &["bottlenecks", "--ready", "--limit", "1", "--offset", "1"],
        None,
    )?;
    assert_eq!(ranked(&ready_page)?, vec![(12, 1)]);
    assert_eq!(ready_page["data"]["total"], 3);
    assert_eq!(ready_page["data"]["next_offset"], 2);
    let maximum_offset = usize::MAX.to_string();
    for (args, total) in [
        (vec!["bottlenecks", "--offset", &maximum_offset], 7),
        (vec!["bottlenecks", "--ready", "--offset", "3"], 3),
        (vec!["bottlenecks", "--under", "3"], 0),
        (vec!["bottlenecks", "--repo", "absent"], 0),
    ] {
        assert_eq!(
            success(&db, &args, None)?["data"],
            json!({"items":[],"total":total,"next_offset":null})
        );
    }
    failure(
        &db,
        &[
            "bottlenecks",
            "--under",
            "99",
            "--repo",
            "absent",
            "--ready",
        ],
        None,
        "not_found",
    )?;
    for (id, dependencies) in [("7", json!([6, 2])), ("9", json!([8, 2]))] {
        success(
            &db,
            &["update", id],
            Some(json!({"revision":2,"depends_on":dependencies})),
        )?;
    }
    let alternatives = success(&db, &["bottlenecks"], None)?;
    assert_eq!(
        ranked(&alternatives)?,
        vec![(1, 6), (2, 4), (8, 1), (10, 1), (12, 1), (15, 1), (4, 1)]
    );
    success(
        &db,
        &["update", "2"],
        Some(json!({"revision":2,"status":"done"})),
    )?;
    let completed_bridge = success(&db, &["bottlenecks"], None)?;
    assert_eq!(
        ranked(&completed_bridge)?,
        vec![(1, 2), (8, 1), (10, 1), (12, 1), (15, 1), (4, 1)]
    );
    success(
        &db,
        &["update", "1"],
        Some(json!({"revision":2,"status":"done"})),
    )?;
    let completed_root = success(&db, &["bottlenecks"], None)?;
    assert_eq!(
        ranked(&completed_root)?,
        vec![(8, 1), (10, 1), (12, 1), (15, 1), (4, 1)]
    );
    assert_eq!(
        ranked(&success(&db, &["bottlenecks", "--ready"], None)?)?,
        vec![(12, 1), (15, 1)]
    );
    let schema = success(&db, &["schema", "bottlenecks"], None)?;
    let output = jsonschema::validator_for(&schema["data"]["output"])?;
    for response in [
        &empty,
        &all,
        &ready,
        &repo,
        &under,
        &page,
        &ready_page,
        &alternatives,
        &completed_bridge,
        &completed_root,
    ] {
        assert!(output.is_valid(response), "{response}");
    }
    let mut invalid_count = all.clone();
    invalid_count["data"]["items"][0]["affected"] = json!(0);
    assert!(!output.is_valid(&invalid_count));
    Ok(())
}

#[test]
fn bottleneck_schema_matches_candidate_filters_before_storage_access() -> TestResult {
    let temp = tempfile::tempdir()?;
    let db = temp.path().join("missing/backlog.redb");
    let schema = success(&db, &["schema", "bottlenecks"], None)?;
    let input = jsonschema::validator_for(&schema["data"]["input"])?;
    for value in [
        json!({}),
        json!({"ready":true,"unassigned":true,"repo":null,"under":null,"limit":1}),
        json!({"ready":false,"unassigned":false,"repo":"repo/A","under":1,"limit":500,"offset":usize::MAX}),
    ] {
        assert!(input.is_valid(&value), "{value}");
    }
    for value in [
        json!({"under":0}),
        json!({"repo":"repo/A","unassigned":true}),
        json!({"ready":null}),
    ] {
        assert!(!input.is_valid(&value), "{value}");
    }
    for limit in [0, 501] {
        assert!(!input.is_valid(&json!({"limit":limit})));
        failure(
            &db,
            &["bottlenecks", "--limit", &limit.to_string()],
            None,
            "invalid_input",
        )?;
    }
    for args in [
        vec!["bottlenecks", "--under", "0"],
        vec!["bottlenecks", "--unassigned", "--repo", "repo/A"],
        vec!["bottlenecks", "--repo", "repo/A", "--unassigned"],
    ] {
        failure(&db, &args, None, "invalid_input")?;
    }
    assert!(!db.parent().ok_or("missing fixture parent")?.exists());
    Ok(())
}

#[test]
fn goal_overviews_include_complete_trees_before_filtering_and_pagination() -> TestResult {
    let temp = tempfile::tempdir()?;
    let db = temp.path().join("goals-overview.redb");
    success(&db, &["init"], None)?;
    let empty = success(&db, &["goals"], None)?;
    assert_eq!(
        empty["data"],
        json!({"items":[],"total":0,"next_offset":null})
    );
    assert_eq!(success(&db, &["goals", "--active"], None)?, empty);
    for input in [
        json!({"title":"Older deep work","repos":["repo/A"],"priority":"high"}),
        json!({"title":"Low-priority goal","repos":["repo/goal"],"priority":"low"}),
        json!({"title":"Completed bridge","repos":["repo/B"]}),
        json!({"title":"Cancelled bridge","repos":["repo/C"]}),
        json!({"title":"External prerequisite","repos":["repo/external"],"priority":"low"}),
        json!({"title":"Closed goal with active work","priority":"high"}),
        json!({"title":"Active child","repos":["repo/child"]}),
        json!({"title":"Closed-only goal","priority":"high"}),
        json!({"title":"Closed descendant","priority":"high"}),
        json!({"title":"Standalone ready work"}),
        json!({"title":"Independent dependent goal","priority":"high"}),
        json!({"title":"Ready deep sibling","repos":["repo/D"],"priority":"low"}),
    ] {
        success(&db, &["add"], Some(input))?;
    }
    for (id, input) in [
        ("1", json!({"revision":1,"parent":4,"depends_on":[5]})),
        ("3", json!({"revision":1,"parent":2,"status":"done"})),
        ("4", json!({"revision":1,"parent":3,"status":"cancelled"})),
        ("5", json!({"revision":1,"status":"cancelled"})),
        ("6", json!({"revision":1,"status":"done"})),
        (
            "7",
            json!({"revision":1,"parent":6,"status":"in_progress","depends_on":[5]}),
        ),
        ("8", json!({"revision":1,"status":"cancelled"})),
        ("9", json!({"revision":1,"parent":8,"status":"done"})),
        ("11", json!({"revision":1,"depends_on":[1]})),
        ("12", json!({"revision":1,"parent":4})),
    ] {
        success(&db, &["update", id], Some(input))?;
    }
    let ids = |value: &Value| -> Result<Vec<u64>, &'static str> {
        value["data"]["items"]
            .as_array()
            .ok_or("missing goal items")?
            .iter()
            .map(|item| item["goal"]["id"].as_u64().ok_or("missing goal ID"))
            .collect()
    };
    let work = |value: &Value, id: u64| -> Result<Value, &'static str> {
        value["data"]["items"]
            .as_array()
            .ok_or("missing goal items")?
            .iter()
            .find(|item| item["goal"]["id"].as_u64() == Some(id))
            .map(|item| item["work"].clone())
            .ok_or("missing goal summary")
    };
    let all = success(&db, &["goals"], None)?;
    assert_eq!(ids(&all)?, vec![6, 8, 11, 10, 2, 5]);
    assert_eq!(all["data"]["total"], 6);
    assert!(all["data"]["next_offset"].is_null());
    for (id, expected) in [
        (
            6,
            json!({"total":2,"open":0,"in_progress":1,"done":1,"cancelled":0,"ready":0,"blocked":1}),
        ),
        (
            8,
            json!({"total":2,"open":0,"in_progress":0,"done":1,"cancelled":1,"ready":0,"blocked":0}),
        ),
        (
            11,
            json!({"total":1,"open":1,"in_progress":0,"done":0,"cancelled":0,"ready":0,"blocked":1}),
        ),
        (
            10,
            json!({"total":1,"open":1,"in_progress":0,"done":0,"cancelled":0,"ready":1,"blocked":0}),
        ),
        (
            2,
            json!({"total":5,"open":3,"in_progress":0,"done":1,"cancelled":1,"ready":2,"blocked":1}),
        ),
        (
            5,
            json!({"total":1,"open":0,"in_progress":0,"done":0,"cancelled":1,"ready":0,"blocked":0}),
        ),
    ] {
        assert_eq!(work(&all, id)?, expected, "goal {id}");
    }
    for item in all["data"]["items"]
        .as_array()
        .ok_or("missing goal items")?
    {
        assert!(item["goal"]["parent"].is_null());
    }
    assert_eq!(all["data"]["items"][0]["goal"]["status"], "done");
    assert_eq!(all["data"]["items"][2]["goal"]["blocked_by"], json!([1]));
    assert_eq!(all["data"]["items"][4]["goal"]["blocked_by"], json!([]));
    let page = success(&db, &["goals", "--limit", "2", "--offset", "1"], None)?;
    assert_eq!(ids(&page)?, vec![8, 11]);
    assert_eq!(page["data"]["total"], 6);
    assert_eq!(page["data"]["next_offset"], 3);
    assert_eq!(work(&page, 8)?, work(&all, 8)?);
    let active = success(&db, &["goals", "--active"], None)?;
    assert_eq!(ids(&active)?, vec![6, 11, 10, 2]);
    assert_eq!(active["data"]["total"], 4);
    assert_eq!(work(&active, 6)?, work(&all, 6)?);
    assert_eq!(work(&active, 2)?, work(&all, 2)?);
    let active_page = success(
        &db,
        &["goals", "--active", "--limit", "2", "--offset", "1"],
        None,
    )?;
    assert_eq!(ids(&active_page)?, vec![11, 10]);
    assert_eq!(active_page["data"]["total"], 4);
    assert_eq!(active_page["data"]["next_offset"], 3);
    let maximum_offset = usize::MAX.to_string();
    for (args, total) in [
        (vec!["goals", "--offset", &maximum_offset], 6),
        (vec!["goals", "--active", "--offset", "4"], 4),
    ] {
        assert_eq!(
            success(&db, &args, None)?["data"],
            json!({"items":[],"total":total,"next_offset":null})
        );
    }
    success(
        &db,
        &["update", "5"],
        Some(json!({"revision":2,"status":"done"})),
    )?;
    let unblocked = success(&db, &["goals"], None)?;
    assert_eq!(
        work(&unblocked, 2)?,
        json!({"total":5,"open":3,"in_progress":0,"done":1,"cancelled":1,"ready":3,"blocked":0})
    );
    assert_eq!(
        work(&unblocked, 6)?,
        json!({"total":2,"open":0,"in_progress":1,"done":1,"cancelled":0,"ready":0,"blocked":0})
    );
    assert_eq!(work(&unblocked, 11)?, work(&all, 11)?);
    success(
        &db,
        &["update", "1"],
        Some(json!({"revision":2,"parent":6})),
    )?;
    let moved = success(&db, &["goals"], None)?;
    assert_eq!(ids(&moved)?, ids(&all)?);
    assert_eq!(
        work(&moved, 2)?,
        json!({"total":4,"open":2,"in_progress":0,"done":1,"cancelled":1,"ready":2,"blocked":0})
    );
    assert_eq!(
        work(&moved, 6)?,
        json!({"total":3,"open":1,"in_progress":1,"done":1,"cancelled":0,"ready":1,"blocked":0})
    );
    success(
        &db,
        &["update", "1"],
        Some(json!({"revision":3,"clear_parent":true})),
    )?;
    let split = success(&db, &["goals"], None)?;
    assert_eq!(ids(&split)?, vec![1, 6, 8, 11, 10, 2, 5]);
    assert_eq!(split["data"]["total"], 7);
    assert_eq!(
        work(&split, 1)?,
        json!({"total":1,"open":1,"in_progress":0,"done":0,"cancelled":0,"ready":1,"blocked":0})
    );
    success(
        &db,
        &["update", "10"],
        Some(json!({"revision":1,"parent":1})),
    )?;
    let merged = success(&db, &["goals"], None)?;
    assert_eq!(ids(&merged)?, vec![1, 6, 8, 11, 2, 5]);
    assert_eq!(merged["data"]["total"], 6);
    assert_eq!(
        work(&merged, 1)?,
        json!({"total":2,"open":2,"in_progress":0,"done":0,"cancelled":0,"ready":2,"blocked":0})
    );
    success(
        &db,
        &["update", "1"],
        Some(json!({"revision":4,"parent":2})),
    )?;
    let combined = success(&db, &["goals"], None)?;
    assert_eq!(ids(&combined)?, vec![6, 8, 11, 2, 5]);
    assert_eq!(combined["data"]["total"], 5);
    assert_eq!(
        work(&combined, 2)?,
        json!({"total":6,"open":4,"in_progress":0,"done":1,"cancelled":1,"ready":4,"blocked":0})
    );
    let schema = success(&db, &["schema", "goals"], None)?;
    let output = jsonschema::validator_for(&schema["data"]["output"])?;
    for response in [
        &empty,
        &all,
        &page,
        &active,
        &active_page,
        &unblocked,
        &moved,
        &split,
        &merged,
        &combined,
    ] {
        assert!(output.is_valid(response), "{response}");
    }
    for response in [&all, &unblocked, &moved, &split, &merged, &combined] {
        let totals = response["data"]["items"]
            .as_array()
            .ok_or("missing goal items")?
            .iter()
            .map(|item| {
                item["work"]["total"]
                    .as_u64()
                    .ok_or("missing goal work total")
            })
            .collect::<Result<Vec<_>, _>>()?;
        assert_eq!(totals.into_iter().sum::<u64>(), 12);
    }
    Ok(())
}

#[test]
fn goal_overview_schema_and_limits_match_cli_before_storage_access() -> TestResult {
    let temp = tempfile::tempdir()?;
    let db = temp.path().join("missing/backlog.redb");
    let schema = success(&db, &["schema", "goals"], None)?;
    let input = jsonschema::validator_for(&schema["data"]["input"])?;
    for value in [
        json!({}),
        json!({"active":true,"limit":1}),
        json!({"active":false,"limit":500,"offset":usize::MAX}),
    ] {
        assert!(input.is_valid(&value), "{value}");
    }
    for limit in [0, 501] {
        assert!(!input.is_valid(&json!({"limit":limit})));
        failure(
            &db,
            &["goals", "--limit", &limit.to_string()],
            None,
            "invalid_input",
        )?;
    }
    for value in [
        json!({"active":null}),
        json!({"active":"true"}),
        json!({"offset":-1}),
    ] {
        assert!(!input.is_valid(&value), "{value}");
    }
    assert!(!db.parent().ok_or("missing fixture parent")?.exists());
    Ok(())
}

#[test]
fn repository_counts_use_distinct_memberships_before_filtering_and_pagination() -> TestResult {
    let temp = tempfile::tempdir()?;
    let db = temp.path().join("repos.redb");
    success(&db, &["init"], None)?;
    let empty = success(&db, &["repos"], None)?;
    assert_eq!(
        empty["data"],
        json!({"items":[],"total":0,"next_offset":null})
    );
    assert_eq!(success(&db, &["repos", "--active"], None)?, empty);
    for input in [
        json!({"title":"Unassigned ready"}),
        json!({"title":"Unassigned history"}),
        json!({"title":"Cancelled prerequisite","repos":["repo/B"]}),
        json!({"title":"Completed prerequisite","repos":["repo/C"]}),
        json!({"title":"Blocked work","repos":["repo/A"],"depends_on":[3]}),
        json!({"title":"Unblocked progress","repos":["repo/A"],"depends_on":[4]}),
        json!({"title":"Shared ready work","repos":["repo/A","repo/B"]}),
        json!({"title":"Completed consumer","repos":["repo/A"],"depends_on":[3]}),
        json!({"title":"Cancelled consumer","repos":["repo/A"],"depends_on":[5]}),
        json!({"title":"Case-sensitive history","repos":["repo/a"]}),
    ] {
        success(&db, &["add"], Some(input))?;
    }
    for (id, status) in [
        ("2", "done"),
        ("3", "cancelled"),
        ("4", "done"),
        ("6", "in_progress"),
        ("8", "done"),
        ("9", "cancelled"),
        ("10", "done"),
    ] {
        success(
            &db,
            &["update", id],
            Some(json!({"revision":1,"status":status})),
        )?;
    }
    let mut shared = success(&db, &["show", "7"], None)?["data"]["ticket"].clone();
    shared["repos"] = json!(["repo/B", "repo/A", "repo/A"]);
    let encoded = serde_json::to_string(&shared)?;
    let native = redb::Database::open(&db)?;
    let transaction = native.begin_write()?;
    transaction
        .open_table(redb::TableDefinition::<u64, &str>::new("tickets"))?
        .insert(7, encoded.as_str())?;
    transaction.commit()?;
    drop(native);
    let expected = json!([
        {"repo":null,"total":2,"open":1,"in_progress":0,"done":1,"cancelled":0,"ready":1,"blocked":0},
        {"repo":"repo/A","total":5,"open":2,"in_progress":1,"done":1,"cancelled":1,"ready":1,"blocked":1},
        {"repo":"repo/B","total":2,"open":1,"in_progress":0,"done":0,"cancelled":1,"ready":1,"blocked":0},
        {"repo":"repo/C","total":1,"open":0,"in_progress":0,"done":1,"cancelled":0,"ready":0,"blocked":0},
        {"repo":"repo/a","total":1,"open":0,"in_progress":0,"done":1,"cancelled":0,"ready":0,"blocked":0}
    ]);
    let all = success(&db, &["repos"], None)?;
    assert_eq!(all["data"]["items"], expected);
    assert_eq!(all["data"]["total"], 5);
    assert!(all["data"]["next_offset"].is_null());
    let page = success(&db, &["repos", "--limit", "2", "--offset", "1"], None)?;
    assert_eq!(page["data"]["items"], json!([expected[1], expected[2]]));
    assert_eq!(page["data"]["total"], 5);
    assert_eq!(page["data"]["next_offset"], 3);
    let active = success(&db, &["repos", "--active"], None)?;
    assert_eq!(
        active["data"]["items"],
        json!([expected[0], expected[1], expected[2]])
    );
    assert_eq!(active["data"]["total"], 3);
    let active_page = success(
        &db,
        &["repos", "--active", "--limit", "2", "--offset", "1"],
        None,
    )?;
    assert_eq!(
        active_page["data"]["items"],
        json!([expected[1], expected[2]])
    );
    assert_eq!(active_page["data"]["total"], 3);
    assert!(active_page["data"]["next_offset"].is_null());
    let maximum_offset = usize::MAX.to_string();
    for (args, total) in [
        (vec!["repos", "--offset", &maximum_offset], 5),
        (vec!["repos", "--active", "--offset", "3"], 3),
    ] {
        assert_eq!(
            success(&db, &args, None)?["data"],
            json!({"items":[],"total":total,"next_offset":null})
        );
    }
    success(
        &db,
        &["update", "3"],
        Some(json!({"revision":2,"status":"done"})),
    )?;
    let refreshed = success(&db, &["repos"], None)?;
    assert_eq!(
        refreshed["data"]["items"][1],
        json!({"repo":"repo/A","total":5,"open":2,"in_progress":1,"done":1,"cancelled":1,"ready":2,"blocked":0})
    );
    assert_eq!(
        refreshed["data"]["items"][2],
        json!({"repo":"repo/B","total":2,"open":1,"in_progress":0,"done":1,"cancelled":0,"ready":1,"blocked":0})
    );
    success(
        &db,
        &["update", "4"],
        Some(json!({"revision":2,"repos":[]})),
    )?;
    let reassigned = success(&db, &["repos"], None)?;
    assert_eq!(reassigned["data"]["total"], 4);
    assert_eq!(
        reassigned["data"]["items"][0],
        json!({"repo":null,"total":3,"open":1,"in_progress":0,"done":2,"cancelled":0,"ready":1,"blocked":0})
    );
    assert_eq!(
        reassigned["data"]["items"][1],
        refreshed["data"]["items"][1]
    );
    assert_eq!(reassigned["data"]["items"][3]["repo"], "repo/a");
    let schema = success(&db, &["schema", "repos"], None)?;
    let output = jsonschema::validator_for(&schema["data"]["output"])?;
    for response in [
        &empty,
        &all,
        &page,
        &active,
        &active_page,
        &refreshed,
        &reassigned,
    ] {
        assert!(output.is_valid(response), "{response}");
    }
    Ok(())
}

#[test]
fn unassigned_queries_filter_before_pagination_and_keep_external_readiness() -> TestResult {
    let temp = tempfile::tempdir()?;
    let db = temp.path().join("unassigned.redb");
    success(&db, &["init"], None)?;
    for input in [
        json!({"title":"Goal","repos":["repo/goal"]}),
        json!({"title":"needle blocked","parent":1,"priority":"high"}),
        json!({"title":"needle named","parent":1,"priority":"high","repos":["repo/A"]}),
        json!({"title":"Discovery context","parent":1,"priority":"high"}),
        json!({"title":"External prerequisite","repos":["repo/external"]}),
        json!({"title":"needle completed","parent":1}),
        json!({"title":"needle outside goal"}),
        json!({"title":"needle named null","repos":["null"]}),
    ] {
        success(&db, &["add"], Some(input))?;
    }
    success(
        &db,
        &["update", "2"],
        Some(json!({"revision":1,"depends_on":[5]})),
    )?;
    for (id, status) in [("5", "cancelled"), ("6", "done")] {
        success(
            &db,
            &["update", id],
            Some(json!({"revision":1,"status":status})),
        )?;
    }
    success(
        &db,
        &["note", "4"],
        Some(json!({"body":"needle discovery"})),
    )?;
    let ids = |value: &Value| -> Result<Vec<u64>, &'static str> {
        value["data"]["items"]
            .as_array()
            .ok_or("missing ticket items")?
            .iter()
            .map(|item| item["id"].as_u64().ok_or("missing ticket ID"))
            .collect()
    };
    let active = success(&db, &["list", "--unassigned"], None)?;
    assert_eq!(ids(&active)?, vec![2, 4, 7]);
    assert_eq!(active["data"]["total"], 3);
    let page = success(
        &db,
        &[
            "list",
            "--unassigned",
            "--under",
            "1",
            "--limit",
            "1",
            "--offset",
            "1",
        ],
        None,
    )?;
    assert_eq!(ids(&page)?, vec![4]);
    assert_eq!(page["data"]["total"], 2);
    assert!(page["data"]["next_offset"].is_null());
    assert_eq!(
        ids(&success(
            &db,
            &["list", "--unassigned", "--under", "1", "--ready"],
            None
        )?)?,
        vec![4]
    );
    assert_eq!(
        ids(&success(&db, &["list", "--unassigned", "--all"], None)?)?,
        vec![2, 4, 6, 7]
    );
    let found = success(
        &db,
        &[
            "search",
            "needle",
            "--unassigned",
            "--under",
            "1",
            "--limit",
            "1",
            "--offset",
            "1",
        ],
        None,
    )?;
    assert_eq!(ids(&found)?, vec![4]);
    assert_eq!(found["data"]["total"], 3);
    assert_eq!(found["data"]["next_offset"], 2);
    assert_eq!(
        ids(&success(
            &db,
            &["search", "needle", "--unassigned", "--status", "done"],
            None
        )?)?,
        vec![6]
    );
    success(
        &db,
        &["update", "5"],
        Some(json!({"revision":2,"status":"done"})),
    )?;
    assert_eq!(
        ids(&success(
            &db,
            &["list", "--unassigned", "--under", "1", "--ready"],
            None
        )?)?,
        vec![2, 4]
    );
    success(
        &db,
        &["update", "2"],
        Some(json!({"revision":2,"repos":["repo/assigned"]})),
    )?;
    success(
        &db,
        &["update", "3"],
        Some(json!({"revision":1,"repos":[]})),
    )?;
    assert_eq!(
        ids(&success(
            &db,
            &["list", "--unassigned", "--under", "1"],
            None
        )?)?,
        vec![3, 4]
    );
    assert_eq!(
        ids(&success(
            &db,
            &["search", "needle", "--unassigned", "--under", "1"],
            None
        )?)?,
        vec![3, 4, 6]
    );
    Ok(())
}

#[test]
fn repository_and_unassigned_schemas_match_cli_before_storage_access() -> TestResult {
    let temp = tempfile::tempdir()?;
    let db = temp.path().join("missing/backlog.redb");
    let schema = success(&db, &["schema", "repos"], None)?;
    let input = jsonschema::validator_for(&schema["data"]["input"])?;
    for value in [
        json!({}),
        json!({"active":true,"limit":1}),
        json!({"active":false,"limit":500,"offset":usize::MAX}),
    ] {
        assert!(input.is_valid(&value), "{value}");
    }
    for limit in [0, 501] {
        assert!(!input.is_valid(&json!({"limit":limit})));
        failure(
            &db,
            &["repos", "--limit", &limit.to_string()],
            None,
            "invalid_input",
        )?;
    }
    for command in ["list", "search"] {
        let schema = success(&db, &["schema", command], None)?;
        let input = jsonschema::validator_for(&schema["data"]["input"])?;
        let mut value = if command == "search" {
            json!({"query":"needle"})
        } else {
            json!({})
        };
        value["unassigned"] = json!(true);
        assert!(input.is_valid(&value));
        value["repo"] = Value::Null;
        assert!(input.is_valid(&value));
        value["repo"] = json!("repo/A");
        assert!(!input.is_valid(&value));
        value["unassigned"] = json!(false);
        assert!(input.is_valid(&value));
        let mut args = vec![command];
        if command == "search" {
            args.push("needle");
        }
        for flags in [
            ["--unassigned", "--repo", "repo/A"],
            ["--repo", "repo/A", "--unassigned"],
        ] {
            let mut conflicting = args.clone();
            conflicting.extend(flags);
            failure(&db, &conflicting, None, "invalid_input")?;
        }
    }
    assert!(!db.parent().ok_or("missing fixture parent")?.exists());
    Ok(())
}

#[test]
fn goal_queries_follow_all_parent_levels_before_filtering_and_pagination() -> TestResult {
    let temp = tempfile::tempdir()?;
    let db = temp.path().join("goals.redb");
    success(&db, &["init"], None)?;
    for input in [
        json!({"title":"Leaf", "repos":["repo/A"], "priority":"high"}),
        json!({"title":"needle middle", "repos":["repo/A"]}),
        json!({"title":"Branch", "repos":["repo/B"]}),
        json!({"title":"needle external prerequisite"}),
        json!({"title":"needle goal", "repos":["repo/A"]}),
        json!({"title":"needle completed child", "parent":3, "repos":["repo/A"]}),
        json!({"title":"Cancelled child", "parent":5}),
        json!({"title":"needle outside dependent", "depends_on":[1]}),
    ] {
        success(&db, &["add"], Some(input))?;
    }
    for (id, input) in [
        ("1", json!({"revision":1,"parent":2,"depends_on":[4]})),
        ("2", json!({"revision":1,"parent":3,"status":"in_progress"})),
        ("3", json!({"revision":1,"parent":5})),
        ("5", json!({"revision":1,"status":"cancelled"})),
        ("6", json!({"revision":1,"status":"done"})),
        ("7", json!({"revision":1,"status":"cancelled"})),
    ] {
        success(&db, &["update", id], Some(input))?;
    }
    for _ in 0..2 {
        success(
            &db,
            &["note", "1"],
            Some(json!({"body":"needle discovery"})),
        )?;
    }
    let first = success(&db, &["list", "--under", "5", "--limit", "1"], None)?;
    assert_eq!(first["data"]["total"], 3);
    assert_eq!(first["data"]["items"][0]["id"], 1);
    assert_eq!(first["data"]["items"][0]["blocked_by"], json!([4]));
    assert_eq!(first["data"]["next_offset"], 1);
    let second = success(
        &db,
        &["list", "--under", "5", "--limit", "1", "--offset", "1"],
        None,
    )?;
    assert_eq!(second["data"]["total"], 3);
    assert_eq!(second["data"]["items"][0]["id"], 2);
    assert_eq!(second["data"]["next_offset"], 2);
    assert_eq!(
        success(&db, &["list", "--under", "5", "--all"], None)?["data"]["total"],
        5
    );
    assert_eq!(
        success(&db, &["list", "--under", "3"], None)?["data"]["total"],
        2
    );
    assert_eq!(
        success(&db, &["list", "--under", "1"], None)?["data"]["total"],
        0
    );
    let ready = success(&db, &["list", "--under", "5", "--ready"], None)?;
    assert_eq!(ready["data"]["total"], 1);
    assert_eq!(ready["data"]["items"][0]["id"], 3);
    success(
        &db,
        &["update", "4"],
        Some(json!({"revision":1,"status":"done"})),
    )?;
    let unblocked = success(&db, &["list", "--under", "5", "--ready"], None)?;
    assert_eq!(unblocked["data"]["total"], 2);
    assert_eq!(unblocked["data"]["items"][0]["id"], 1);
    assert_eq!(unblocked["data"]["items"][0]["blocked_by"], json!([]));
    let repo = success(&db, &["list", "--under", "5", "--repo", "repo/A"], None)?;
    assert_eq!(repo["data"]["total"], 2);
    let closed = success(
        &db,
        &["list", "--under", "5", "--status", "cancelled"],
        None,
    )?;
    assert_eq!(closed["data"]["total"], 1);
    assert_eq!(closed["data"]["items"][0]["id"], 7);
    let found = success(
        &db,
        &["search", "needle", "--under", "5", "--limit", "1"],
        None,
    )?;
    assert_eq!(found["data"]["total"], 3);
    assert_eq!(found["data"]["items"][0]["id"], 1);
    assert_eq!(found["data"]["next_offset"], 1);
    let completed = success(
        &db,
        &[
            "search", "needle", "--under", "5", "--repo", "repo/A", "--status", "done",
        ],
        None,
    )?;
    assert_eq!(completed["data"]["total"], 1);
    assert_eq!(completed["data"]["items"][0]["id"], 6);
    success(
        &db,
        &["update", "3"],
        Some(json!({"revision":2,"status":"done"})),
    )?;
    assert_eq!(
        success(&db, &["list", "--under", "5"], None)?["data"]["total"],
        2
    );
    success(
        &db,
        &["update", "2"],
        Some(json!({"revision":2,"clear_parent":true})),
    )?;
    assert_eq!(
        success(&db, &["list", "--under", "5", "--all"], None)?["data"]["total"],
        3
    );
    assert_eq!(
        success(&db, &["search", "needle", "--under", "5"], None)?["data"]["total"],
        1
    );
    for args in [
        vec!["list", "--under", "99"],
        vec!["search", "absent", "--under", "99"],
    ] {
        failure(&db, &args, None, "not_found")?;
    }
    Ok(())
}

#[test]
fn goal_query_ids_and_schemas_require_positive_ids() -> TestResult {
    let temp = tempfile::tempdir()?;
    let db = temp.path().join("absent.redb");
    for command in ["list", "search"] {
        let mut args = vec![command];
        if command == "search" {
            args.push("needle");
        }
        args.extend(["--under", "0"]);
        failure(&db, &args, None, "invalid_input")?;
        let schema = success(&db, &["schema", command], None)?;
        let validator = jsonschema::validator_for(&schema["data"]["input"])?;
        let mut input = json!({"query":"needle","under":1});
        if command == "list" {
            input
                .as_object_mut()
                .ok_or("missing object")?
                .remove("query");
        }
        assert!(validator.is_valid(&input));
        input["under"] = json!(0);
        assert!(!validator.is_valid(&input));
        input["under"] = Value::Null;
        assert!(validator.is_valid(&input));
        input
            .as_object_mut()
            .ok_or("missing object")?
            .remove("under");
        assert!(validator.is_valid(&input));
    }
    assert!(!db.exists());
    Ok(())
}

#[test]
fn blocker_queries_follow_active_prerequisites_across_repositories() -> TestResult {
    let temp = tempfile::tempdir()?;
    let db = temp.path().join("blockers.redb");
    success(&db, &["init"], None)?;
    for input in [
        json!({"title":"Goal", "repos":["repo/A"]}),
        json!({"title":"Direct B", "repos":["repo/B"]}),
        json!({"title":"Direct C", "repos":["repo/C"]}),
        json!({"title":"Shared leaf", "priority":"high", "repos":["repo/foundation"]}),
        json!({"title":"Completed prerequisite"}),
        json!({"title":"Hidden behind completion", "priority":"high"}),
        json!({"title":"Cancelled prerequisite"}),
        json!({"title":"Hidden behind cancellation"}),
        json!({"title":"Decomposition only", "parent":1}),
        json!({"title":"Outside consumer", "depends_on":[1]}),
        json!({"title":"Already in progress"}),
    ] {
        success(&db, &["add"], Some(input))?;
    }
    for (id, input) in [
        ("1", json!({"revision":1,"depends_on":[2,3,5,7,11]})),
        ("2", json!({"revision":1,"depends_on":[4]})),
        ("3", json!({"revision":1,"depends_on":[4]})),
        ("5", json!({"revision":1,"depends_on":[6],"status":"done"})),
        (
            "7",
            json!({"revision":1,"depends_on":[8],"status":"cancelled"}),
        ),
        ("11", json!({"revision":1,"status":"in_progress"})),
    ] {
        success(&db, &["update", id], Some(input))?;
    }
    let all = success(&db, &["blockers", "1"], None)?;
    let ids = all["data"]["items"]
        .as_array()
        .ok_or("missing blockers")?
        .iter()
        .map(|item| item["id"].clone())
        .collect::<Vec<_>>();
    assert_eq!(ids, vec![json!(4), json!(2), json!(3), json!(7), json!(11)]);
    assert_eq!(all["data"]["items"][0]["repos"], json!(["repo/foundation"]));
    assert_eq!(all["data"]["items"][1]["blocked_by"], json!([4]));
    assert_eq!(all["data"]["items"][3]["status"], "cancelled");
    assert_eq!(all["data"]["items"][3]["blocked_by"], json!([8]));
    let page = success(
        &db,
        &["blockers", "1", "--limit", "1", "--offset", "1"],
        None,
    )?;
    assert_eq!(page["data"]["total"], 5);
    assert_eq!(page["data"]["items"][0]["id"], 2);
    assert_eq!(page["data"]["next_offset"], 2);
    let exhausted = success(&db, &["blockers", "1", "--offset", "5"], None)?;
    assert_eq!(exhausted["data"]["total"], 5);
    assert_eq!(exhausted["data"]["items"], json!([]));
    assert!(exhausted["data"]["next_offset"].is_null());
    let ready = success(&db, &["blockers", "1", "--ready", "--limit", "1"], None)?;
    assert_eq!(ready["data"]["total"], 1);
    assert_eq!(ready["data"]["items"][0]["id"], 4);
    assert!(ready["data"]["next_offset"].is_null());
    assert_eq!(success(&db, &["blockers", "4"], None)?["data"]["total"], 0);
    success(
        &db,
        &["update", "2"],
        Some(json!({"revision":2,"depends_on":[4,6,8]})),
    )?;
    let alternate_paths = success(&db, &["blockers", "1", "--ready"], None)?;
    assert_eq!(alternate_paths["data"]["total"], 3);
    assert_eq!(alternate_paths["data"]["items"][0]["id"], 4);
    assert_eq!(alternate_paths["data"]["items"][1]["id"], 6);
    assert_eq!(alternate_paths["data"]["items"][2]["id"], 8);
    success(
        &db,
        &["update", "2"],
        Some(json!({"revision":3,"depends_on":[4]})),
    )?;
    success(
        &db,
        &["update", "1"],
        Some(json!({"revision":2,"status":"in_progress"})),
    )?;
    assert_eq!(success(&db, &["blockers", "1"], None)?["data"]["total"], 5);
    success(
        &db,
        &["update", "4"],
        Some(json!({"revision":1,"status":"done"})),
    )?;
    let unblocked = success(&db, &["blockers", "1", "--ready"], None)?;
    assert_eq!(unblocked["data"]["total"], 2);
    assert_eq!(unblocked["data"]["items"][0]["id"], 2);
    assert_eq!(unblocked["data"]["items"][1]["id"], 3);
    success(
        &db,
        &["update", "7"],
        Some(json!({"revision":2,"status":"open"})),
    )?;
    let reopened = success(&db, &["blockers", "1", "--ready"], None)?;
    assert_eq!(reopened["data"]["total"], 3);
    assert_eq!(reopened["data"]["items"][2]["id"], 8);
    success(
        &db,
        &["update", "1"],
        Some(json!({"revision":3,"depends_on":[5]})),
    )?;
    assert_eq!(success(&db, &["blockers", "1"], None)?["data"]["total"], 0);
    success(
        &db,
        &["update", "1"],
        Some(json!({"revision":4,"depends_on":[6],"status":"done"})),
    )?;
    assert_eq!(success(&db, &["blockers", "1"], None)?["data"]["total"], 0);
    success(
        &db,
        &["update", "1"],
        Some(json!({"revision":5,"status":"cancelled"})),
    )?;
    assert_eq!(success(&db, &["blockers", "1"], None)?["data"]["total"], 0);
    success(
        &db,
        &["update", "1"],
        Some(json!({"revision":6,"status":"open"})),
    )?;
    assert_eq!(
        success(&db, &["blockers", "1", "--ready"], None)?["data"]["items"][0]["id"],
        6
    );
    failure(&db, &["blockers", "99"], None, "not_found")?;
    let schema = success(&db, &["schema", "blockers"], None)?;
    assert!(jsonschema::validator_for(&schema["data"]["output"])?.is_valid(&all));
    Ok(())
}

#[test]
fn blocker_argument_schema_matches_cli_ranges_without_opening_storage() -> TestResult {
    let temp = tempfile::tempdir()?;
    let db = temp.path().join("absent.redb");
    for args in [
        vec!["blockers", "0"],
        vec!["blockers", "1", "--limit", "0"],
        vec!["blockers", "1", "--limit", "501"],
        vec!["blockers", "1", "--under", "2"],
    ] {
        failure(&db, &args, None, "invalid_input")?;
    }
    let schema = success(&db, &["schema", "blockers"], None)?;
    let validator = jsonschema::validator_for(&schema["data"]["input"])?;
    for input in [
        json!({"id":1}),
        json!({"id":1,"ready":true,"limit":500,"offset":1}),
    ] {
        assert!(validator.is_valid(&input));
    }
    for input in [
        json!({"id":0}),
        json!({"id":1,"limit":0}),
        json!({"id":1,"limit":501}),
        json!({"id":1,"repo":"repo/A"}),
    ] {
        assert!(!validator.is_valid(&input));
    }
    assert!(!db.exists());
    Ok(())
}

#[test]
fn impact_queries_trace_active_consumers_before_filtering_and_pagination() -> TestResult {
    let temp = tempfile::tempdir()?;
    let db = temp.path().join("impact.redb");
    success(&db, &["init"], None)?;
    for input in [
        json!({"title":"Shared foundation","repos":["repo/base"]}),
        json!({"title":"Direct consumer","repos":["repo/A"],"depends_on":[1],"priority":"low"}),
        json!({"title":"Indirect goal","repos":["repo/B"],"depends_on":[2],"priority":"high"}),
        json!({"title":"Second path","repos":["repo/C"],"depends_on":[1]}),
        json!({"title":"Shared consumer","repos":["repo/B"],"depends_on":[2,4],"priority":"high"}),
        json!({"title":"Finished branch","depends_on":[1]}),
        json!({"title":"Behind completion","depends_on":[6]}),
        json!({"title":"Cancelled branch","depends_on":[1]}),
        json!({"title":"Behind cancellation","depends_on":[8]}),
        json!({"title":"Decomposition only","parent":1}),
        json!({"title":"Root prerequisite"}),
        json!({"title":"Other prerequisite"}),
    ] {
        success(&db, &["add"], Some(input))?;
    }
    for (id, input) in [
        ("1", json!({"revision":1,"depends_on":[11]})),
        ("4", json!({"revision":1,"status":"in_progress"})),
        ("5", json!({"revision":1,"depends_on":[2,4,12]})),
        ("6", json!({"revision":1,"status":"done"})),
        ("8", json!({"revision":1,"status":"cancelled"})),
    ] {
        success(&db, &["update", id], Some(input))?;
    }
    let ids = |value: &Value| -> Result<Vec<Value>, &'static str> {
        Ok(value["data"]["items"]
            .as_array()
            .ok_or("missing impact items")?
            .iter()
            .map(|item| item["id"].clone())
            .collect())
    };
    let impact = success(&db, &["impact", "1"], None)?;
    assert_eq!(ids(&impact)?, vec![json!(3), json!(5), json!(4), json!(2)]);
    assert_eq!(impact["data"]["total"], 4);
    assert_eq!(impact["data"]["items"][1]["blocked_by"], json!([2, 4, 12]));
    assert_eq!(impact["data"]["items"][2]["status"], "in_progress");
    let page = success(&db, &["impact", "1", "--limit", "1", "--offset", "1"], None)?;
    assert_eq!(ids(&page)?, vec![json!(5)]);
    assert_eq!(page["data"]["total"], 4);
    assert_eq!(page["data"]["next_offset"], 2);
    let filtered = success(
        &db,
        &["impact", "1", "--repo", "repo/B", "--limit", "1"],
        None,
    )?;
    assert_eq!(ids(&filtered)?, vec![json!(3)]);
    assert_eq!(filtered["data"]["total"], 2);
    assert_eq!(filtered["data"]["next_offset"], 1);
    let maximum_offset = usize::MAX.to_string();
    for (args, total) in [
        (vec!["impact", "1", "--offset", "4"], 4),
        (vec!["impact", "1", "--offset", &maximum_offset], 4),
        (vec!["impact", "1", "--repo", "repo/b"], 0),
        (vec!["impact", "10"], 0),
    ] {
        let empty = success(&db, &args, None)?;
        assert_eq!(ids(&empty)?, Vec::<Value>::new());
        assert_eq!(empty["data"]["total"], total);
        assert!(empty["data"]["next_offset"].is_null());
    }
    success(
        &db,
        &["update", "1"],
        Some(json!({"revision":2,"status":"cancelled"})),
    )?;
    assert_eq!(ids(&success(&db, &["impact", "1"], None)?)?, ids(&impact)?);
    success(
        &db,
        &["update", "1"],
        Some(json!({"revision":3,"status":"done"})),
    )?;
    assert_eq!(success(&db, &["impact", "1"], None)?["data"]["total"], 0);
    success(
        &db,
        &["update", "1"],
        Some(json!({"revision":4,"status":"open"})),
    )?;
    success(
        &db,
        &["update", "2"],
        Some(json!({"revision":1,"status":"done"})),
    )?;
    assert_eq!(
        ids(&success(&db, &["impact", "1"], None)?)?,
        vec![json!(5), json!(4)]
    );
    success(
        &db,
        &["update", "4"],
        Some(json!({"revision":2,"status":"cancelled"})),
    )?;
    assert_eq!(success(&db, &["impact", "1"], None)?["data"]["total"], 0);
    success(
        &db,
        &["update", "8"],
        Some(json!({"revision":2,"status":"open"})),
    )?;
    assert_eq!(
        ids(&success(&db, &["impact", "1"], None)?)?,
        vec![json!(8), json!(9)]
    );
    success(
        &db,
        &["update", "7"],
        Some(json!({"revision":1,"depends_on":[6,8]})),
    )?;
    assert_eq!(
        ids(&success(&db, &["impact", "1"], None)?)?,
        vec![json!(7), json!(8), json!(9)]
    );
    failure(&db, &["impact", "99"], None, "not_found")?;
    let schema = success(&db, &["schema", "impact"], None)?;
    assert!(jsonschema::validator_for(&schema["data"]["output"])?.is_valid(&impact));
    Ok(())
}

#[test]
fn impact_argument_schema_matches_cli_without_opening_storage() -> TestResult {
    let temp = tempfile::tempdir()?;
    let db = temp.path().join("absent.redb");
    for args in [
        vec!["impact", "0"],
        vec!["impact", "1", "--limit", "0"],
        vec!["impact", "1", "--limit", "501"],
        vec!["impact", "1", "--ready"],
    ] {
        failure(&db, &args, None, "invalid_input")?;
    }
    let schema = success(&db, &["schema", "impact"], None)?;
    let validator = jsonschema::validator_for(&schema["data"]["input"])?;
    for input in [
        json!({"id":1}),
        json!({"id":1,"repo":"repo/A","limit":500,"offset":1}),
    ] {
        assert!(validator.is_valid(&input));
    }
    for input in [
        json!({"id":0}),
        json!({"id":1,"limit":0}),
        json!({"id":1,"limit":501}),
        json!({"id":1,"ready":true}),
    ] {
        assert!(!validator.is_valid(&input));
    }
    assert!(!db.exists());
    Ok(())
}

#[test]
fn generated_schemas_match_real_responses_and_failures() -> TestResult {
    let temp = tempfile::tempdir()?;
    let db = temp.path().join("db.redb");
    let schema = success(&db, &["schema", "add"], None)?;
    assert!(!db.exists());
    assert!(
        jsonschema::validator_for(&schema["data"]["input"])?.is_valid(&json!({"title":"Typed"}))
    );
    assert!(!jsonschema::validator_for(&schema["data"]["input"])?.is_valid(&json!({"title":7})));
    let init = success(&db, &["init"], None)?;
    let added = success(&db, &["add"], Some(json!({"title":"Typed"})))?;
    let updated = success(
        &db,
        &["update", "1"],
        Some(json!({"revision":1,"status":"in_progress"})),
    )?;
    let note = success(&db, &["note", "1"], Some(json!({"body":"Progress"})))?;
    let list = success(&db, &["list", "--status", "in_progress"], None)?;
    let show = success(&db, &["show", "1", "--context"], None)?;
    let search = success(&db, &["search", "Progress"], None)?;
    for (name, value) in [
        ("init", init),
        ("add", added),
        ("update", updated),
        ("note", note),
        ("list", list),
        ("show", show),
        ("search", search),
    ] {
        let schema = success(&db, &["schema", name], None)?;
        assert!(
            jsonschema::validator_for(&schema["data"]["output"])?.is_valid(&value),
            "{name}: {value}"
        );
    }
    let error = failure(
        &db,
        &["add"],
        Some(json!({"title":"bad", "depends_on":["wrong"]})),
        "invalid_input",
    )?;
    assert!(
        error["error"]["path"]
            .as_str()
            .is_some_and(|p| p.contains("depends_on"))
    );
    assert!(jsonschema::validator_for(&schema["data"]["error"])?.is_valid(&error));
    failure(&db, &["list", "--limit", "0"], None, "invalid_input")?;
    failure(
        &db,
        &["add"],
        Some(json!({"title":"bad","unknown":true})),
        "invalid_input",
    )?;
    Ok(())
}

#[test]
fn note_queries_recover_old_discoveries_before_paginating() -> TestResult {
    let temp = tempfile::tempdir()?;
    let db = temp.path().join("backlog.redb");
    success(&db, &["init"], None)?;
    success(
        &db,
        &["add"],
        Some(json!({"title":"Adopt shared checks","body":"Purpose stays visible"})),
    )?;
    success(&db, &["add"], Some(json!({"title":"Another repository"})))?;
    let query = "キャッシュ[共有].*";
    for (ticket, body) in [
        (
            "1",
            format!("Reusable discovery: {query} can help every repository"),
        ),
        ("2", format!("Another ticket has {query}")),
        ("1", format!("Reusable follow-up: {query} needs an adapter")),
    ] {
        success(&db, &["note", ticket], Some(json!({"body":body})))?;
    }
    for index in 0..21 {
        success(
            &db,
            &["note", "1"],
            Some(json!({"body":format!("Unrelated progress {index}")})),
        )?;
    }
    let unfiltered = success(&db, &["show", "1", "--context"], None)?;
    assert_eq!(unfiltered["data"]["notes"]["total"], 23);
    assert!(
        unfiltered["data"]["notes"]["items"]
            .as_array()
            .ok_or("missing notes")?
            .iter()
            .all(|n| n["id"].as_u64().is_some_and(|id| id > 3))
    );
    assert_eq!(success(&db, &["search", query], None)?["data"]["total"], 2);
    let filtered = success(
        &db,
        &[
            "show",
            "1",
            "--context",
            "--notes-query",
            query,
            "--notes-limit",
            "1",
        ],
        None,
    )?;
    assert_eq!(filtered["data"]["ticket"], unfiltered["data"]["ticket"]);
    assert_eq!(filtered["data"]["context"], unfiltered["data"]["context"]);
    assert_eq!(filtered["data"]["notes"]["total"], 2);
    assert_eq!(filtered["data"]["notes"]["items"][0]["id"], 3);
    assert_eq!(filtered["data"]["notes"]["next_offset"], 1);
    let older = success(
        &db,
        &["show", "1", "--notes-query", query, "--notes-offset", "1"],
        None,
    )?;
    assert_eq!(older["data"]["notes"]["total"], 2);
    assert_eq!(older["data"]["notes"]["items"][0]["id"], 1);
    assert_eq!(
        older["data"]["notes"]["items"][0]["body"],
        format!("Reusable discovery: {query} can help every repository")
    );
    assert!(older["data"]["notes"]["next_offset"].is_null());
    let exhausted = success(
        &db,
        &["show", "1", "--notes-query", query, "--notes-offset", "2"],
        None,
    )?;
    assert_eq!(exhausted["data"]["notes"]["items"], json!([]));
    assert_eq!(exhausted["data"]["notes"]["total"], 2);
    assert!(exhausted["data"]["notes"]["next_offset"].is_null());
    let wrong_case = success(&db, &["show", "1", "--notes-query", "reusable"], None)?;
    assert_eq!(wrong_case["data"]["notes"]["total"], 0);
    let regex = success(&db, &["show", "1", "--notes-query", "キャッシュ.*"], None)?;
    assert_eq!(regex["data"]["notes"]["items"], json!([]));
    success(
        &db,
        &["update", "1"],
        Some(json!({"revision":1,"status":"done"})),
    )?;
    assert_eq!(
        success(&db, &["show", "1", "--notes-query", query], None)?["data"]["notes"]["total"],
        2
    );
    let schema = success(&db, &["schema", "show"], None)?;
    assert!(jsonschema::validator_for(&schema["data"]["output"])?.is_valid(&filtered));
    Ok(())
}

#[test]
fn note_query_validation_and_schema_require_nonempty_text() -> TestResult {
    let temp = tempfile::tempdir()?;
    let db = temp.path().join("absent.redb");
    for query in ["", " \n\t", "\u{3000}"] {
        let error = failure(
            &db,
            &["show", "1", "--notes-query", query],
            None,
            "invalid_input",
        )?;
        assert_eq!(error["error"]["path"], "notes_query");
    }
    assert!(!db.exists());
    let schema = success(&db, &["schema", "show"], None)?;
    let input = jsonschema::validator_for(&schema["data"]["input"])?;
    assert!(input.is_valid(&json!({"id":1})));
    assert!(input.is_valid(&json!({"id":1,"notes_query":null})));
    assert!(input.is_valid(&json!({"id":1,"notes_query":"共通化"})));
    assert!(!input.is_valid(&json!({"id":1,"notes_query":""})));
    Ok(())
}

#[test]
fn lock_conflicts_are_bounded_and_released() -> TestResult {
    let temp = tempfile::tempdir()?;
    let db = temp.path().join("db.redb");
    success(&db, &["init"], None)?;
    let held = redb::Database::open(&db)?;
    let error = failure(
        &db,
        &["--lock-timeout-ms", "0", "list"],
        None,
        "database_busy",
    )?;
    assert_eq!(error["error"]["retryable"], true);
    drop(held);
    success(&db, &["list"], None)?;
    Ok(())
}

#[test]
fn queries_share_readers_and_preserve_healthy_database_bytes() -> TestResult {
    let temp = tempfile::tempdir()?;
    let db = temp.path().join("readable.redb");
    success(&db, &["init"], None)?;
    success(
        &db,
        &["add"],
        Some(json!({"title":"Read shared context","body":"Readable snapshot"})),
    )?;
    success(
        &db,
        &["note", "1"],
        Some(json!({"body":"Existing discovery"})),
    )?;
    let before = std::fs::read(&db)?;
    let reader = redb::ReadOnlyDatabase::open(&db)?;
    assert_eq!(
        success(&db, &["--lock-timeout-ms", "0", "list"], None)?["data"]["total"],
        1
    );
    assert_eq!(
        success(
            &db,
            &["--lock-timeout-ms", "0", "show", "1", "--context"],
            None
        )?["data"]["ticket"]["body"],
        "Readable snapshot"
    );
    assert_eq!(
        success(
            &db,
            &["--lock-timeout-ms", "0", "search", "discovery"],
            None
        )?["data"]["items"][0]["id"],
        1
    );
    assert_eq!(
        success(&db, &["--lock-timeout-ms", "0", "blockers", "1"], None)?["data"]["total"],
        0
    );
    assert_eq!(
        success(&db, &["--lock-timeout-ms", "0", "impact", "1"], None)?["data"]["total"],
        0
    );
    let repos = success(&db, &["--lock-timeout-ms", "0", "repos"], None)?;
    assert_eq!(repos["data"]["total"], 1);
    assert_eq!(
        repos["data"]["items"][0],
        json!({"repo":null,"total":1,"open":1,"in_progress":0,"done":0,"cancelled":0,"ready":1,"blocked":0})
    );
    let goals = success(&db, &["--lock-timeout-ms", "0", "goals"], None)?;
    assert_eq!(goals["data"]["total"], 1);
    assert_eq!(goals["data"]["items"][0]["goal"]["id"], 1);
    assert_eq!(
        goals["data"]["items"][0]["work"],
        json!({"total":1,"open":1,"in_progress":0,"done":0,"cancelled":0,"ready":1,"blocked":0})
    );
    let notes = success(&db, &["--lock-timeout-ms", "0", "notes"], None)?;
    assert_eq!(notes["data"]["total"], 1);
    assert_eq!(
        notes["data"]["items"][0]["note"]["body"],
        "Existing discovery"
    );
    assert_eq!(
        notes["data"]["items"][0]["ticket"]["title"],
        "Read shared context"
    );
    assert_eq!(notes["data"]["items"][0]["ticket"]["blocked_by"], json!([]));
    for args in [
        &["--lock-timeout-ms", "0", "bottlenecks"][..],
        &["--lock-timeout-ms", "0", "bottlenecks", "--ready"][..],
        &["--lock-timeout-ms", "0", "bottlenecks", "--for-goal", "1"][..],
        &[
            "--lock-timeout-ms",
            "0",
            "bottlenecks",
            "--for-goal",
            "1",
            "--ready",
        ][..],
    ] {
        assert_eq!(
            success(&db, args, None)?["data"],
            json!({"items":[],"total":0,"next_offset":null})
        );
    }
    for args in [
        &["--lock-timeout-ms", "0", "repos", "--active"][..],
        &["--lock-timeout-ms", "0", "goals", "--active"][..],
        &["--lock-timeout-ms", "0", "list", "--unassigned"][..],
        &[
            "--lock-timeout-ms",
            "0",
            "search",
            "discovery",
            "--unassigned",
        ][..],
    ] {
        assert_eq!(success(&db, args, None)?["data"]["total"], 1);
    }
    failure(
        &db,
        &["--lock-timeout-ms", "0", "add"],
        Some(json!({"title":"Reader excludes writers"})),
        "database_busy",
    )?;
    drop(reader);
    assert_eq!(std::fs::read(&db)?, before);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&db, std::fs::Permissions::from_mode(0o400))?;
        for args in [
            &["list"][..],
            &["show", "1"][..],
            &["search", "discovery"][..],
            &["blockers", "1"][..],
            &["impact", "1"][..],
            &["repos"][..],
            &["repos", "--active"][..],
            &["goals"][..],
            &["goals", "--active"][..],
            &["notes"][..],
            &["bottlenecks"][..],
            &["bottlenecks", "--ready"][..],
            &["bottlenecks", "--for-goal", "1"][..],
            &["bottlenecks", "--for-goal", "1", "--ready"][..],
            &["list", "--unassigned"][..],
            &["search", "discovery", "--unassigned"][..],
        ] {
            success(&db, args, None)?;
            assert_eq!(std::fs::read(&db)?, before);
        }
    }
    Ok(())
}

#[test]
fn query_validation_errors_never_require_writes_or_modify_clean_sources() -> TestResult {
    let temp = tempfile::tempdir()?;
    for (name, code) in [
        ("foreign", "corrupt_data"),
        ("unsupported", "unsupported_version"),
        ("corrupt", "corrupt_data"),
    ] {
        let db = temp.path().join(format!("{name}.redb"));
        if name != "foreign" {
            success(&db, &["init"], None)?;
            success(&db, &["add"], Some(json!({"title":"Keep source"})))?;
        }
        let native = redb::Database::create(&db)?;
        let transaction = native.begin_write()?;
        match name {
            "foreign" => {
                transaction
                    .open_table(redb::TableDefinition::<&str, &str>::new("foreign"))?
                    .insert("owner", "keep")?;
            }
            "unsupported" => {
                transaction
                    .open_table(redb::TableDefinition::<&str, u64>::new("meta"))?
                    .insert("schema_version", 99)?;
            }
            _ => {
                transaction
                    .open_table(redb::TableDefinition::<u64, &str>::new("tickets"))?
                    .insert(1, "{}")?;
            }
        }
        transaction.commit()?;
        drop(native);
        let before = std::fs::read(&db)?;
        let reader = redb::ReadOnlyDatabase::open(&db)?;
        for args in [
            &["--lock-timeout-ms", "0", "list"][..],
            &["--lock-timeout-ms", "0", "show", "1"][..],
            &["--lock-timeout-ms", "0", "search", "keep"][..],
            &["--lock-timeout-ms", "0", "blockers", "1"][..],
            &["--lock-timeout-ms", "0", "impact", "1"][..],
            &["--lock-timeout-ms", "0", "repos"][..],
            &["--lock-timeout-ms", "0", "goals"][..],
            &["--lock-timeout-ms", "0", "notes"][..],
            &["--lock-timeout-ms", "0", "bottlenecks"][..],
            &["--lock-timeout-ms", "0", "bottlenecks", "--for-goal", "1"][..],
            &["--lock-timeout-ms", "0", "list", "--unassigned"][..],
            &["--lock-timeout-ms", "0", "search", "keep", "--unassigned"][..],
        ] {
            failure(&db, args, None, code)?;
            assert_eq!(std::fs::read(&db)?, before);
        }
        drop(reader);
    }
    Ok(())
}

#[test]
fn archives_restore_relationships_revisions_sequences_and_replay_records() -> TestResult {
    let temp = tempfile::tempdir()?;
    let source = temp.path().join("source.redb");
    let archive = temp.path().join("archive.redb");
    let restored = temp.path().join("recovered/backlog.redb");
    let archive_path = archive.to_str().ok_or("non-UTF-8 fixture path")?;
    let source_path = source.to_str().ok_or("non-UTF-8 fixture path")?;
    success(&source, &["init"], None)?;
    let capture = json!({"title":"Shared goal","request_id":"capture-goal"});
    let original = success(&source, &["add"], Some(capture.clone()))?;
    success(&source, &["add"], Some(json!({"title":"Prerequisite"})))?;
    success(
        &source,
        &["add"],
        Some(json!({"title":"Consumer","parent":1,"depends_on":[2],"repos":["owner/app"]})),
    )?;
    success(
        &source,
        &["update", "1"],
        Some(json!({"revision":1,"body":"Preserve this context","status":"in_progress"})),
    )?;
    let note_input =
        json!({"body":"Resume after checking the dependency","request_id":"capture-note"});
    let note = success(&source, &["note", "3"], Some(note_input.clone()))?;
    let expected = success(&source, &["show", "3", "--context"], None)?;
    let source_bytes = std::fs::read(&source)?;
    let saved = success(&source, &["backup", archive_path], None)?;
    assert_eq!(std::fs::read(&source)?, source_bytes);
    assert_eq!(
        saved["data"],
        json!({"schema_version":1,"tickets":3,"notes":1,"requests":2})
    );
    for command in ["backup", "restore"] {
        let schema = success(&source, &["schema", command], None)?;
        assert!(jsonschema::validator_for(&schema["data"]["output"])?.is_valid(&saved));
    }
    success(&source, &["add"], Some(json!({"title":"After the backup"})))?;
    let archive_bytes = std::fs::read(&archive)?;
    let recovered = success(&restored, &["restore", archive_path], None)?;
    assert_eq!(recovered, saved);
    assert_eq!(std::fs::read(&archive)?, archive_bytes);
    assert_eq!(
        success(&restored, &["show", "3", "--context"], None)?,
        expected
    );
    assert_eq!(success(&restored, &["add"], Some(capture))?, original);
    assert_eq!(success(&restored, &["note", "3"], Some(note_input))?, note);
    assert_eq!(
        success(&restored, &["list", "--all"], None)?["data"]["total"],
        3
    );
    assert_eq!(
        success(&restored, &["add"], Some(json!({"title":"Next ticket"})))?["data"]["id"],
        4
    );
    assert_eq!(
        success(&restored, &["note", "3"], Some(json!({"body":"Next note"})))?["data"]["id"],
        2
    );
    assert_eq!(
        success(
            &restored,
            &["update", "1"],
            Some(json!({"revision":2,"status":"done"}))
        )?["data"]["revision"],
        3
    );
    failure(
        &source,
        &["backup", source_path],
        None,
        "destination_exists",
    )?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&archive)?.permissions().mode() & 0o777,
            0o600
        );
    }
    Ok(())
}

#[test]
fn archives_preserve_existing_destinations_and_refuse_invalid_sources() -> TestResult {
    let temp = tempfile::tempdir()?;
    let source = temp.path().join("source.redb");
    let destination = temp.path().join("destination.redb");
    let destination_path = destination.to_str().ok_or("non-UTF-8 fixture path")?;
    success(&source, &["init"], None)?;
    success(&source, &["add"], Some(json!({"title":"Source"})))?;
    for bytes in [b"owned data".as_slice(), b"".as_slice()] {
        std::fs::write(&destination, bytes)?;
        failure(
            &source,
            &["backup", destination_path],
            None,
            "destination_exists",
        )?;
        failure(
            &destination,
            &["restore", source.to_str().ok_or("fixture path")?],
            None,
            "destination_exists",
        )?;
        assert_eq!(std::fs::read(&destination)?, bytes);
    }
    let absent = temp.path().join("absent.redb");
    let unpublished = temp.path().join("new/invalid.redb");
    let unpublished_path = unpublished.to_str().ok_or("fixture path")?;
    failure(
        &absent,
        &["backup", unpublished_path],
        None,
        "not_initialized",
    )?;
    std::fs::write(&absent, b"foreign file")?;
    failure(&absent, &["backup", unpublished_path], None, "storage")?;
    assert!(!unpublished.exists());
    assert!(!temp.path().join("new").exists());
    assert_eq!(std::fs::read(&absent)?, b"foreign file");
    let held = redb::Database::open(&source)?;
    failure(
        &source,
        &["--lock-timeout-ms", "0", "backup", unpublished_path],
        None,
        "database_busy",
    )?;
    drop(held);
    assert!(!unpublished.exists());
    #[cfg(unix)]
    {
        let alias = temp.path().join("alias.redb");
        std::os::unix::fs::symlink("missing-target.redb", &alias)?;
        failure(
            &source,
            &["backup", alias.to_str().ok_or("fixture path")?],
            None,
            "destination_exists",
        )?;
        assert_eq!(
            std::fs::read_link(&alias)?,
            Path::new("missing-target.redb")
        );
        assert!(!temp.path().join("missing-target.redb").exists());
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o400))?;
        success(
            &source,
            &[
                "backup",
                temp.path()
                    .join("read-only.redb")
                    .to_str()
                    .ok_or("fixture path")?,
            ],
            None,
        )?;
    }
    Ok(())
}

#[test]
fn competing_archive_publishers_never_replace_the_winning_snapshot() -> TestResult {
    let temp = tempfile::tempdir()?;
    let destination = temp.path().join("new/archive/winner.redb");
    let mut children = Vec::new();
    for title in ["Left", "Right"] {
        let source = temp.path().join(format!("{title}.redb"));
        success(&source, &["init"], None)?;
        success(&source, &["add"], Some(json!({"title":title})))?;
        children.push(spawn_input(
            &source,
            &["backup", destination.to_str().ok_or("fixture path")?],
            None,
        )?);
    }
    let left = finish(children.remove(0))?;
    let right = finish(children.remove(0))?;
    assert_ne!(left.0, right.0);
    let error = if left.0 { right.1 } else { left.1 };
    assert_eq!(error["error"]["code"], "destination_exists");
    let detail = success(&destination, &["show", "1"], None)?;
    assert_eq!(
        detail["data"]["ticket"]["title"],
        if left.0 { "Left" } else { "Right" }
    );
    assert_eq!(success(&destination, &["list"], None)?["data"]["total"], 1);
    Ok(())
}

#[test]
fn archives_require_a_valid_supported_snapshot_before_creating_a_destination() -> TestResult {
    let temp = tempfile::tempdir()?;
    let source = temp.path().join("source.redb");
    let destination = temp.path().join("new/destination.redb");
    let destination_path = destination.to_str().ok_or("fixture path")?;
    success(&source, &["init"], None)?;
    success(&source, &["add"], Some(json!({"title":"Source"})))?;
    for (version, code) in [(99, "unsupported_version"), (1, "corrupt_data")] {
        let native = redb::Database::open(&source)?;
        let transaction = native.begin_write()?;
        transaction
            .open_table(redb::TableDefinition::<&str, u64>::new("meta"))?
            .insert("schema_version", version)?;
        if version == 1 {
            transaction
                .open_table(redb::TableDefinition::<u64, &str>::new("tickets"))?
                .insert(1, "{\"id\":1}")?;
        }
        transaction.commit()?;
        drop(native);
        let before = std::fs::read(&source)?;
        failure(&source, &["backup", destination_path], None, code)?;
        failure(
            &destination,
            &["restore", source.to_str().ok_or("fixture path")?],
            None,
            code,
        )?;
        assert_eq!(std::fs::read(&source)?, before);
        assert!(!destination.exists());
        assert!(!temp.path().join("new").exists());
    }
    Ok(())
}

#[test]
fn schemas_preserve_cli_ranges_and_argument_conflicts() -> TestResult {
    let temp = tempfile::tempdir()?;
    let db = temp.path().join("db.redb");
    for (command, field, base) in [
        ("list", "limit", json!({})),
        ("repos", "limit", json!({})),
        ("goals", "limit", json!({})),
        ("bottlenecks", "limit", json!({})),
        ("search", "limit", json!({"query":"work"})),
        ("show", "notes_limit", json!({"id":1})),
    ] {
        let schema = success(&db, &["schema", command], None)?;
        let validator = jsonschema::validator_for(&schema["data"]["input"])?;
        for (limit, valid) in [(0, false), (1, true), (500, true), (501, false)] {
            let mut input = base.clone();
            input[field] = json!(limit);
            assert_eq!(validator.is_valid(&input), valid, "{command}: {input}");
            if !valid {
                let limit = limit.to_string();
                let args = match command {
                    "search" => vec!["search", "work", "--limit", &limit],
                    "show" => vec!["show", "1", "--notes-limit", &limit],
                    "repos" => vec!["repos", "--limit", &limit],
                    "goals" => vec!["goals", "--limit", &limit],
                    "bottlenecks" => vec!["bottlenecks", "--limit", &limit],
                    _ => vec!["list", "--limit", &limit],
                };
                failure(&db, &args, None, "invalid_input")?;
            }
        }
    }
    let schema = success(&db, &["schema", "list"], None)?;
    let validator = jsonschema::validator_for(&schema["data"]["input"])?;
    assert!(!validator.is_valid(&json!({"all":true,"status":"done"})));
    assert!(validator.is_valid(&json!({"all":false,"status":"done"})));
    assert!(validator.is_valid(&json!({"all":true})));
    assert!(validator.is_valid(&json!({"all":true,"status":null})));
    failure(
        &db,
        &["list", "--all", "--status", "done"],
        None,
        "invalid_input",
    )?;
    let schema = success(&db, &["schema", "show"], None)?;
    assert!(!jsonschema::validator_for(&schema["data"]["input"])?.is_valid(&json!({"id":0})));
    failure(&db, &["show", "0"], None, "invalid_input")?;
    assert!(!db.exists());
    Ok(())
}

#[test]
fn schemas_preserve_positive_ids_and_update_parent_exclusivity() -> TestResult {
    let temp = tempfile::tempdir()?;
    let db = temp.path().join("db.redb");
    for (command, base) in [
        ("add", json!({"title":"Work"})),
        ("update", json!({"revision":1})),
    ] {
        let schema = success(&db, &["schema", command], None)?;
        let validator = jsonschema::validator_for(&schema["data"]["input"])?;
        for (field, value) in [("parent", json!(0)), ("depends_on", json!([0]))] {
            let mut input = base.clone();
            input[field] = value;
            assert!(!validator.is_valid(&input), "{command}: {input}");
        }
        for (field, value) in [("parent", json!(1)), ("depends_on", json!([1]))] {
            let mut input = base.clone();
            input[field] = value;
            assert!(validator.is_valid(&input), "{command}: {input}");
        }
    }
    let schema = success(&db, &["schema", "update"], None)?;
    let validator = jsonschema::validator_for(&schema["data"]["input"])?;
    assert!(!validator.is_valid(&json!({"revision":0})));
    assert!(!validator.is_valid(&json!({"revision":1,"parent":1,"clear_parent":true})));
    assert!(validator.is_valid(&json!({"revision":1,"parent":null,"clear_parent":true})));
    assert!(validator.is_valid(&json!({"revision":1,"parent":1,"clear_parent":false})));
    Ok(())
}

#[test]
fn unknown_files_are_preserved_and_reads_do_not_initialize() -> TestResult {
    let temp = tempfile::tempdir()?;
    let db = temp.path().join("db.redb");
    failure(&db, &["list"], None, "not_initialized")?;
    failure(&db, &["repos"], None, "not_initialized")?;
    failure(&db, &["goals"], None, "not_initialized")?;
    failure(&db, &["bottlenecks"], None, "not_initialized")?;
    assert!(!db.exists());
    std::fs::write(&db, b"user-owned data")?;
    failure(&db, &["init"], None, "storage")?;
    assert_eq!(std::fs::read(&db)?, b"user-owned data");
    Ok(())
}

#[test]
fn missing_storage_tables_are_refused_without_reinitializing() -> TestResult {
    let temp = tempfile::tempdir()?;
    let db = temp.path().join("db.redb");
    success(&db, &["init"], None)?;
    success(&db, &["add"], Some(json!({"title":"Keep history"})))?;
    let native = redb::Database::open(&db)?;
    let transaction = native.begin_write()?;
    transaction.delete_table(redb::TableDefinition::<u64, &str>::new("tickets"))?;
    transaction.commit()?;
    drop(native);
    failure(
        &db,
        &["add"],
        Some(json!({"title":"Do not recreate storage"})),
        "corrupt_data",
    )?;
    failure(&db, &["init"], None, "corrupt_data")?;
    Ok(())
}

#[test]
fn foreign_multimap_tables_are_refused_without_initializing_or_mutating() -> TestResult {
    use redb::{ReadableDatabase, ReadableTableMetadata};
    let temp = tempfile::tempdir()?;
    let foreign = redb::MultimapTableDefinition::<&str, &str>::new("foreign-data");
    for initialized in [false, true] {
        let db = temp.path().join(if initialized {
            "backlog.redb"
        } else {
            "foreign.redb"
        });
        if initialized {
            success(&db, &["init"], None)?;
            success(&db, &["add"], Some(json!({"title":"Keep existing work"})))?;
        }
        let native = redb::Database::create(&db)?;
        let transaction = native.begin_write()?;
        transaction
            .open_multimap_table(foreign)?
            .insert("owner", "original")?;
        transaction.commit()?;
        drop(native);
        failure(&db, &["init"], None, "corrupt_data")?;
        if initialized {
            failure(&db, &["list"], None, "corrupt_data")?;
            failure(
                &db,
                &["add"],
                Some(json!({"title":"Do not write"})),
                "corrupt_data",
            )?;
        }
        let native = redb::Database::open(&db)?;
        let transaction = native.begin_read()?;
        let table = transaction.open_multimap_table(foreign)?;
        let mut values = table.get("owner")?;
        assert_eq!(
            values
                .next()
                .transpose()?
                .ok_or("missing original value")?
                .value(),
            "original"
        );
        assert!(values.next().is_none());
        if initialized {
            let tickets =
                transaction.open_table(redb::TableDefinition::<u64, &str>::new("tickets"))?;
            assert_eq!(tickets.len()?, 1);
        } else {
            assert_eq!(transaction.list_tables()?.count(), 0);
        }
    }
    Ok(())
}

#[test]
fn concurrent_writers_preserve_revisions_and_replay_identity() -> TestResult {
    let temp = tempfile::tempdir()?;
    let db = temp.path().join("db.redb");
    success(&db, &["init"], None)?;
    success(&db, &["add"], Some(json!({"title":"Concurrent"})))?;
    let held = redb::Database::open(&db)?;
    let left = spawn_input(
        &db,
        &["update", "1"],
        Some(json!({"revision":1,"body":"left"})),
    )?;
    let right = spawn_input(
        &db,
        &["update", "1"],
        Some(json!({"revision":1,"body":"right"})),
    )?;
    drop(held);
    let left = finish(left)?;
    let right = finish(right)?;
    assert_eq!([left.0, right.0].into_iter().filter(|ok| *ok).count(), 1);
    let rejected = if left.0 { &right.1 } else { &left.1 };
    assert_eq!(rejected["error"]["code"], "conflict");
    assert_eq!(
        success(&db, &["show", "1"], None)?["data"]["ticket"]["revision"],
        2
    );
    let input = json!({"title":"Once", "request_id":"parallel-capture"});
    let left = spawn_input(&db, &["add"], Some(input.clone()))?;
    let right = spawn_input(&db, &["add"], Some(input))?;
    let left = finish(left)?;
    let right = finish(right)?;
    assert!(left.0 && right.0);
    assert_eq!(left.1, right.1);
    assert_eq!(success(&db, &["list"], None)?["data"]["total"], 2);
    Ok(())
}

#[test]
fn reserved_table_names_do_not_hide_incompatible_storage_types() -> TestResult {
    use redb::{ReadableDatabase, ReadableTableMetadata};
    let temp = tempfile::tempdir()?;
    for multimap in [false, true] {
        let db = temp.path().join(if multimap {
            "multimap.redb"
        } else {
            "wrong-value.redb"
        });
        success(&db, &["init"], None)?;
        success(&db, &["add"], Some(json!({"title":"Preserve work"})))?;
        let native = redb::Database::open(&db)?;
        let transaction = native.begin_write()?;
        transaction.delete_table(redb::TableDefinition::<&str, &str>::new("requests"))?;
        if multimap {
            transaction
                .open_multimap_table(redb::MultimapTableDefinition::<&str, u64>::new("requests"))?
                .insert("foreign", 42)?;
        } else {
            transaction
                .open_table(redb::TableDefinition::<&str, u64>::new("requests"))?
                .insert("foreign", 42)?;
        }
        transaction.commit()?;
        drop(native);
        for args in [&["init"][..], &["list"][..]] {
            failure(&db, args, None, "corrupt_data")?;
        }
        failure(
            &db,
            &["add"],
            Some(json!({"title":"Do not write"})),
            "corrupt_data",
        )?;
        let native = redb::Database::open(&db)?;
        let transaction = native.begin_read()?;
        assert_eq!(
            transaction
                .open_table(redb::TableDefinition::<u64, &str>::new("tickets"))?
                .len()?,
            1
        );
        if multimap {
            let table = transaction
                .open_multimap_table(redb::MultimapTableDefinition::<&str, u64>::new("requests"))?;
            assert_eq!(
                table
                    .get("foreign")?
                    .next()
                    .transpose()?
                    .ok_or("missing foreign value")?
                    .value(),
                42
            );
        } else {
            let table =
                transaction.open_table(redb::TableDefinition::<&str, u64>::new("requests"))?;
            assert_eq!(
                table
                    .get("foreign")?
                    .ok_or("missing foreign value")?
                    .value(),
                42
            );
        }
    }
    Ok(())
}

#[test]
fn crash_writer_child() -> TestResult {
    let Some(path) = std::env::var_os("WMBD_CRASH_TEST_DB") else {
        return Ok(());
    };
    let database = redb::Database::open(path)?;
    let transaction = database.begin_write()?;
    transaction
        .open_table(redb::TableDefinition::<u64, &str>::new("tickets"))?
        .insert(1, "uncommitted corrupt record")?;
    println!("transaction-open");
    std::io::stdout().flush()?;
    std::io::stdin().read_exact(&mut [0_u8; 1])?;
    std::process::exit(23);
}

#[test]
fn abrupt_exit_rolls_back_writes_and_releases_the_process_lock() -> TestResult {
    let temp = tempfile::tempdir()?;
    let db = temp.path().join("db.redb");
    success(&db, &["init"], None)?;
    success(&db, &["add"], Some(json!({"title":"Committed"})))?;
    let mut child = Command::new(std::env::current_exe()?)
        .args(["--exact", "crash_writer_child", "--nocapture"])
        .env("WMBD_CRASH_TEST_DB", &db)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;
    let mut output = std::io::BufReader::new(child.stdout.take().ok_or("missing child stdout")?);
    loop {
        let mut line = String::new();
        if output.read_line(&mut line)? == 0 {
            return Err("child exited before starting its transaction".into());
        }
        if line.trim() == "transaction-open" {
            break;
        }
    }
    child
        .stdin
        .take()
        .ok_or("missing child stdin")?
        .write_all(b"x")?;
    assert_eq!(child.wait()?.code(), Some(23));
    assert!(matches!(
        redb::ReadOnlyDatabase::open(&db),
        Err(redb::DatabaseError::RepairAborted)
    ));
    let dirty_bytes = std::fs::read(&db)?;
    let archive = temp.path().join("new/archive.redb");
    let archive_path = archive.to_str().ok_or("fixture path")?;
    let error = failure(&db, &["backup", archive_path], None, "recovery_required")?;
    assert!(
        error["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("list"))
    );
    let restored = temp.path().join("recovered/backlog.redb");
    failure(
        &restored,
        &["restore", db.to_str().ok_or("fixture path")?],
        None,
        "recovery_required",
    )?;
    assert_eq!(std::fs::read(&db)?, dirty_bytes);
    assert!(!temp.path().join("new").exists());
    assert!(!temp.path().join("recovered").exists());
    assert_eq!(
        success(&db, &["show", "1"], None)?["data"]["ticket"]["title"],
        "Committed"
    );
    success(&db, &["add"], Some(json!({"title":"After recovery"})))?;
    Ok(())
}

#[cfg(unix)]
#[test]
fn inaccessible_paths_are_not_reported_as_uninitialized() -> TestResult {
    let temp = tempfile::tempdir()?;
    let db = temp.path().join("loop.redb");
    std::os::unix::fs::symlink("loop.redb", &db)?;
    failure(&db, &["list"], None, "storage")?;
    Ok(())
}
