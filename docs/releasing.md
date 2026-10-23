# Release preparation

[Project overview](../README.md) · [Contributing](../CONTRIBUTING.md)

Source and versioned distribution artifacts are available on [GitHub](https://github.com/christypixel68-cloud/mdlink-audit/releases). The crate has not been published to crates.io. Use this checklist to verify a candidate before creating a versioned release or publishing distribution artifacts.

## Verify a candidate

From the repository root:

```sh
cargo fmt --all --check
cargo clippy --locked --all-targets -- -D warnings
cargo build --release --locked
cargo test --locked
cargo run --release --locked --bin mdlink-audit -- --fail-on-empty
cargo run --locked --example demo
bash scripts/smoke-install.sh
```

The release build places `mdlink-audit`, `mdlink-audit-action` and `mdlink-audit-corpus` in `target/release/`. The checker's audit itself makes no network requests.

To verify the pre-commit integration, add the hook to a temporary consumer repository, select only an image change, and confirm the hook fails before and passes after correcting the image path. Hook installation builds the crate and may download dependencies.

The [CI workflow](../.github/workflows/ci.yml) configures Linux, Windows, and macOS builds and unit tests and an audit of this repository's documentation. The composite Action is also installed and exercised on all three operating systems, including a deliberately broken HTML link and its correction. Review the [Actions results](https://github.com/christypixel68-cloud/mdlink-audit/actions/workflows/ci.yml) for the exact commit being released; a local test does not establish that every configured platform passed. Symlink fixtures skip only when the host cannot create symlinks.

## Inspect artifacts

Inspect the release binaries and the source archive (`cargo package --list --allow-dirty` lists its files) before distribution. Confirm that the archive includes the sources, license, documentation, tests, examples, and the install smoke script. Exclude virtual environments, caches, credentials, and unrelated workspace content.

Build into a directory containing only the current version. Keep previous artifacts elsewhere before running `scripts/smoke-install.sh`, which installs the three programs into a temporary root offline and probes a broken and a fixed link. Upload the binaries, source archive, and a SHA-256 checksum file to a GitHub release targeting the exact passing commit. Download the public artifacts and verify their hashes before announcing the release. Do not move an existing release tag to a different commit.

Check that the `version` in `Cargo.toml` (which `mdlink-audit --version` prints) matches the release tag and that `Cargo.lock` is committed. Record real changes and limitations in [CHANGELOG.md](../CHANGELOG.md). Package metadata and repository links must refer to the actual public repository before publication.

## Publish deliberately

Create a release only after reviewing the candidate and passing the intended platform checks. If publishing to a package index, verify ownership and name availability there first; the source checkout does not reserve the name. Prefer a dedicated publishing environment with trusted publishing over storing API tokens in this repository. Do not reuse an already-published version number.

Keep release notes factual: list the changes, tests that ran, known limitations, and upgrade effects. Do not describe configured jobs as successful until they have run.
