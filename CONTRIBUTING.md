# Contributing

Thanks for helping improve mdlink-audit. Useful contributions include reproducible bug reports, clearer documentation, parser fixtures, and cross-platform fixes.

[Usage](docs/usage.md) · [Design and limitations](docs/design.md) · [Roadmap](docs/roadmap.md)

## Set up a checkout

Install a stable Rust toolchain with Cargo, then build and test:

```sh
cargo build --release --locked
cargo test --locked
cargo run --release --locked --bin mdlink-audit -- . --fail-on-empty
```

The integration tests run the programs Cargo builds. To run them against other builds, name those programs in
`MDLINK_AUDIT_BIN`, `MDLINK_AUDIT_ACTION_BIN` and `MDLINK_AUDIT_CORPUS_BIN`.

## Report a bug

Before reporting, read the known limitations and try the smallest case that still fails. Include:

- The exact command, selected root, and current working directory.
- The mdlink-audit version, operating system, and relevant filesystem details.
- Minimal Markdown source and destination files, preserving the filename casing and encoding involved.
- Expected behavior, actual output, and exit status.
- Relevant configuration, especially exclusions and ignored links.

For a heading problem, include the source link and exact destination heading. For a Windows casing problem, show the path as it exists on disk. For a symlink problem, state which path is a symlink and where it points. Remove secrets and private repository details from shared examples.

Use a code fence when posting deliberately broken Markdown so that the issue itself stays readable. Security-sensitive reports should follow [SECURITY.md](SECURITY.md).

## Make a change

Keep each pull request focused on one behavior or fix. Explain a concrete before-and-after example. Larger parser or route changes benefit from an issue that first agrees on the expected semantics.

Add regression tests for meaningful behavior changes. Prefer small temporary repositories that test observable results, including exit codes where appropriate. Include Windows and POSIX considerations when touching path resolution. A documentation-only change usually needs a documentation audit rather than new unit tests.

Run lint, format verification, the test suite, and the documentation checker before opening a pull request. Update the usage guide or limitations when public behavior changes. Describe which checks you ran and any checks you could not run; do not claim a platform was tested if it was not. See [release preparation](docs/releasing.md) for installed-package checks.

## Review priorities

Changes should preserve offline operation, respect the repository boundary, avoid swallowing input errors, and produce understandable diagnostics. Unsupported Markdown patterns should be documented rather than silently presented as complete compatibility.

Do not add external network checks, runtime services, or automatic edits to user documents without first defining their scope and failure behavior. The project is currently a checker, and fixes should be deliberate user actions.
