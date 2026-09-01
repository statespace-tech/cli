# Releasing Statespace

## Prerequisites

- The repository is public at `statespace-tech/cli`.
- The `main` branch passes CI.
- GitHub private vulnerability reporting is enabled.
- The `statespace-cli` crate exists on crates.io.
- Its trusted publisher uses `statespace-tech/cli` and `.github/workflows/release.yml`.

## Release

1. Update the version in `Cargo.toml`.
2. Move relevant entries from `Unreleased` to the new version in `CHANGELOG.md`.
3. Run all checks from `AGENTS.md`.
4. Run `cargo publish --locked --dry-run`.
5. Commit and push the release changes.
6. Wait for CI to pass.
7. Create and push a signed `vMAJOR.MINOR.PATCH` tag.

The tag must match the version in `Cargo.toml`. The release workflow publishes the `ssp` binary to crates.io and builds binary archives for macOS and Linux on x86_64 and arm64. Linux archives use musl for portable static binaries. The workflow publishes a SHA-256 sidecar file with each archive and includes `install.sh` in the release. A workflow rerun skips a crate version that already exists.

For the first release only, run `cargo publish --locked` before you create the tag. Then configure the crate's trusted publisher on crates.io with the repository and workflow above. Future tags publish both the crate and the GitHub release.

After publication, test a clean CLI installation:

```bash
curl -fsSL https://statespace.com/install | bash
ssp --version
```

Check the source archive, license file, release notes, and checksums before announcing the release.
