default:
    @just --list

check *args:
    cargo run --locked --package xtask --target-dir target/xtask-tool -- check {{args}}

native:
    cargo run --locked --package xtask --target-dir target/xtask-tool -- native

security *args:
    cargo run --locked --package xtask --target-dir target/xtask-tool -- security {{args}}

proofs:
    cargo run --locked --package xtask --target-dir target/xtask-tool -- proofs

verify-evidence:
    cargo run --locked --package xtask --target-dir target/xtask-tool -- verify-evidence

build:
    cargo build --locked --release --bin wmbd
