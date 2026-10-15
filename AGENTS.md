# Contributor instructions

`ssp` defines apps, variants, and experiments. `ssp proxy` is a reverse proxy that
assigns each request's subject and forwards it to the app's default or an experiment's
arm; on Docker it also runs them as containers. `ssp controller` runs the arms on
Kubernetes.

## Rules

- Print exactly one JSON document per command, except `ssp proxy` and `ssp controller`.
  Errors print `{"error": "..."}` and exit 1.
- Change things through the CLI; read data through SQL. Add no command that only reads
  recorded data.
- Keep commands noninteractive, except `ssp login`.
- Print secrets only in the response that creates them.
- Require `--yes` for irreversible commands.
- Keep `src/signing.rs` byte-for-byte compatible with the backend's signed message.
- `ssp proxy` must never block or slow the app: drop events rather than wait, and send a
  subject to the default rather than fail its request.
- Run only what the customer approved: variants signed by a trusted key, and control arms
  whose image the proxy or controller ran as the default itself.
- Never record the default.
- Never change the bucket function: it would move subjects of running experiments.

## Checks

```shell
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --locked
sh -n install.sh && shellcheck install.sh
STATESPACE_DOCKER_TESTS=1 cargo test --locked --test cli docker   # needs Docker
```

## Commits

Use Conventional Commits, such as `feat(proxy): restart arms with backoff`.
