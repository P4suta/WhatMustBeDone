use clap::Parser;
use std::io::Write;
use std::process::ExitCode;
use what_must_be_done::api::ErrorEnvelope;
use what_must_be_done::cli::{Cli, execute};
use what_must_be_done::error::Error;

fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => {
            if matches!(
                error.kind(),
                clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
            ) {
                return if error.print().is_ok() {
                    ExitCode::SUCCESS
                } else {
                    ExitCode::FAILURE
                };
            }
            return report(Error::input("arguments", error.to_string()), 2);
        }
    };
    match execute(cli) {
        Ok(value) => {
            let stdout = std::io::stdout();
            let mut output = stdout.lock();
            if serde_json::to_writer(&mut output, &value).is_err() || writeln!(output).is_err() {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            }
        }
        Err(error) => report(error, 1),
    }
}

fn report(error: Error, status: u8) -> ExitCode {
    let stderr = std::io::stderr();
    let mut output = stderr.lock();
    if serde_json::to_writer(
        &mut output,
        &ErrorEnvelope {
            api_version: 1,
            error,
        },
    )
    .is_err()
        || writeln!(output).is_err()
    {
        ExitCode::FAILURE
    } else {
        ExitCode::from(status)
    }
}
