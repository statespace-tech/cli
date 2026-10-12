<br>

<div align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/statespace-tech/cli/main/assets/header-dark.png">
    <source media="(prefers-color-scheme: light)" srcset="https://raw.githubusercontent.com/statespace-tech/cli/main/assets/header-light.png">
    <img src="https://raw.githubusercontent.com/statespace-tech/cli/main/assets/header-light.png" alt="Statespace" width="520">
  </picture>
</div>

<div align="center">

<br>

[![CI](https://github.com/statespace-tech/cli/actions/workflows/ci.yml/badge.svg)](https://github.com/statespace-tech/cli/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-Apache--2.0-007ec6?style=flat-square)](https://github.com/statespace-tech/cli/blob/main/LICENSE)
[![crates.io](https://img.shields.io/crates/v/statespace-cli?style=flat-square)](https://crates.io/crates/statespace-cli)
[![Discord](https://img.shields.io/discord/1541944682727084143?label=Discord&logo=discord&logoColor=white&color=5865F2&style=flat-square)](https://discord.gg/qKEFpqG9mr)

</div>

Statespace A/B tests whole versions of your app in production. Build a branch, sign it,
and `ssp host` serves it to a share of real users beside the version you deployed. Then
measure what it does to the outcomes you care about.

---

**Website:** [statespace.com](https://statespace.com) · **Documentation:** [docs.statespace.com](https://docs.statespace.com)

---

## Install

Install the Statespace CLI on macOS or Linux.

```shell
curl -fsSL https://statespace.com/install | bash
```

The installer downloads the latest release and verifies its SHA-256 checksum. Only the
latest release is published. Pushing a `v*` tag publishes a release and removes the
previous one; pushes to `main` publish nothing.

## How it works

`ssp host` replaces your app's start command. It runs your deployed code, called control,
and every variant that a running experiment uses, each as its own process on a private
port. A proxy in front of them assigns each request's subject and forwards the request.

```text
load balancer → ssp host :8080 → control      (your deployed code)
                               → feature-x@3  (a signed build of a branch)
```

Your app needs no SDK and no code changes. It reports outcomes by printing a JSON line.

## Quickstart

Sign in, create an app, and create a signing key. Keep the private key in CI.

```shell
ssp login
ssp app create web
ssp variant keygen
```

Build a branch the way you build production, then package, sign, and upload it.

```shell
git checkout feature-x && uv sync --frozen
ssp variant create feature-x . --app web --sign-key "$SSP_SIGNING_KEY"
```

Create an experiment that gives the variant half of the subjects, and start it.

```shell
ssp experiment create checkout --app web --variant feature-x=0.5 \
  --note "A one-page checkout should convert better."
ssp experiment start checkout
```

Create a runtime key, and replace your start command with `ssp host`.

```shell
ssp key create --name production --preset runtime   # store it as SSP_API_KEY
ssp host --app web --command "gunicorn -b 127.0.0.1:{port} app:app" --trust-key "$PUBLIC_KEY"
```

Print an outcome line wherever something happens that you want to measure.

```python
print(json.dumps({"statespace": "purchase", "subject": user_id, "amount": 42}))
```

Compare the outcome across variants.

```shell
ssp experiment results checkout --outcome purchase --metric amount
```

## Variants

A variant is a ready-to-run build of your app: code and installed dependencies, built for
the machines it runs on. Build it in CI, like production, then upload it.

```shell
ssp variant create feature-x ./build --app web --sign-key "$SSP_SIGNING_KEY"
```

Hosts run only variants signed by a key they trust, so a leaked Statespace key cannot put
code in production. The signature covers the bundle, its name and app, its start command,
and its commit, which defaults to the directory's Git `HEAD`.

A variant starts with the host's `--command` unless it brings its own.

```shell
ssp variant create feature-x ./build --app web --command "uvicorn app.server:app --port {port}"
```

Uploading changed files creates the variant's next version. Identical files do not.

```shell
ssp variant show feature-x --app web
ssp variant logs feature-x --app web   # lifecycle events and captured output
```

## Experiments

An experiment splits its subjects between control and its variants. `--traffic` sets the
share of the app's subjects it holds while it runs. Running experiments of an app never
share a subject.

```shell
ssp experiment create checkout --app web --variant feature-x=0.5 --variant feature-y=0.25 \
  --traffic 0.2 --subject 'request.cookies.session' \
  --eligibility 'request.headers["cf-ipcountry"] == "US"'
```

`--subject` and `--eligibility` are CEL expressions over the request's `method`, `path`,
`query`, `headers`, and `cookies`. The subject defaults to `request.cookies.ssp_sid`, a
cookie the host sets for every new visitor. It cannot change once the experiment starts.

Edits change a draft. The running version changes only when you start the draft, which
pins each variant's newest version.

```shell
ssp experiment update checkout --variant feature-x=0.5 --note "feature-y regressed latency"
ssp experiment start checkout --note "drop feature-y"
```

Stop an experiment to send every subject back to control. Restart a published version to
roll back.

```shell
ssp experiment stop checkout --note "feature-x won; merging it"
ssp experiment start checkout --version 1
```

## Hosts

```shell
ssp host --app web \
  --listen 0.0.0.0:8080 \
  --command "gunicorn -b 127.0.0.1:{port} app:app" \
  --health /health \
  --trust-key "$PUBLIC_KEY" \
  --variant-env DATABASE_URL="$VARIANT_DATABASE_URL" \
  --capture stdout --capture stderr --capture http \
  --redact 'sk_live_\w+'
```

| Flag | Meaning |
| --- | --- |
| `--command` | How to start the app, with `{port}` for the port it must listen on at 127.0.0.1 |
| `--health` | A path that must answer 200 before a process gets traffic |
| `--trust-key` | A public key whose signed variants may run. Repeat to rotate keys |
| `--max-variants` | How many variants run at once, besides control. Defaults to 4 |
| `--variant-env` | An environment variable that variants get and control does not |
| `--capture` | Also send `stdout`, `stderr`, or one row per `http` request |
| `--redact` | A regular expression replaced in captured output before it leaves the machine |
| `--stale-after` | Seconds to keep routing while Statespace is unreachable. Defaults to 300 |

Processes get `PORT`, `STATESPACE_APP`, `STATESPACE_VARIANT`, and
`STATESPACE_VARIANT_VERSION`. Requests carry `x-statespace-subject`,
`x-statespace-variant`, and `x-statespace-experiment`; clients cannot set them.

A variant that is starting or failing sends its subjects to control. Hosts always send
lifecycle events, CPU and memory, and per-minute HTTP summaries for each process.

Variants share your database, cache, and queues with control. Give them a database user
without schema permissions with `--variant-env`, and turn off schedulers and workers that
run inside your web process, so nothing runs twice.

Try variants locally, unsigned, choosing one per request with a header.

```shell
ssp host --local --command "python3 app.py {port}" --variant feature-x=../feature-x
curl -H 'x-statespace-variant: feature-x' localhost:8080
```

[`examples/fastapi`](examples/fastapi) is a complete app: a FastAPI shop with a GitHub
Actions workflow that publishes each branch as a variant, a Dockerfile, and a systemd
service.

## Data

Every app has a PostgreSQL schema of views: `logs`, `runs`, `outcomes`, `requests`,
`output`, `metrics`, `lifecycle`, and `notes`. Create a read-only credential for one app
or for all of them.

```shell
ssp database credential create --name analysis --app web
psql "$DATABASE_URL" -c "SELECT variant, outcome_name, data FROM web.logs LIMIT 10"
```

Erase one subject's data from an app.

```shell
ssp app erase-subject web user-42
```

## Keys

Create a key for `ssp host`. It can read configurations and send events.

```shell
ssp key create --name production --preset runtime
```

Create a key for CI that publishes variants, or for an agent that also runs experiments.

```shell
ssp key create --name ci --scope experiments:write
ssp key create --name agent --preset admin --expires-in-days 30
```

## Agents

Every command except `host` prints one JSON document, and every command except `login` is
noninteractive. Errors print `{"error": "..."}` and exit with status 1.

```shell
ssp experiment results checkout --outcome purchase | jq '.variants[] | {variant, mean, effect}'
```

This repository includes an agent skill that teaches the full loop: build a branch,
publish it as a variant, run an experiment, and read the result.

```shell
cp -r skills/statespace ~/.claude/skills/
```

## License

Apache-2.0
