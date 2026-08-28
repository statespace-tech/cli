# Contributing to Statespace

Thank you for helping build Statespace.

## Before you start

Use a GitHub issue for a substantial feature or public contract change. Describe the problem, the proposed interface, and compatibility risks. Small fixes can go directly to a pull request.

Do not use public issues for security reports. Follow [SECURITY.md](SECURITY.md).

## Development

Install stable Rust.

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --locked
cargo package --locked
sh -n install.sh
shellcheck install.sh
```

The repository supports Rust 1.85 and later. CI verifies the minimum supported Rust version separately.

Read the nearest `AGENTS.md` before you change the repository. Update documentation and tests when you change a public interface.

## Pull requests

- Keep each pull request focused.
- Explain the user-visible result.
- Add or update tests.
- Preserve compatibility unless the pull request documents a migration.
- Confirm that no credential, database capability URL, assignment context, or outcome data is present.
- Add an entry to `CHANGELOG.md` for a user-visible change.

By contributing, you agree that your contribution is licensed under the MIT License.
