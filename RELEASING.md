# Releasing Statespace

## Prerequisites

- The repository is public at `statespace-tech/cli`.
- The `main` branch passes CI.
- GitHub private vulnerability reporting is enabled.

## Release

1. Update the version in `Cargo.toml`.
2. Move relevant entries from `Unreleased` to the new version in `CHANGELOG.md`.
3. Run all checks from `AGENTS.md`.
4. Commit the release changes.
5. Create and push a signed `vMAJOR.MINOR.PATCH` tag.

The release workflow builds binary archives for macOS and Linux on x86_64 and arm64. Linux archives use musl for portable static binaries. The workflow publishes a SHA-256 sidecar file with each archive.

After publication, test a clean CLI installation:

```bash
curl -fsSL https://statespace.com/install | bash
ssp --version
```

Check the source archive, license file, release notes, and checksums before announcing the release.
