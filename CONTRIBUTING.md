# Contributing

Thank you for helping build Statespace.

Open an issue before a substantial feature or a change to a public command, so we can agree
on the interface first. Small fixes can go straight to a pull request.

Do not report security issues publicly. Follow [SECURITY.md](SECURITY.md).

## Development

Install stable Rust and run the checks that CI runs.

```shell
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --locked
sh -n install.sh && shellcheck install.sh
```

Run the CLI against a local service by setting its URL.

```shell
STATESPACE_URL=http://localhost:8080 cargo run -- experiment list
```

## Pull requests

- Keep each pull request focused on one change and explain the user-visible result.
- Add or update tests and the README for public behavior.
- Add an entry to `CHANGELOG.md` for a user-visible change.
- Use [Conventional Commits](https://www.conventionalcommits.org) for commit messages.

## Releases

Bump the version in `Cargo.toml`, update `CHANGELOG.md`, and push a `v` tag. The release
workflow builds checksummed archives for each platform, publishes the GitHub release, and
publishes the crate.

```shell
git tag v0.1.1 && git push origin v0.1.1
```

By contributing, you agree that your contribution is licensed under the Apache License 2.0.
