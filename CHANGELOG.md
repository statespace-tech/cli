# Changelog

Notable changes to the `ssp` CLI. Releases follow semantic versioning.

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
