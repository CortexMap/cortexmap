use gh_workflow::*;

#[test]
fn main() {
    // Build and Test job with coverage
    let build_job = Job::new("Build and Test")
        .name("Build and Test")
        .runs_on("ubuntu-latest")
        .permissions(Permissions::default().contents(Level::Read))
        .add_step(Step::new("Checkout Code").uses("actions", "checkout", "34e114876b0b11c390a56381ad16ebd13914f8d5"))
        .add_step(protoc_and_lcov_install())
        .add_step(test_infrastructure())
        .add_step(
            Step::new("Setup Rust Toolchain")
                .uses(
                    "actions-rust-lang",
                    "setup-rust-toolchain",
                    "1780873c7b576612439a134613cc4cc74ce5538c",
                )
                .with(
                    Input::default()
                        .add("toolchain", "nightly")
                        .add("components", "llvm-tools-preview")
                        .add("cache", "true"),
                ),
        )
        .add_step(
            Step::new("Cache cargo-llvm-cov")
                .uses("actions", "cache", "0057852bfaa89a56745cba8c7296529d2fc39830")
                .with(
                    Input::default()
                        .add("path", "~/.cargo/bin/cargo-llvm-cov")
                        .add(
                            "key",
                            "${{ runner.os }}-cargo-llvm-cov-${{ hashFiles('**/Cargo.lock') }}",
                        )
                        .add("restore-keys", "${{ runner.os }}-cargo-llvm-cov-"),
                ),
        )
        .add_step(Step::new("Install cargo-llvm-cov").run("cargo install cargo-llvm-cov || true"))
        .add_step(
            Step::new("Generate coverage (main branch or expensive)")
                .run(
                    "(cd fetcher-be && cargo +nightly llvm-cov --release --all-features --workspace --lcov --output-path ../lcov-fetcher.info) && \
(cd brainatlas-be && cargo +nightly llvm-cov --release --all-features --workspace --lcov --output-path ../lcov-brainatlas.info) && \
(cd orch && cargo +nightly llvm-cov --release --all-features --workspace --lcov --output-path ../lcov-orch.info)",
                )
                .if_condition(Expression::new(
                    "${{ github.ref == 'refs/heads/main' || contains(github.event.pull_request.labels.*.name, 'ci: expensive') }}",
                )),
        )
        .add_step(
            Step::new("Generate coverage (fast)")
                .run(
                    "(cd fetcher-be && cargo +nightly llvm-cov --workspace --lcov --output-path ../lcov-fetcher.info) && \
(cd brainatlas-be && cargo +nightly llvm-cov --workspace --lcov --output-path ../lcov-brainatlas.info) && \
(cd orch && cargo +nightly llvm-cov --workspace --lcov --output-path ../lcov-orch.info)",
                )
                .if_condition(Expression::new(
                    "${{ github.ref != 'refs/heads/main' && !contains(github.event.pull_request.labels.*.name, 'ci: expensive') }}",
                )),
        )
        .add_step(Step::new("Merge coverage reports").run(
            "lcov \
--add-tracefile lcov-fetcher.info \
--add-tracefile lcov-brainatlas.info \
--add-tracefile lcov-orch.info \
--output-file lcov.info",
        ))
        .add_step(
            Step::new("Upload Coverage to Codecov")
                .uses("Wandalen", "wretry.action", "e68c23e6309f2871ca8ae4763e7629b9c258e1ea")
                .with(
                    Input::default()
                        .add("action", "codecov/codecov-action@v4")
                        .add("attempt_limit", "3")
                        .add("attempt_delay", "10000")
                        .add(
                            "with",
                            "token: ${{ secrets.CODECOV_TOKEN }}\nfiles: lcov.info",
                        ),
                ),
        );

    // Lint job
    let lint_job = Job::new("Lint")
        .name("Lint")
        .runs_on("ubuntu-latest")
        .permissions(Permissions::default().contents(Level::Read))
        .add_step(Step::new("Checkout Code").uses("actions", "checkout", "34e114876b0b11c390a56381ad16ebd13914f8d5"))
        .add_step(protoc_install())
        .add_step(
            Step::new("Setup Rust Toolchain")
                .uses(
                    "actions-rust-lang",
                    "setup-rust-toolchain",
                    "1780873c7b576612439a134613cc4cc74ce5538c",
                )
                .with(
                    Input::default()
                        .add("toolchain", "nightly")
                        .add("components", "clippy, rustfmt"),
                ),
        )
        .add_step(Step::new("Cargo Fmt").run(
            "(cd fetcher-be && cargo +nightly fmt --all --check) && \
(cd brainatlas-be && cargo +nightly fmt --all --check) && \
(cd orch && cargo +nightly fmt --all --check)",
        ))
        .add_step(Step::new("Cargo Clippy").run(
            "(cd fetcher-be && cargo +nightly clippy --all-features --workspace -- -D warnings) && \
(cd brainatlas-be && cargo +nightly clippy --all-features --workspace -- -D warnings) && \
(cd orch && cargo +nightly clippy --all-features --workspace -- -D warnings)",
        ));

    let workflow = Workflow::new("ci")
        .name("ci")
        .env(Env::from(("RUSTFLAGS", "-Dwarnings")))
        .on(
            Event::default()
                .pull_request(
                    PullRequest::default()
                        .add_branch("main")
                        .add_type(PullRequestType::Opened)
                        .add_type(PullRequestType::Synchronize)
                        .add_type(PullRequestType::Reopened),
                )
                .push(Push::default().add_branch("main")),
        )
        .add_job("build", build_job)
        .add_job("lint", lint_job);

    workflow.generate().unwrap();
}

fn protoc_install() -> Step<Run> {
    Step::new("Install protoc")
        .run("sudo apt-get update && sudo apt-get install -y protobuf-compiler")
}

fn protoc_and_lcov_install() -> Step<Run> {
    Step::new("Install protoc and lcov")
        .run("sudo apt-get update && sudo apt-get install -y protobuf-compiler lcov")
}

fn test_infrastructure() -> Step<Run> {
    Step::new("Start Test Infrastructure").run(
        "docker compose -f docker-compose.test.yml up -d && \
sleep 5",
    )
}
