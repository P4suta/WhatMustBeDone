use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Output, Stdio};
use wmbd_core::{CounterexampleEvidence, ProofCheckKind, ProofCheckResult};

mod audit;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    kani_version: String,
    proof_target: String,
    audit_version: String,
    advisory_revision: String,
    gitleaks_version: String,
    harnesses: Vec<String>,
}

#[derive(Deserialize, Serialize)]
struct Receipt {
    source_sha256: String,
    kani_version: String,
    harnesses: Vec<String>,
    results: Value,
}

#[derive(Deserialize)]
struct Verification {
    summary: Summary,
    results: Vec<Harness>,
}

#[derive(Deserialize)]
struct Summary {
    total_harnesses: usize,
    executed: usize,
    successful: usize,
    failed: usize,
    status: String,
}

#[derive(Deserialize)]
struct Harness {
    harness_id: String,
    status: String,
    checks: Vec<Check>,
}

#[derive(Deserialize)]
struct Check {
    status: String,
    category: String,
}

impl Check {
    fn result(&self) -> ProofCheckResult {
        match self.status.as_str() {
            "Success" => ProofCheckResult::Success,
            "Satisfied" => ProofCheckResult::Satisfied,
            "Unreachable" => ProofCheckResult::Unreachable,
            "Failure" => ProofCheckResult::Failure,
            "Unsatisfiable" => ProofCheckResult::Unsatisfiable,
            _ => ProofCheckResult::Invalid,
        }
    }

    fn kind(&self) -> ProofCheckKind {
        match self.category.as_str() {
            "assertion" => ProofCheckKind::Assertion,
            "cover" => ProofCheckKind::Cover,
            _ => ProofCheckKind::Other,
        }
    }
}

#[derive(Deserialize)]
struct CounterexampleError {
    harness_id: String,
    has_errors: bool,
    error_type: String,
    failed_properties_type: String,
    exit_status: String,
}

fn main() -> ExitCode {
    match execute() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("verification failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn execute() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or("missing workspace root")?;
    let config: Config = serde_json::from_slice(&std::fs::read(root.join("verification.json"))?)?;
    let args: Vec<_> = std::env::args().skip(1).collect();
    let advisory_repo = root.join("target/verification/advisory-db");
    match args.as_slice() {
        [command] if command == "native" => native(root),
        [command] if command == "proofs" => proofs(root, &config),
        [command] if command == "verify-evidence" => verify_receipt(root, &config),
        [command] if command == "adversarial" => adversarial(root, &config),
        [command] if command == "security" => security(root, &config, &advisory_repo),
        [command, flag, repo] if command == "security" && flag == "--advisory-repo" => {
            security(root, &config, Path::new(repo))
        }
        [command] if command == "check" => check(root, &config, &advisory_repo),
        [command, flag] if command == "check" && flag == "--offline" => {
            check(root, &config, &advisory_repo)
        }
        [command, flag, repo] if command == "check" && flag == "--advisory-repo" => {
            check(root, &config, Path::new(repo))
        }
        [command, offline, flag, repo]
            if command == "check" && offline == "--offline" && flag == "--advisory-repo" =>
        {
            check(root, &config, Path::new(repo))
        }
        _ => Err(
            "usage: cargo xtask <native|check [--offline] [--advisory-repo PATH]|security [--advisory-repo PATH]|proofs|verify-evidence|adversarial>"
                .into(),
        ),
    }
}

fn checked(command: &mut Command) -> Result<()> {
    eprintln!("Running {command:?}");
    let status = command
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|error| format!("failed to start {command:?}: {error}"))?;
    if !status.success() {
        return Err(format!("{command:?} exited with {status}").into());
    }
    Ok(())
}

fn captured(command: &mut Command) -> Result<Output> {
    eprintln!("Running {command:?}");
    let output = command
        .output()
        .map_err(|error| format!("failed to start {command:?}: {error}"))?;
    print!("{}", String::from_utf8_lossy(&output.stdout));
    eprint!("{}", String::from_utf8_lossy(&output.stderr));
    if !output.status.success() {
        return Err(format!("{command:?} exited with {}", output.status).into());
    }
    Ok(output)
}

fn cargo(root: &Path, args: &[&str]) -> Command {
    let mut command = Command::new("cargo");
    command.current_dir(root).args(args);
    command
}

fn config_names(config: &Config) -> Result<BTreeSet<String>> {
    let names: BTreeSet<_> = config.harnesses.iter().cloned().collect();
    if names.is_empty() || names.len() != config.harnesses.len() {
        return Err("proof inventory must be nonempty and unique".into());
    }
    Ok(names)
}

fn validate_inventory(value: &Value, config: &Config) -> Result<()> {
    if value["kani-version"].as_str() != Some(config.kani_version.as_str()) {
        return Err("Kani version differs from the pinned version".into());
    }
    let groups = value["standard-harnesses"]
        .as_object()
        .ok_or("missing discovered harness inventory")?;
    let mut names = BTreeSet::new();
    for group in groups.values() {
        for name in group.as_array().ok_or("invalid harness group")? {
            if !names.insert(name.as_str().ok_or("invalid harness name")?.to_owned()) {
                return Err("duplicate discovered harness".into());
            }
        }
    }
    if names != config_names(config)? || value["totals"]["contract-harnesses"].as_u64() != Some(0) {
        return Err("discovered proof inventory differs from verification.json".into());
    }
    Ok(())
}

fn validate_metadata(value: &Value, config: &Config) -> Result<()> {
    if value["metadata"]["kani_version"].as_str() != Some(config.kani_version.as_str())
        || value["metadata"]["target"].as_str() != Some(config.proof_target.as_str())
    {
        return Err("incorrect report verifier version or target".into());
    }
    Ok(())
}

fn validate_report(value: &Value, config: &Config) -> Result<()> {
    validate_metadata(value, config)?;
    let report: Verification = serde_json::from_value(value["verification_results"].clone())?;
    let expected = config_names(config)?;
    let actual: BTreeSet<_> = report
        .results
        .iter()
        .map(|r| r.harness_id.clone())
        .collect();
    let counts = wmbd_core::ProofCounts {
        expected: expected.len(),
        total: report.summary.total_harnesses,
        executed: report.summary.executed,
        successful: report.summary.successful,
        failed: report.summary.failed,
    };
    if actual != expected
        || report.results.len() != expected.len()
        || !counts.complete()
        || report.summary.status != "completed"
    {
        return Err("proof report is incomplete or failed".into());
    }
    for harness in report.results {
        if harness.status != "Success"
            || harness.checks.is_empty()
            || !harness
                .checks
                .iter()
                .any(|check| check.category == "assertion" && check.status == "Success")
            || !harness
                .checks
                .iter()
                .any(|check| check.category == "cover" && check.status == "Satisfied")
            || harness
                .checks
                .iter()
                .any(|c| !c.result().accepted(c.category == "cover"))
        {
            return Err(format!(
                "harness {} has failed, empty, or undetermined checks",
                harness.harness_id
            )
            .into());
        }
    }
    Ok(())
}

fn validate_counterexample(
    value: &Value,
    config: &Config,
    expected: &str,
    process_failed: bool,
) -> Result<()> {
    validate_metadata(value, config)?;
    let report: Verification = serde_json::from_value(value["verification_results"].clone())?;
    let errors: Vec<CounterexampleError> = serde_json::from_value(value["error_details"].clone())?;
    let [harness] = report.results.as_slice() else {
        return Err("negative control must report exactly one harness".into());
    };
    let [error] = errors.as_slice() else {
        return Err("negative control must report exactly one attributed error".into());
    };
    let evidence = CounterexampleEvidence {
        counts: wmbd_core::ProofCounts {
            expected: 1,
            total: report.summary.total_harnesses,
            executed: report.summary.executed,
            successful: report.summary.successful,
            failed: report.summary.failed,
        },
        process_failed,
        report_completed: report.summary.status == "completed",
        harness_matches: harness.harness_id == expected && harness.status == "Failure",
        assertion_failed: harness
            .checks
            .iter()
            .any(|c| c.status == "Failure" && c.category == "assertion"),
        checks_valid: harness
            .checks
            .iter()
            .all(|c| c.result().supports_counterexample(c.kind())),
        error_is_assertion: error.harness_id == expected
            && error.has_errors
            && error.error_type == "assertion_failure"
            && error.failed_properties_type == "PanicsOnly"
            && error.exit_status == "properties_failed",
    };
    if !evidence.accepted() {
        return Err(
            "negative control is incomplete or lacks a sound assertion counterexample".into(),
        );
    }
    Ok(())
}

fn read_json(path: impl AsRef<Path>) -> Result<Value> {
    Ok(serde_json::from_slice(&std::fs::read(path)?)?)
}

fn report_dir(root: &Path) -> Result<tempfile::TempDir> {
    let parent = root.join("target/verification");
    std::fs::create_dir_all(&parent)?;
    Ok(tempfile::Builder::new().prefix("run-").tempdir_in(parent)?)
}

fn kani_command(root: &Path, directory: &Path) -> Command {
    let mut command = cargo(directory, &["kani", "-p", "wmbd-core", "--manifest-path"]);
    command
        .arg(root.join("Cargo.toml"))
        .arg("--target-dir")
        .arg(root.join("target/kani"));
    command
}

fn proofs(root: &Path, config: &Config) -> Result<()> {
    let source_sha256 = source_hash(root)?;
    let directory = report_dir(root)?;
    checked(kani_command(root, directory.path()).args(["list", "--format", "json"]))?;
    validate_inventory(&read_json(directory.path().join("kani-list.json"))?, config)?;
    let report = directory.path().join("proofs.json");
    checked(
        kani_command(root, directory.path())
            .args(["-Z", "unstable-options", "--export-json"])
            .arg(&report)
            .args(["--output-format", "terse"]),
    )?;
    let results = read_json(report)?;
    validate_report(&results, config)?;
    if source_hash(root)? != source_sha256 {
        return Err("sources changed during verification".into());
    }
    let receipt = Receipt {
        source_sha256,
        kani_version: config.kani_version.clone(),
        harnesses: config.harnesses.clone(),
        results,
    };
    std::fs::write(
        root.join("target/verification/proofs.json"),
        serde_json::to_vec_pretty(&receipt)?,
    )?;
    Ok(())
}

fn verify_receipt(root: &Path, config: &Config) -> Result<()> {
    let receipt: Receipt = serde_json::from_slice(&std::fs::read(
        root.join("target/verification/proofs.json"),
    )?)?;
    if receipt.source_sha256 != source_hash(root)?
        || receipt.kani_version != config.kani_version
        || receipt.harnesses != config.harnesses
    {
        return Err("proof receipt is stale; run just proofs".into());
    }
    validate_report(&receipt.results, config)?;
    println!("Source-bound proof receipt is current");
    Ok(())
}

fn collect_files(path: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    if path.is_dir() {
        for entry in std::fs::read_dir(path)? {
            collect_files(&entry?.path(), files)?;
        }
    } else {
        files.push(path.to_owned());
    }
    Ok(())
}

fn source_hash(root: &Path) -> Result<String> {
    let mut files = Vec::new();
    for directory in ["src", "crates", "tests", "xtask/src"] {
        collect_files(&root.join(directory), &mut files)?;
    }
    for file in [
        "Cargo.toml",
        "Cargo.lock",
        ".mise.toml",
        ".cargo/config.toml",
        ".gitleaks.toml",
        "justfile",
        "verification.json",
        "verification/tool-assets.sha256",
        ".github/workflows/verification.yml",
        "xtask/Cargo.toml",
        "docs/verification.md",
    ] {
        files.push(root.join(file));
    }
    let ignore = root.join(".gitleaksignore");
    if ignore.try_exists()? {
        files.push(ignore);
    }
    files.sort();
    let mut hash = Sha256::new();
    for file in files {
        let path = file
            .strip_prefix(root)?
            .to_str()
            .ok_or("source path is not valid UTF-8")?;
        let content = std::fs::read(&file)?;
        hash.update(u64::try_from(path.len())?.to_be_bytes());
        hash.update(path.as_bytes());
        hash.update(u64::try_from(content.len())?.to_be_bytes());
        hash.update(content);
    }
    Ok(hash
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn version(root: &Path, program: &str, args: &[&str], expected: &str) -> Result<()> {
    let output = captured(Command::new(program).current_dir(root).args(args))?;
    let text = String::from_utf8(output.stdout)?;
    if !text.split_whitespace().any(|word| word == expected) {
        return Err(format!("{program} must be version {expected}").into());
    }
    Ok(())
}

fn native(root: &Path) -> Result<()> {
    checked(&mut cargo(root, &["fmt", "--all", "--check"]))?;
    checked(&mut cargo(
        root,
        &[
            "clippy",
            "--locked",
            "--offline",
            "--workspace",
            "--all-targets",
            "--",
            "-D",
            "warnings",
        ],
    ))?;
    checked(&mut cargo(
        root,
        &["test", "--locked", "--offline", "--workspace"],
    ))?;
    println!("Native checks passed");
    Ok(())
}

fn check(root: &Path, config: &Config, advisory_repo: &Path) -> Result<()> {
    native(root)?;
    proofs(root, config)?;
    adversarial(root, config)?;
    security(root, config, advisory_repo)?;
    verify_receipt(root, config)?;
    println!("All checks passed against the pinned advisory snapshot; freshness not evaluated");
    Ok(())
}

fn security(root: &Path, config: &Config, advisory_repo: &Path) -> Result<()> {
    let source_sha256 = source_hash(root)?;
    version(root, "gitleaks", &["version"], &config.gitleaks_version)?;
    let directory = report_dir(root)?;
    checked(
        Command::new("gitleaks")
            .current_dir(root)
            .env_remove("GITLEAKS_CONFIG")
            .env_remove("GITLEAKS_CONFIG_TOML")
            .args(["dir", ".", "--no-banner", "--redact", "--config"])
            .arg(root.join(".gitleaks.toml"))
            .args(["--report-format", "json", "--report-path"])
            .arg(directory.path().join("secrets.json")),
    )?;
    version(
        root,
        "cargo",
        &["audit", "--version"],
        &config.audit_version,
    )?;
    let report = audit::run(root, advisory_repo, &config.advisory_revision)?;
    if source_hash(root)? != source_sha256 {
        return Err("sources changed during security checks".into());
    }
    std::fs::write(
        root.join("target/verification/audit.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "source_sha256": source_sha256,
            "audit_version": config.audit_version,
            "gitleaks_version": config.gitleaks_version,
            "audit": report,
        }))?,
    )?;
    println!(
        "Security checks passed against the pinned advisory snapshot; freshness not evaluated"
    );
    Ok(())
}

fn adversarial(root: &Path, config: &Config) -> Result<()> {
    let source = std::fs::read_to_string(root.join("crates/core/src/lib.rs"))?;
    let mutants = [
        (
            "counterexample_contract",
            "&& self.error_is_assertion",
            "&& true",
        ),
        (
            "audit_coverage_contract",
            "&& self.settings_complete",
            "&& true",
        ),
        (
            "scoped_dependency_contract",
            "self == Self::Included && member",
            "self == Self::Included",
        ),
        (
            "dependency_relations_contract",
            "if edge.from == edge.to {\n                return Err(RelationError::SelfReference);",
            "if false {\n                return Err(RelationError::SelfReference);",
        ),
        (
            "bottleneck_rank_contract",
            "affected: core::cmp::Reverse<core::num::NonZeroU64>,\n    priority: core::cmp::Reverse<WorkPriority>,",
            "priority: core::cmp::Reverse<WorkPriority>,\n    affected: core::cmp::Reverse<core::num::NonZeroU64>,",
        ),
        (
            "blocker_chain_contract",
            "node.state != DependencyState::Done",
            "true",
        ),
        (
            "blocker_chain_contract",
            "next.state == DependencyState::Active",
            "true",
        ),
        (
            "impact_chain_contract",
            "DependencyQuery::Impact => root.state == DependencyState::Done",
            "DependencyQuery::Impact => root.state != DependencyState::Active",
        ),
        (
            "impact_chain_contract",
            "DependencyQuery::Impact => node.state == DependencyState::Active",
            "DependencyQuery::Impact => true",
        ),
        (
            "impact_chain_contract",
            "DependencyQuery::Impact if edge.to == next.id => edge.from",
            "DependencyQuery::Impact if edge.from == next.id => edge.to",
        ),
        (
            "proof_check_contract",
            "Self::Success | Self::Unreachable => !cover",
            "Self::Success | Self::Unreachable => true",
        ),
        (
            "revision_change_contract",
            "if revision_matches(expected, current)",
            "if true",
        ),
        ("revision_change_contract", "if !changed", "if false"),
        (
            "repo_count_contract",
            "let open = state == WorkState::Open;",
            "let open = state != WorkState::Cancelled;",
        ),
        (
            "repo_count_contract",
            "active && !dependencies_done",
            "open && !dependencies_done",
        ),
        (
            "descendant_selection_contract",
            "*mark = DescendantMark::Outside;",
            "*mark = DescendantMark::Included;",
        ),
        (
            "subtree_selection_contract",
            "if node.id == root {\n                *mark = DescendantMark::Pending;\n            }",
            "if node.id == root {\n                *mark = DescendantMark::Outside;\n            }",
        ),
        ("readiness_contract", "open && dependencies_done", "open"),
        (
            "identifiers_and_revisions",
            "expected != 0 && expected == current",
            "expected != 0",
        ),
        (
            "triangle_cycle_contract",
            "Err(RelationError::Cycle)",
            "Ok(())",
        ),
    ];
    for (harness, before, after) in mutants {
        eprintln!("Checking negative control: {harness} ({before} -> {after})");
        if !source.contains(before) {
            return Err(format!("missing mutation target: {harness}").into());
        }
        let directory = report_dir(root)?;
        let scratch = directory.path();
        std::fs::create_dir(scratch.join("src"))?;
        std::fs::write(
            scratch.join("Cargo.toml"),
            "[package]\nname = \"wmbd-core\"\nversion = \"0.1.0\"\nedition = \"2024\"\n[workspace]\n[lints.rust]\nunexpected_cfgs = { level = \"warn\", check-cfg = ['cfg(kani)'] }\n",
        )?;
        std::fs::write(
            scratch.join("src/lib.rs"),
            source.replacen(before, after, 1),
        )?;
        let report = scratch.join("failure.json");
        let output = cargo(scratch, &["kani", "--target-dir"])
            .arg(scratch.join("build"))
            .args(["-Z", "unstable-options", "--harness"])
            .arg(format!("proofs::{harness}"))
            .args(["--exact", "--export-json"])
            .arg(&report)
            .args(["--output-format", "terse"])
            .output()?;
        let value = read_json(report)?;
        if let Err(error) = validate_counterexample(
            &value,
            config,
            &format!("proofs::{harness}"),
            !output.status.success(),
        ) {
            eprint!("{}", String::from_utf8_lossy(&output.stderr));
            return Err(
                format!("negative control {harness} ({before} -> {after}): {error}").into(),
            );
        }
        println!("Rejected production mutant: {harness} ({before} -> {after})");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::{BufRead, Read, Write};
    use std::time::Duration;

    fn source_fixture() -> Result<tempfile::TempDir> {
        let directory = tempfile::tempdir()?;
        for path in ["src", "crates", "tests", "xtask/src"] {
            std::fs::create_dir_all(directory.path().join(path))?;
        }
        for file in [
            "Cargo.toml",
            "Cargo.lock",
            ".mise.toml",
            ".cargo/config.toml",
            ".gitleaks.toml",
            "justfile",
            "verification.json",
            "verification/tool-assets.sha256",
            ".github/workflows/verification.yml",
            "xtask/Cargo.toml",
            "docs/verification.md",
        ] {
            let path = directory.path().join(file);
            std::fs::create_dir_all(path.parent().ok_or("missing fixture parent")?)?;
            std::fs::write(path, [])?;
        }
        Ok(directory)
    }

    #[test]
    fn source_hash_distinguishes_file_boundaries_and_scan_configuration() -> Result<()> {
        let first = source_fixture()?;
        let second = source_fixture()?;
        std::fs::write(first.path().join("src/a"), b"x\0src/b\0y")?;
        std::fs::write(second.path().join("src/a"), b"x")?;
        std::fs::write(second.path().join("src/b"), b"y")?;
        assert_ne!(source_hash(first.path())?, source_hash(second.path())?);

        let root = source_fixture()?;
        let original = source_hash(root.path())?;
        let config = root.path().join(".gitleaks.toml");
        std::fs::write(&config, b"[extend]\nuseDefault = true\n")?;
        assert_ne!(source_hash(root.path())?, original);
        std::fs::write(config, [])?;
        assert_eq!(source_hash(root.path())?, original);
        let ignore = root.path().join(".gitleaksignore");
        std::fs::write(&ignore, [])?;
        let empty_ignore = source_hash(root.path())?;
        assert_ne!(empty_ignore, original);
        std::fs::write(&ignore, b"reviewed-fingerprint\n")?;
        assert_ne!(source_hash(root.path())?, empty_ignore);
        std::fs::remove_file(ignore)?;
        assert_eq!(source_hash(root.path())?, original);
        Ok(())
    }

    fn fixture(mode: &str) -> Result<Command> {
        let mut command = Command::new(std::env::current_exe()?);
        command
            .args(["--exact", "tests::command_fixture_child", "--nocapture"])
            .env("WMBD_XTASK_FIXTURE", mode);
        Ok(command)
    }

    #[test]
    fn command_fixture_child() -> Result<()> {
        let Ok(mode) = std::env::var("WMBD_XTASK_FIXTURE") else {
            return Ok(());
        };
        match mode.as_str() {
            "success" => std::process::exit(0),
            "failure" => {
                eprintln!("expected runner fixture failure");
                std::process::exit(23);
            }
            "raw" => {
                std::io::stdout().write_all(b"raw-stdout-\xff\n")?;
                std::io::stderr().write_all(b"raw-stderr-\xfe\n")?;
                std::io::stdout().flush()?;
                std::io::stderr().flush()?;
                std::process::exit(0);
            }
            "relay" => match checked(&mut fixture("blocked")?) {
                Ok(()) => std::process::exit(0),
                Err(error) => {
                    eprintln!("{error}");
                    std::process::exit(29);
                }
            },
            "blocked" => {
                let path =
                    std::env::var_os("WMBD_XTASK_RELEASE").ok_or("missing fixture release")?;
                std::io::stdout().write_all(b"live-stdout\n")?;
                std::io::stderr().write_all(b"live-stderr\n")?;
                std::io::stdout().flush()?;
                std::io::stderr().flush()?;
                let deadline = std::time::Instant::now() + Duration::from_secs(15);
                loop {
                    match std::fs::metadata(&path) {
                        Ok(_) => break,
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                            if std::time::Instant::now() >= deadline {
                                return Err("fixture release timed out".into());
                            }
                            std::thread::sleep(Duration::from_millis(1));
                        }
                        Err(error) => return Err(error.into()),
                    }
                }
                std::io::stdout().write_all(b"released-stdout-\xff\n")?;
                std::io::stderr().write_all(b"released-stderr-\xfe\n")?;
                std::io::stdout().flush()?;
                std::io::stderr().flush()?;
                std::process::exit(23);
            }
            _ => Err("unknown fixture mode".into()),
        }
    }

    fn observe(
        stream: impl Read + Send + 'static,
        marker: &'static [u8],
        events: std::sync::mpsc::Sender<&'static [u8]>,
    ) -> std::thread::JoinHandle<std::io::Result<Vec<u8>>> {
        std::thread::spawn(move || {
            let mut stream = std::io::BufReader::new(stream);
            let mut bytes = Vec::new();
            loop {
                let mut line = Vec::new();
                if stream.read_until(b'\n', &mut line)? == 0 {
                    return Ok(bytes);
                }
                if line == marker {
                    let _ = events.send(marker);
                }
                bytes.extend(line);
            }
        })
    }

    #[test]
    fn checked_streams_both_channels_before_child_exit_and_preserves_failure() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("gate");
        let mut relay = fixture("relay")?
            .env("WMBD_XTASK_RELEASE", &path)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let (events, receive) = std::sync::mpsc::channel();
        let stdout = observe(
            relay.stdout.take().ok_or("missing relay stdout")?,
            b"live-stdout\n",
            events.clone(),
        );
        let stderr = observe(
            relay.stderr.take().ok_or("missing relay stderr")?,
            b"live-stderr\n",
            events,
        );
        let live: Result<()> = (|| {
            let mut seen = BTreeSet::new();
            for _ in 0..2 {
                seen.insert(receive.recv_timeout(Duration::from_secs(5))?);
            }
            if seen != BTreeSet::from([b"live-stdout\n".as_slice(), b"live-stderr\n".as_slice()]) {
                return Err("both streams must be visible before release".into());
            }
            if relay.try_wait()?.is_some() {
                return Err("relay exited before release".into());
            }
            Ok(())
        })();
        let release = std::fs::write(&path, []);
        let status = relay.wait();
        let stdout = stdout.join();
        let stderr = stderr.join();
        release?;
        let status = status?;
        let stdout = stdout.map_err(|_| "stdout observer panicked")??;
        let stderr = stderr.map_err(|_| "stderr observer panicked")??;
        live?;
        assert_eq!(status.code(), Some(29));
        assert!(stdout.ends_with(b"released-stdout-\xff\n"));
        assert!(
            stderr
                .windows(b"released-stderr-\xfe\n".len())
                .any(|bytes| bytes == b"released-stderr-\xfe\n")
        );
        assert!(String::from_utf8_lossy(&stderr).contains("exit status: 23"));
        Ok(())
    }

    #[test]
    fn command_modes_preserve_success_capture_and_spawn_errors() -> Result<()> {
        checked(&mut fixture("success")?)?;
        let raw = captured(&mut fixture("raw")?)?;
        assert!(raw.stdout.ends_with(b"raw-stdout-\xff\n"));
        assert_eq!(raw.stderr, b"raw-stderr-\xfe\n");
        for error in [
            checked(&mut fixture("failure")?).err(),
            captured(&mut fixture("failure")?).err(),
        ] {
            let error = error.ok_or("failed child was accepted")?;
            assert!(error.to_string().contains("exit status: 23"));
        }
        let temp = tempfile::tempdir()?;
        let program = temp.path().join("missing checker");
        for error in [
            checked(&mut Command::new(&program)).err(),
            captured(&mut Command::new(&program)).err(),
        ] {
            let error = error.ok_or("missing program was accepted")?;
            assert!(error.to_string().contains("missing checker"));
        }
        Ok(())
    }

    fn counterexample_config() -> Config {
        Config {
            kani_version: "0.68.0".into(),
            proof_target: "aarch64-apple-darwin".into(),
            audit_version: String::new(),
            advisory_revision: String::new(),
            gitleaks_version: String::new(),
            harnesses: vec!["proof".into()],
        }
    }

    fn counterexample_fixture() -> Value {
        json!({
            "metadata": {"kani_version":"0.68.0", "target":"aarch64-apple-darwin"},
            "verification_results": {
                "summary": {"total_harnesses":1, "executed":1, "successful":0, "failed":1, "status":"completed"},
                "results": [{"harness_id":"proof", "status":"Failure", "checks":[
                    {"status":"Failure", "category":"assertion"},
                    {"status":"Success", "category":"overflow"},
                    {"status":"Satisfied", "category":"cover"}
                ]}]
            },
            "error_details": [{"harness_id":"proof", "has_errors":true, "error_type":"assertion_failure", "failed_properties_type":"PanicsOnly", "exit_status":"properties_failed"}]
        })
    }

    #[test]
    fn negative_controls_refuse_solver_errors_even_with_assertion_failures() -> Result<()> {
        let config = counterexample_config();
        let good = counterexample_fixture();
        validate_counterexample(&good, &config, "proof", true)?;
        let mut mixed = good.clone();
        mixed["error_details"][0]["error_type"] = json!("property_error");
        mixed["error_details"][0]["failed_properties_type"] = json!("Error");
        assert!(validate_counterexample(&mixed, &config, "proof", true).is_err());
        mixed["verification_results"]["results"][0]["checks"]
            .as_array_mut()
            .ok_or("missing fixture checks")?
            .push(json!({"status":"Error", "category":"assertion"}));
        assert!(validate_counterexample(&mixed, &config, "proof", true).is_err());
        for category in ["assertion", "cover", "overflow"] {
            for status in [
                "Error",
                "Unknown",
                "Undetermined",
                "Covered",
                "Uncovered",
                "FutureStatus",
            ] {
                let mut changed = good.clone();
                changed["verification_results"]["results"][0]["checks"]
                    .as_array_mut()
                    .ok_or("missing fixture checks")?
                    .push(json!({"status":status, "category":category}));
                assert!(
                    validate_counterexample(&changed, &config, "proof", true).is_err(),
                    "{category}: {status}"
                );
            }
        }
        for (category, status) in [
            ("overflow", "Failure"),
            ("cover", "Failure"),
            ("cover", "Success"),
            ("assertion", "Satisfied"),
            ("overflow", "Unsatisfiable"),
        ] {
            let mut changed = good.clone();
            changed["verification_results"]["results"][0]["checks"]
                .as_array_mut()
                .ok_or("missing fixture checks")?
                .push(json!({"status":status, "category":category}));
            assert!(
                validate_counterexample(&changed, &config, "proof", true).is_err(),
                "{category}: {status}"
            );
        }
        for status in ["Satisfied", "Unsatisfiable", "Unreachable"] {
            let mut changed = good.clone();
            changed["verification_results"]["results"][0]["checks"][2]["status"] = json!(status);
            validate_counterexample(&changed, &config, "proof", true)?;
        }
        let mut unreachable = good;
        unreachable["verification_results"]["results"][0]["checks"][1]["status"] =
            json!("Unreachable");
        validate_counterexample(&unreachable, &config, "proof", true)?;
        Ok(())
    }

    #[test]
    fn negative_controls_require_complete_attributed_single_harness_failures() -> Result<()> {
        let config = counterexample_config();
        let good = counterexample_fixture();
        assert!(validate_counterexample(&good, &config, "proof", false).is_err());
        assert!(validate_counterexample(&good, &config, "another", true).is_err());
        for (pointer, replacement) in [
            ("/metadata/kani_version", json!("unknown")),
            ("/metadata/target", json!("x86_64-unknown-linux-gnu")),
            ("/verification_results/summary/status", json!("interrupted")),
            ("/verification_results/summary/total_harnesses", json!(0)),
            ("/verification_results/summary/total_harnesses", json!(2)),
            ("/verification_results/summary/executed", json!(0)),
            ("/verification_results/summary/executed", json!(2)),
            ("/verification_results/summary/successful", json!(1)),
            ("/verification_results/summary/failed", json!(0)),
            ("/verification_results/summary/failed", json!(2)),
            (
                "/verification_results/results/0/harness_id",
                json!("another"),
            ),
            ("/verification_results/results/0/status", json!("Success")),
            ("/verification_results/results/0/checks", json!([])),
            (
                "/verification_results/results/0/checks/0/status",
                json!("Success"),
            ),
            ("/error_details/0/harness_id", json!("another")),
            ("/error_details/0/has_errors", json!(false)),
            ("/error_details/0/error_type", json!("property_error")),
            ("/error_details/0/error_type", json!("verification_failure")),
            ("/error_details/0/error_type", json!("unknown_failure")),
            ("/error_details/0/failed_properties_type", json!("Error")),
            ("/error_details/0/failed_properties_type", json!("Other")),
            ("/error_details/0/exit_status", json!("timeout")),
            ("/error_details/0/exit_status", json!("out_of_memory")),
            ("/error_details/0/exit_status", json!("exit_code_1")),
        ] {
            let mut changed = good.clone();
            *changed
                .pointer_mut(pointer)
                .ok_or("missing fixture field")? = replacement;
            assert!(
                validate_counterexample(&changed, &config, "proof", true).is_err(),
                "{pointer}"
            );
        }
        for pointer in ["/verification_results/results", "/error_details"] {
            let mut empty = good.clone();
            *empty.pointer_mut(pointer).ok_or("missing fixture field")? = json!([]);
            assert!(validate_counterexample(&empty, &config, "proof", true).is_err());
            let mut duplicate = good.clone();
            let entries = duplicate
                .pointer_mut(pointer)
                .and_then(Value::as_array_mut)
                .ok_or("missing fixture array")?;
            entries.push(entries.first().ok_or("empty fixture array")?.clone());
            assert!(validate_counterexample(&duplicate, &config, "proof", true).is_err());
        }
        for field in [
            "harness_id",
            "has_errors",
            "error_type",
            "failed_properties_type",
            "exit_status",
        ] {
            let mut missing = good.clone();
            missing["error_details"][0]
                .as_object_mut()
                .ok_or("missing fixture object")?
                .remove(field);
            assert!(
                validate_counterexample(&missing, &config, "proof", true).is_err(),
                "{field}"
            );
            let mut invalid = good.clone();
            invalid["error_details"][0][field] = Value::Null;
            assert!(
                validate_counterexample(&invalid, &config, "proof", true).is_err(),
                "{field}"
            );
        }
        Ok(())
    }

    #[test]
    fn evidence_refuses_missing_failed_and_empty_harnesses() -> Result<()> {
        let config = Config {
            kani_version: "0.68.0".into(),
            proof_target: "aarch64-apple-darwin".into(),
            audit_version: String::new(),
            advisory_revision: String::new(),
            gitleaks_version: String::new(),
            harnesses: vec!["proof".into()],
        };
        let good = json!({"metadata":{"kani_version":"0.68.0","target":"aarch64-apple-darwin"},"verification_results":{"summary":{"total_harnesses":1,"executed":1,"successful":1,"failed":0,"status":"completed"},"results":[{"harness_id":"proof","status":"Success","checks":[{"status":"Success","category":"assertion"},{"status":"Satisfied","category":"cover"}]}]}});
        validate_report(&good, &config)?;
        for target in [Value::Null, json!("x86_64-unknown-linux-gnu"), json!(7)] {
            let mut changed = good.clone();
            changed["metadata"]["target"] = target;
            assert!(validate_report(&changed, &config).is_err());
        }
        for (category, status, accepted) in [
            ("cover", "Unreachable", false),
            ("cover", "Success", false),
            ("cover", "Failure", false),
            ("cover", "Undetermined", false),
            ("cover", "Unknown", false),
            ("assertion", "Unreachable", true),
            ("assertion", "Unknown", false),
        ] {
            let mut changed = good.clone();
            changed["verification_results"]["results"][0]["checks"]
                .as_array_mut()
                .ok_or("missing fixture checks")?
                .push(json!({"category":category,"status":status}));
            assert_eq!(
                validate_report(&changed, &config).is_ok(),
                accepted,
                "{category}: {status}"
            );
        }
        let mut missing = good.clone();
        missing["verification_results"]["results"] = json!([]);
        assert!(validate_report(&missing, &config).is_err());
        let mut failed = good.clone();
        failed["verification_results"]["results"][0]["status"] = json!("Failure");
        assert!(validate_report(&failed, &config).is_err());
        let mut empty = good.clone();
        empty["verification_results"]["results"][0]["checks"] = json!([]);
        assert!(validate_report(&empty, &config).is_err());
        let mut timeout = good;
        timeout["verification_results"]["summary"]["status"] = json!("timeout");
        assert!(validate_report(&timeout, &config).is_err());
        Ok(())
    }
}
