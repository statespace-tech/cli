# Contributor instructions

`ssp` defines groups and experiments, builds functions, and reads results. Runtime
assignment and event delivery belong in the SDKs.

## Rules

- Print exactly one JSON document per command. Errors print `{"error": "..."}` and exit 1.
- Keep commands noninteractive, except `ssp login`.
- Print secrets only in the response that creates them.
- Require `--yes` for irreversible commands.
- Resolve every `NAME=VALUE` with the rules in `src/params.rs`. Never turn an unrecognized
  value into a string.
- Pin build tools and verify their checksums in `src/build/toolchain.rs`.

## Checks

```shell
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --locked
sh -n install.sh && shellcheck install.sh
```

## Commits

Use Conventional Commits, such as `feat(group): merge parameters on update`.
