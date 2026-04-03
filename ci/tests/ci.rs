use gh_workflow::generate::Generate;
use gh_workflow::*;
use serde_json::json;

#[test]
fn main() {
    // Test and Coverage job — runs in parallel via matrix (one per workspace)
    let test_job = Job::new("Test and Coverage")
        .name("Test (${{ matrix.workspace }})")
        .runs_on("ubuntu-latest")
        .permissions(Permissions::default().contents(Level::Read))
        .strategy(
            Strategy::default()
                .fail_fast(false)
                .matrix(json!({
                    "include": [
                        {"workspace": "fetcher-be", "lcov_name": "lcov-fetcher"},
                        {"workspace": "brainatlas-be", "lcov_name": "lcov-brainatlas"},
                        {"workspace": "orch", "lcov_name": "lcov-orch"}
                    ]
                })),
        )
        .add_step(Step::new("Checkout Code").uses("actions", "checkout", "34e114876b0b11c390a56381ad16ebd13914f8d5"))
        .add_step(protoc_and_lcov_install())
        .add_step(test_infrastructure())
        .add_step(run_migrations())
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
                        .add("cache", "true")
                        .add(
                            "cache-workspaces",
                            "${{ matrix.workspace }} -> ${{ matrix.workspace }}/target",
                        ),
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
            Step::new("Generate coverage")
                .run(
                    "cd ${{ matrix.workspace }} && cargo +nightly llvm-cov --all-features --workspace --lcov --output-path ../${{ matrix.lcov_name }}.info -- --test-threads=1",
                )
                .env(test_env()),
        )
        .add_step(
            Step::new("Upload Coverage Artifact")
                .uses("actions", "upload-artifact", "v4")
                .with(
                    Input::default()
                        .add("name", "${{ matrix.lcov_name }}")
                        .add("path", "${{ matrix.lcov_name }}.info")
                        .add("retention-days", "1"),
                ),
        );

    // Coverage merge job — waits for all matrix test jobs, then combines reports
    let coverage_job = Job::new("Merge Coverage")
        .name("Merge Coverage")
        .runs_on("ubuntu-latest")
        .add_needs("test")
        .cond(Expression::new("always()"))
        .permissions(Permissions::default().contents(Level::Read))
        .add_step(Step::new("Checkout Code").uses("actions", "checkout", "34e114876b0b11c390a56381ad16ebd13914f8d5"))
        .add_step(
            Step::new("Install lcov")
                .run("sudo apt-get update && sudo apt-get install -y lcov"),
        )
        .add_step(
            Step::new("Download Coverage Artifacts")
                .uses("actions", "download-artifact", "v4")
                .with(
                    Input::default()
                        .add("pattern", "lcov-*")
                        .add("merge-multiple", "true"),
                ),
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

    let workflow = Workflow::new("ci")
        .name("ci")
        .env(Env::from(("RUSTFLAGS", "-Dwarnings")))
        .on(Event::default()
            .pull_request(
                PullRequest::default()
                    .add_branch("main")
                    .add_type(PullRequestType::Opened)
                    .add_type(PullRequestType::Synchronize)
                    .add_type(PullRequestType::Reopened),
            )
            .push(Push::default().add_branch("main")))
        .add_job("test", test_job)
        .add_job("coverage", coverage_job);

    workflow.generate().unwrap();
}

#[test]
fn autofix_workflow() {
    let lint_fix_job = Job::new("Lint Fix")
        .name("Lint Fix")
        .runs_on("ubuntu-latest")
        .permissions(Permissions::default().contents(Level::Read))
        .add_step(Step::new("Checkout Code").uses(
            "actions",
            "checkout",
            "34e114876b0b11c390a56381ad16ebd13914f8d5",
        ))
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
                        .add("components", "clippy, rustfmt")
                        .add("cache", "true")
                        .add(
                            "cache-workspaces",
                            "fetcher-be -> fetcher-be/target\nbrainatlas-be -> brainatlas-be/target\norch -> orch/target",
                        ),
                ),
        )
        .add_step(Step::new("Cargo Fmt").run(
            "(cd fetcher-be && cargo +nightly fmt --all) && \
(cd brainatlas-be && cargo +nightly fmt --all) && \
(cd orch && cargo +nightly fmt --all)",
        ))
        .add_step(Step::new("Cargo Clippy").run(
            "(cd fetcher-be && cargo +nightly clippy --all-features --workspace --fix --allow-dirty -- -D warnings) && \
(cd brainatlas-be && cargo +nightly clippy --all-features --workspace --fix --allow-dirty -- -D warnings) && \
(cd orch && cargo +nightly clippy --all-features --workspace --fix --allow-dirty -- -D warnings)",
        ))
        .add_step(Step::new("Autofix").uses(
            "autofix-ci",
            "action",
            "7a166d7532b277f34e16238930461bf77f9d7ed8",
        ));

    let events = Event::default()
        .pull_request(
            PullRequest::default()
                .add_branch("main")
                .add_type(PullRequestType::Opened)
                .add_type(PullRequestType::Synchronize)
                .add_type(PullRequestType::Reopened),
        )
        .push(Push::default().add_branch("main"));

    let workflow = Workflow::new("autofix")
        .name("autofix.ci")
        .env(Env::from(("RUSTFLAGS", "-Dwarnings")))
        .on(events)
        .concurrency(
            Concurrency::default()
                .group("autofix-${{github.ref}}")
                .cancel_in_progress(false),
        )
        .add_job("lint", lint_fix_job);

    Generate::new(workflow).name("autofix.yml").generate().unwrap();
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
        "docker compose -f docker-compose.test.yml up -d --wait postgres-test redis-test minio-test && \
docker compose -f docker-compose.test.yml run --rm minio-setup",
    )
}

fn run_migrations() -> Step<Run> {
    Step::new("Run Database Migrations")
        .run(r#"export PGPASSWORD=test_password
PSQL="psql -h localhost -p 5433 -U test_user -d test_db"
$PSQL -v ON_ERROR_STOP=1 -c "
CREATE TABLE IF NOT EXISTS region_mapping (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    region_id INTEGER NOT NULL UNIQUE,
    name VARCHAR(255) NOT NULL UNIQUE,
    acronym VARCHAR(50),
    red INTEGER,
    green INTEGER,
    blue INTEGER,
    structure_order INTEGER,
    parent_region_id INTEGER,
    parent_acronym VARCHAR(50),
    created_at TIMESTAMP
);
CREATE TABLE IF NOT EXISTS region_summary (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    region_id INTEGER NOT NULL UNIQUE,
    name VARCHAR(255) NOT NULL UNIQUE,
    acronym VARCHAR(50),
    summary TEXT,
    created_at TIMESTAMP
);"
for f in fetcher-be/migrations/*/up.sql; do
  echo "Running migration: $f"
  $PSQL -f "$f" || true
done
for f in orch/migrations/*/up.sql; do
  echo "Running migration: $f"
  $PSQL -f "$f" || true
done
for f in brainatlas-be/migrations/*/up.sql; do
  echo "Running migration: $f"
  $PSQL -f "$f" || true
done"#)
}

fn test_env() -> Env {
    Env::default()
        .add(
            "DATABASE_URL",
            "postgresql://test_user:test_password@localhost:5433/test_db",
        )
        .add(
            "TEST_DATABASE_URL",
            "postgresql://test_user:test_password@localhost:5433/test_db",
        )
        .add("RUN_INTEGRATION_TESTS", "1")
        .add("REDIS_URL", "redis://127.0.0.1:6380")
        .add("S3_ENDPOINT", "http://localhost:9000")
        .add("S3_ACCESS_KEY", "test_access_key")
        .add("S3_SECRET_KEY", "test_secret_key")
        .add("S3_BUCKET", "test-bucket")
}
