# Changelog

This project records notable CLI changes. Releases use semantic versioning.

## Unreleased

### Added

- Project-scoped read-only and read-write token commands.
- A logout command for saved account sessions.

### Changed

- Experiment definitions now use `assignment`.
- Public Linux releases now use portable musl binaries.
- The CLI now uses explicit HTTP timeouts and a smaller asynchronous runtime.

## 0.1.0 - 2026-08-24

### Added

- The `ssp` binary.
- Browser login with GitHub or Google.
- Stateless project commands with YAML output.
- YAML experiment creation and lifecycle commands.
- Account plan inspection and administrative plan updates.
- Direct project database credentials for DuckDB v2 and Quack.
