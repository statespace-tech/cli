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

Save an experiment config as `experiment.yaml`.

```yaml
name: new-ranking
description: Test reciprocal rank fusion.
assignment: user_id
eligibility: 'context.country == "US"'
groups:
  - name: treatment
    weight: 0.2
    config:
      reranker: rrf
```

Create and start it.

```shell
ssp experiment create --file experiment.yaml
ssp experiment start --name new-ranking
```

Use the SDK for your application's language to assign subjects and record outcomes.

```python
import statespace

with statespace.init("new-ranking") as run:
    config = run.get_config("u_42", context={"country": "US"})
    reranker = config.get("reranker")
    run.log({"relevance": 0.7})
```

Query the outcomes directly from the CLI:

```shell
ssp query 'SELECT group_name, count(*) FROM statespace.logs GROUP BY group_name'
```

# CLI reference

## Experiments

List experiments or inspect the latest version.

```shell
ssp experiment list
ssp experiment show --name new-ranking
```

Edit `experiment.yaml`, then publish the next immutable draft version. Starting it stops new assignments to the prior version.

```shell
ssp experiment publish --file experiment.yaml
ssp experiment start --name new-ranking --version 2
```

Stop or delete an experiment.

```shell
ssp experiment stop --name new-ranking
ssp experiment delete --name new-ranking
```

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
ssp query 'SELECT * FROM statespace.logs ORDER BY run_timestamp DESC LIMIT 20'
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
