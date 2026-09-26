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

Install the Python SDK and create an API key:

```shell
python -m pip install git+https://github.com/statespace-tech/python-sdk.git@feat/components
ssp login
ssp token create --name quickstart
export STATESPACE_TOKEN=ssp_token_...
```

Save the trend-based function as `forecast.py`:

```python
from statistics import linear_regression


def predict(history: list[float]) -> float:
    slope, intercept = linear_regression(range(len(history)), history)
    return intercept + slope * len(history)
```

Build and publish the function.

```shell
ssp function build ./forecast.py --language python --entry forecast:predict --output forecast.wasm
ssp function publish forecast.wasm --name forecast
```

Send half the traffic to the function.

```shell
ssp experiment create --name demand --variant forecast@latest=0.5
ssp experiment start --name demand
```

Save the Python app as `app.py`:

```python
from random import gauss
from statistics import fmean

import statespace

run = statespace.init("demand")
for store in range(50):
    demand = [40 + day * 0.5 + gauss(0, 3) for day in range(15)]
    subject = f"store-{store}"
    forecast = run.get_function(subject, default=fmean)(demand[:-1])
    run.log(subject, {"error": abs(forecast - demand[-1])})
run.close()
```

Run the app.

```shell
python app.py
```

Compare average forecast error by group.

```shell
ssp query "SELECT group_name,avg((data->>'error')::float) FROM statespace.logs WHERE experiment_name='demand' GROUP BY 1"
```

# CLI reference

## Experiments

List experiments or inspect the latest version.

```shell
ssp experiment list
ssp experiment show --name demand
```

Replace the full traffic split with `update`.

```shell
ssp experiment update --name demand --variant forecast@latest=0.3
```

Start the draft version when it is ready.

```shell
ssp experiment start --name demand
```

Stop or delete an experiment.

```shell
ssp experiment stop --name demand
ssp experiment delete --name demand
```

## Functions

Build a function from Python, JavaScript, TypeScript, Go, Rust, C, or C++.

```shell
ssp function build ./forecast.py --language python --entry forecast:predict --output forecast.wasm
```

Publish the function to use it in experiments.

```shell
ssp function publish forecast.wasm --name forecast
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
