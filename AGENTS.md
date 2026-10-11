# Contributor instructions

`ssp` defines apps, variants, and experiments, and reads results. `ssp host` runs an app
and its variants behind a proxy that assigns subjects and sends events.

## Rules

- Print exactly one JSON document per command, except `ssp host`. Errors print
  `{"error": "..."}` and exit 1.
- Keep commands noninteractive, except `ssp login`.
- Print secrets only in the response that creates them.
- Require `--yes` for irreversible commands.
- Keep `src/signing.rs` byte-for-byte compatible with the backend's signed message.
- `ssp host` must never block or slow the app: drop events rather than wait, and send a
  subject to control rather than fail its request.
- Assign subjects with the same bucket function as every other Statespace client.

## Checks

```shell
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --locked
sh -n install.sh && shellcheck install.sh
```

## Commits

Use Conventional Commits, such as `feat(host): restart variants with backoff`.
