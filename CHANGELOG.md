# Changelog

Notable changes to the `ssp` CLI. Releases follow semantic versioning.

## 0.2.0 - Unreleased

Experiments compare whole versions of an app instead of functions and values. It replaces
the 0.1.1 model.

### Added

- `ssp app` creates apps. Each app has its own variants, experiments, and schema of
  analysis views.
- `ssp variant create` packages a ready-to-run build reproducibly, uploads it, and signs it
  with a key from `ssp variant keygen`. `ssp variant logs` shows a variant's lifecycle
  events and captured output.
- `ssp host` runs an app's deployed code and its variants behind a proxy that assigns each
  request's subject. It verifies signatures with `--trust-key`, records outcome lines that
  processes print, and can capture output and requests with `--capture`.
- `ssp experiment` takes `--variant`, `--traffic`, `--subject`, and `--note`. Running
  experiments of an app hold disjoint shares of its subjects.
- `ssp database credential create --app` limits a credential to one app.
- `ssp app erase-subject` erases a subject's data in an app. `ssp app outcomes` lists the
  outcome names an app recorded.
- `examples/fastapi`: a FastAPI app with a CI workflow, a Dockerfile, and a systemd service.

### Removed

- `ssp group`, `ssp run`, and the function builders and their toolchains.
- `ssp experiment erase-subject`. Use `ssp app erase-subject`.
- Builds from `main`. The installer installs the latest release, and only release tags
  publish binaries.
- `STATESPACE_VERSION` in the installer. Only the latest release is published; a new
  release removes the previous one.

## 0.1.1 - 2026-10-07

The first release of the group and experiment model. It replaces the 0.1.0 commands.

### Added

- `ssp group` defines reusable, versioned sets of parameters. A parameter is a function or a
  JSON value.
- Functions build from Python, TypeScript, JavaScript, Go, Rust, C, and C++ sources, or come
  from a WebAssembly component. Builds are cached by source content.
- `ssp run` builds a function and runs it locally in the SDK sandbox.
- `ssp experiment` edits a draft of groups and weights. `start` publishes it as an immutable
  version that pins each group's newest version.
- `ssp experiment results` compares an outcome across groups with confidence intervals.
- An agent skill in `skills/statespace`.

### Removed

- `ssp function`. Functions are now parameters of groups.
