# Changelog

This repository records notable CLI changes. Releases use semantic versioning.

## Unreleased

### Added

- Read-only and read-write database token commands.
- A logout command for saved account sessions.
- Experiment start, stop, traffic, layer, and deletion commands.

### Changed

- Experiment definitions now use `assignment`.
- OAuth sign-up now assigns one immutable account database automatically.
- Short options now use lowercase letters.
- Public Linux releases now use portable musl binaries.
- The CLI now uses explicit HTTP timeouts and a smaller asynchronous runtime.

### Removed

- User-managed project and entity commands.
- CLI commands that create or update experiment definitions.
- The `STATESPACE_ENTITY` setting.

## 0.1.0 - 2026-08-24

### Added

- The `ssp` binary.
- Browser login with GitHub or Google.
- Structured YAML output.
- Experiment lifecycle commands.
- Account plan inspection and administrative plan updates.
- Direct account database credentials for DuckDB v2 and Quack.
