# Statespace CLI instructions

The `ssp` CLI manages accounts, SDK tokens, PostgreSQL credentials, and experiment versions. Runtime assignment and event delivery belong in the language SDKs.

## Product rules

- Keep experiments under `ssp experiment`.
- Keep SDK and CI tokens under `ssp token`.
- Keep read-only SQL credentials under `ssp database credential`.
- Create experiment definitions from YAML through the CLI.
- Make each published experiment version immutable.
- Require an explicit start command before an SDK can assign subjects.
- Print command results as JSON.
- Print secrets only when they are created.
- Keep commands noninteractive, except for browser login.

## Checks

```shell
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --locked
cargo +1.85.0 check --locked
cargo package --locked
sh -n install.sh
shellcheck install.sh
```

## Commits

- Use Conventional Commits for every commit.
- Use the format `<type>(<scope>): <description>` when a scope is useful.
- Omit the scope when it does not add useful context.
- Keep each commit focused on one change.
