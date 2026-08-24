# CLI instructions

## Responsibilities

This project owns the `ssp` command. The CLI manages accounts, projects, and experiments. It does not execute customer SQL.

The CLI is a primary interface for the headless company. Every normal workflow must work without a browser. Commands must work for humans, shell scripts, and coding agents.

## Rules

- Keep project operations under `ssp project`.
- Require `--project` for each experiment command. Never store an active project.
- Print command results as YAML.
- Require `--name` for named projects and experiment lifecycle commands.
- Read complete experiment definitions from YAML files.
- Replace draft experiment definitions atomically.
- Do not mutate individual experiment groups.
- Emit readable YAML for resource commands.
- Never print a saved API key.
- Return nonzero status for server and validation errors.
- Keep commands scriptable and noninteractive.
- Preserve environment-variable overrides.
- Keep macOS and Linux release archives compatible with `install.sh`.
- Keep credentials out of stdout, logs, process diagnostics, and error messages.
- Use stdout for requested data. Use stderr for diagnostics.
- Avoid prompts unless a command explicitly requests an interactive mode.
- Document stable output before another tool depends on it.

## Checks

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --locked
```
