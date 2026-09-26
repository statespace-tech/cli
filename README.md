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

[![Test Suite](https://github.com/statespace-tech/cli/actions/workflows/ci.yml/badge.svg)](https://github.com/statespace-tech/cli/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-Apache--2.0-007ec6?style=flat-square)](https://github.com/statespace-tech/cli/blob/main/LICENSE)
[![crates.io](https://img.shields.io/crates/v/statespace-cli?style=flat-square)](https://crates.io/crates/statespace-cli)
[![Discord](https://img.shields.io/discord/1541944682727084143?label=Discord&logo=discord&logoColor=white&color=5865F2&style=flat-square)](https://discord.gg/qKEFpqG9mr)

</div>

---

**Website:** [https://statespace.com](https://statespace.com/)

**Documentation:** [https://docs.statespace.com](https://docs.statespace.com/)

---

Statespace helps you A/B test any application.

## Installation

Install the Statespace CLI on macOS or Linux.

```shell
curl -fsSL https://statespace.com/install | bash
```

Install the SDK for the language used by the application you want to A/B test: [Python](https://github.com/statespace-tech/python-sdk), [TypeScript](https://github.com/statespace-tech/typescript-sdk), or [Go](https://github.com/statespace-tech/go-sdk).

## Quickstart

Log in to your [Statespace account](https://statespace.com/), then create and export your API key:

```shell
ssp login
ssp token create --name quickstart
export STATESPACE_TOKEN=ssp_token_...
```

Build a component from a function in your project. The function receives one JSON value and returns a JSON value.

```shell
ssp component build ./ranker --language python --entry ranker:score --output ranker.wasm
ssp component publish ranker.wasm --name ranker --dry-run
ssp component publish ranker.wasm --name ranker
ssp experiment create --name new-ranking --variant ranker@1=0.2
```

The remaining 80% uses your application default. Create starts the experiment.

Use the SDK to assign a subject, execute its component, and record an outcome.

```python
from statespace import Client

client = Client()
run = client.experiment("new-ranking").assign("u_42")
result = run.execute({"scores": [0.2, 0.9]}, default=lambda inputs: inputs)
run.log({"relevance": 0.7})
```

Query the outcomes directly from the CLI:

```shell
ssp query 'SELECT group_name, count(*) FROM statespace.runs GROUP BY group_name'
```

# CLI reference

## Experiments

List experiments or inspect the latest version.

```shell
ssp experiment list
ssp experiment show --name new-ranking
```

Replace the full traffic split with `update`. Each update creates an immutable internal version. A running experiment stays running. A stopped experiment stays stopped.

```shell
ssp experiment update --name new-ranking --variant ranker@latest=0.3
```

Stop or delete an experiment.

```shell
ssp experiment stop --name new-ranking
ssp experiment delete --name new-ranking
```

## Components

The CLI builds components from Python, JavaScript, TypeScript, Go, Rust, C, and C++ source. The Python, TypeScript, and Go SDKs can each run the resulting artifacts.

`build` creates a local Wasm file. It does not contact Statespace. `publish` validates and uploads that file. Published versions are immutable. Publishing the same bytes under the same name returns the existing version.

```shell
ssp component build ./ranker --language python --entry ranker:score --output ranker.wasm
ssp component build ./ranker --language javascript --entry ranker.js:score --output ranker.wasm
ssp component build ./ranker --language typescript --entry ranker.ts:score --output ranker.wasm
ssp component build ./ranker --language rust --entry score --output ranker.wasm
ssp component build ./ranker --language go --entry .:Score --output ranker.wasm
ssp component build ./ranker.c --language c --entry score --output ranker.wasm
ssp component build ./ranker.cpp --language cpp --entry score --output ranker.wasm
ssp component publish ranker.wasm --name ranker --dry-run
ssp component publish ranker.wasm --name ranker
ssp component list
ssp component show --name ranker --version 1
```

Install the matching local build tool: `componentize-py` for Python, `jco` for JavaScript and TypeScript, `cargo` and `wasm-tools` for Rust, `componentize-go` for Go, or `wit-bindgen` and WASI SDK for C and C++. Rust builds require the `wasm32-unknown-unknown` target. Set `WASI_SDK_PATH` to the WASI SDK directory, or put `wasm32-wasip2-clang` and `wasm32-wasip2-clang++` on `PATH`. The CLI accepts import-free components and a restricted WASI Preview 2 profile. It rejects network and HTTP imports.

C and C++ entries have the signature `char *score(const char *input)`. The input is JSON text. The return value must be JSON text in a buffer allocated with `malloc`; Statespace copies and frees that buffer. C++ source can use C++ code internally, but the entry uses this C-style signature. Go entries use `package:ExportedFunction`, such as `.:Score`; the function accepts one JSON-decodable value and returns a JSON-encodable value or `(value, error)`.

The same artifact runs in the Python, TypeScript, and Go SDKs. The supported WASI profile provides clocks, random data, standard streams, and empty environment and filesystem preopens. It does not provide network or HTTP access. The CLI and backend accept only the listed WASI Preview 2 interfaces at version `0.2.12`.

For local experiments, use `Client.local().component_experiment("new-ranking", {"./ranker.wasm": 1.0})` in Python. The SDK uses the file stem as the variant name and keeps events in memory.

## Tokens

Tokens authenticate SDKs and CI jobs through `STATESPACE_TOKEN`. Create a separate token for each deployment so you can revoke it independently.

```shell
ssp token create --name production
ssp token list
ssp token revoke --id tok_123
```

## PostgreSQL

Each account has an isolated PostgreSQL database. `ssp query` runs one read-only `SELECT` statement and prints a JSON array.

```shell
ssp query 'SELECT * FROM statespace.runs ORDER BY timestamp DESC LIMIT 20'
```

Create a read-only credential for a person, agent, or BI tool. The PostgreSQL connection URL appears once.

```shell
ssp database credential create --name analyst
ssp database credential list
ssp database credential revoke --id dbc_123
```

## Account

Show the authenticated account or remove the local session.

```shell
ssp account
ssp logout
```

# License

Apache-2.0
