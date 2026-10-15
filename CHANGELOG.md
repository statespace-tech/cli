# Changelog

Notable changes to the `ssp` CLI. Releases follow semantic versioning.

## 0.2.0 - 2026-10-15

Experiments compare whole versions of an app instead of functions and values. A variant is
a signed image; each experiment compares it with a control arm of the deployed image. It
replaces the 0.1.1 model.

### Added

- `ssp app` creates apps. Each app has its own variants, experiments, and schema of
  analysis views.
- `ssp variant create --image` resolves an image's tag to its digest with `docker` and
  signs the digest, app, name, and commit with a key from `ssp keygen`.
- `ssp experiment` takes `--split ARM=SHARE`, each arm's share of all the app's subjects.
  Control gets the largest variant's share unless `control=SHARE` sets it. Running
  experiments of an app hold disjoint shares of its subjects.
- `ssp proxy`, a reverse proxy that assigns each request's subject and forwards it to the
  app's default, an experiment's control arm, or a variant. With `--runtime docker`, it
  runs the default and every arm as containers, adopts them after a crash, and swaps the
  default on SIGHUP without dropping requests. With `--runtime kubernetes`, it runs beside
  the default in each pod.
- `ssp controller` runs each arm on Kubernetes as a copy of the default's Deployment with
  the arm's image, reports where it runs, and records the lines its pods print.
- Arms record every line they print, every request, lifecycle events, and per-minute
  metrics. The default is never recorded.
- `ssp token` creates, lists, and revokes tokens and sessions. One token opens the API and,
  with `query:read`, PostgreSQL. Presets: `runtime`, `controller`, `publisher`, `analyst`,
  and `admin`.
- `ssp whoami`, `ssp logout --all`, and `ssp keygen`.
- `ssp app erase-subject` erases a subject's data in an app.
- The `ghcr.io/statespace-tech/ssp` image, for the proxy sidecar and the controller.
- `examples/fastapi`, a FastAPI shop with a CI workflow and a systemd unit, and
  `examples/kubernetes`, its manifests for Kubernetes.

### Removed

- `ssp group`, `ssp run`, and the function builders and their toolchains.
- `ssp experiment results`, `ssp variant logs`, `ssp app hosts`, `ssp app outcomes`, and
  `ssp audit`. Query the app's views and `account.audit` in SQL.
- `ssp key`, `ssp database credential`, `ssp account`, and `ssp storage`. Use `ssp token`
  and `ssp whoami`.
- `SSP_API_KEY`. Use `SSP_TOKEN`.
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
