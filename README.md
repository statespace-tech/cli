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

Statespace A/B tests whole versions of your app in production. Publish a branch's image,
and `ssp proxy` serves it to a share of real users beside a control arm of the version you
deployed. Then query what it did to the outcomes you care about.

---

**Website:** [statespace.com](https://statespace.com) · **Documentation:** [docs.statespace.com](https://docs.statespace.com)

---

## Install

Install the Statespace CLI on macOS or Linux.

```shell
curl -fsSL https://statespace.com/install.sh | bash
```

The installer downloads the latest release and verifies its SHA-256 checksum. Only the
latest release is published. Pushing a `v*` tag publishes a release, the
`ghcr.io/statespace-tech/ssp` image, and removes the previous release; pushes to `main`
publish nothing.

## How it works

`ssp proxy` is a reverse proxy in front of your app. For each request it reads the
subject, assigns it, and forwards the request to one of:

- **default**: the version you deployed, for every subject outside an experiment;
- **control**: an experiment's baseline arm, a copy of the default's image pinned when the
  experiment started;
- **a variant**: a signed image of a branch.

```text
load balancer → ssp proxy :8080 → default            (your deployed image)
                                → control of price   (the default's image, pinned)
                                → lower-price@3      (a signed image of a branch)
```

On a machine with Docker, the proxy runs the default and every arm as containers. On
Kubernetes, it runs beside the default in each pod, and `ssp controller` runs the arms as
copies of the default's Deployment.

Your app reads the `x-statespace-subject` header when it needs the subject, and prints
what it does to standard output or standard error, as it already does. Every line the arms
print is recorded; a JSON line with a `subject` field is tied to that subject's arm.

## Quickstart

Sign in, create an app, and create a signing key. Keep the private key in CI.

```shell
ssp login
ssp app create web
ssp keygen
```

Build the branch's image the way you build production, push it, then sign and publish it.

```shell
docker build -t ghcr.io/acme/web:feature-x . && docker push ghcr.io/acme/web:feature-x
ssp variant create feature-x --app web --image ghcr.io/acme/web:feature-x
```

Create an experiment that gives the variant 5% of the subjects and control another 5%,
and start it.

```shell
ssp experiment create checkout --app web --split feature-x=0.05 \
  --note "A one-page checkout should convert better."
ssp experiment start checkout
```

Create a runtime token, and run your app behind `ssp proxy`.

```shell
ssp token create --name production --preset runtime   # store it as SSP_TOKEN
ssp proxy --app web --runtime docker --image ghcr.io/acme/web:main --trust-key "$PUBLIC_KEY"
```

Print a line wherever something happens that you want to measure.

```python
print(json.dumps({"event": "purchase", "subject": user_id, "amount": 42}))
```

Compare the arms in SQL with a token that may read the app.

```shell
ssp token create --name analysis --preset analyst --app web
psql "$DATABASE_URL" -c "
  SELECT variant, count(DISTINCT subject_id) AS subjects,
         count(DISTINCT subject_id) FILTER (WHERE data->>'event' = 'purchase') AS buyers
  FROM web.logs WHERE experiment = 'checkout' GROUP BY variant"
```

## Variants

A variant is a signed image of your app, usually built from a branch. `--image` takes a tag
or a digest; a tag is resolved to its digest with `docker`, so the signature covers exact
bytes.

```shell
ssp variant create feature-x --app web --image ghcr.io/acme/web:feature-x
```

Proxies and controllers run only variants signed by a key they trust, so a leaked
Statespace token cannot put code in production. The signature covers the image digest, the
variant's name and app, and its commit, which defaults to CI's commit, such as
`GITHUB_SHA`, or the current directory's Git `HEAD`.

Publishing a new digest creates the variant's next version. The same digest does not.

```shell
ssp variant show feature-x --app web
```

## Experiments

`--split` gives each arm its share of all the app's subjects. Control gets the largest
variant's share unless `control=SHARE` sets it. Running experiments of an app never share a
subject; subjects outside every experiment get the default.

```shell
ssp experiment create checkout --app web --split feature-x=0.05 --split feature-y=0.02 \
  --subject 'request.cookies.session' \
  --eligibility 'request.headers["cf-ipcountry"] == "US"'
```

`--subject` and `--eligibility` are CEL expressions over the request's `method`, `path`,
`query`, `headers`, and `cookies`. The subject defaults to `request.cookies.ssp_sid`, a
cookie the proxy sets for every new visitor. It cannot change once the experiment starts.

Edits change a draft. The running version changes only when you start the draft, which
pins each variant's newest version and gives the version a fresh control arm.

```shell
ssp experiment update checkout --split feature-x=0.05 --note "feature-y regressed latency"
ssp experiment start checkout --note "drop feature-y"
```

Stop an experiment to send every subject back to the default, and ship a winner by merging
its branch and deploying it as usual. Restart a published version to roll back.

```shell
ssp experiment stop checkout --note "feature-x won; merging it"
ssp experiment start checkout --version 1
```

## Proxies

On a machine with Docker:

```shell
ssp proxy --app web --runtime docker \
  --listen 0.0.0.0:8080 \
  --image-file /etc/web/image \
  --health /health \
  --trust-key "$PUBLIC_KEY" \
  --variant-env DATABASE_URL="$VARIANT_DATABASE_URL" \
  --redact 'sk_live_\w+'
```

On Kubernetes, beside the default in each pod:

```shell
ssp proxy --app web --runtime kubernetes --default http://localhost:8000
```

| Flag | Meaning |
| --- | --- |
| `--runtime` | `docker` runs the default and the arms; `kubernetes` only routes |
| `--image`, `--image-file` | The default's image. On SIGHUP, the proxy reads `--image-file` again and swaps the default once the new container is ready |
| `--default` | Where the default listens, with `--runtime kubernetes` |
| `--health` | A path that must answer 200 before a container gets traffic |
| `--trust-key` | A public key whose signed variants may run. Repeat to rotate keys |
| `--max-variants` | How many arms run at once, control arms included. Defaults to 4 |
| `--variant-env` | An environment variable every arm gets and the default does not |
| `--redact` | A regular expression replaced in recorded lines before they leave the machine |
| `--port`, `--volume`, `--memory`, `--cpus` | Container settings, with `--runtime docker`. The port defaults to the image's `EXPOSE` |
| `--stale-after` | Seconds to keep routing while Statespace is unreachable. Defaults to 300 |

On Docker, containers inherit the proxy's environment, without its own settings and what
describes the machine, and get `PORT`, `STATESPACE_APP`, `STATESPACE_VARIANT`,
`STATESPACE_EXPERIMENT`, and `STATESPACE_VARIANT_VERSION`. Requests carry
`x-statespace-subject`, `x-statespace-variant`, and `x-statespace-experiment`; clients
cannot set them.

An arm that is starting or failing sends its subjects to the default. A control arm runs
only an image the proxy ran as its default, so Statespace cannot choose what runs. Arms
record every line they print, every request, lifecycle events, CPU and memory, and
per-minute HTTP summaries. The default is never recorded.

Arms share your database, cache, and queues with the default. Give them a database user
without schema permissions with `--variant-env`, and turn off schedulers and workers that
run inside your web process, so nothing runs twice.

## Kubernetes

`ssp controller` runs the arms of an app's experiments as copies of the default's
Deployment, with the arm's image. It reports where each arm runs, so the proxies beside the
default reach it, and records every line the arms print.

```shell
ssp controller --app web --deployment web --trust-key "$PUBLIC_KEY"
```

[`examples/kubernetes`](examples/kubernetes) has the manifests: the sidecar, the
controller, and what the controller may do. [`examples/fastapi`](examples/fastapi) is a
complete app: a FastAPI shop with a GitHub Actions workflow that publishes each branch's
image as a variant, and a systemd unit for a machine with Docker.

## Data

Every app has a PostgreSQL schema of views: `logs`, `runs`, `output`, `requests`,
`metrics`, `lifecycle`, `notes`, `hosts`, `arms`, and `sample_ratio`. Tokens that may read
every app also read `account.audit`.

```shell
ssp token create --name analysis --preset analyst --app web
psql "$DATABASE_URL" -c "SELECT * FROM web.sample_ratio WHERE mismatch"
```

Erase one subject's data from an app.

```shell
ssp app erase-subject web user-42
```

## Tokens

One token opens the API and, with `query:read`, PostgreSQL. Presets cover the usual jobs.

| Preset | For |
| --- | --- |
| `runtime` | `ssp proxy`: read the app's configuration and send events |
| `controller` | `ssp controller`: runtime, and report where arms run |
| `publisher` | CI: publish variants and run experiments |
| `analyst` | SQL: read runs, output, and requests |
| `admin` | Everything, for agents that run the whole loop |

```shell
ssp token create --name agent --preset admin --expires-in-days 30
ssp token list          # tokens and sign-in sessions
ssp whoami              # the credential in use and what it may do
ssp logout --all        # end every session
```

## Agents

Every command except `proxy` and `controller` prints one JSON document, and every command
except `login` is noninteractive. Errors print `{"error": "..."}` and exit with status 1.
The CLI changes things; SQL reads data.

This repository includes an agent skill that teaches the full loop: build a branch,
publish it as a variant, run an experiment, and read the result.

```shell
cp -r skills/statespace ~/.claude/skills/
```

## License

Apache-2.0
