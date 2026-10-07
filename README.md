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

Statespace runs A/B tests on the code and configuration of live software. Change a
function, a prompt, or a number, ship it to a share of real traffic, and measure what it
does to the outcomes you care about.

---

**Website:** [statespace.com](https://statespace.com) · **Documentation:** [docs.statespace.com](https://docs.statespace.com)

---

## Install

Install `ssp` on macOS or Linux.

```shell
curl -fsSL https://statespace.com/install | bash
```

Or build it from source with Cargo.

```shell
cargo install statespace-cli
```

## Quickstart

Sign in with GitHub or Google.

```shell
ssp login
```

Write a candidate change. Here it is a new ranking function in `rerank.py`.

```python
def score(items: list[int]) -> list[int]:
    return sorted(items, reverse=True)
```

Run it locally in the same sandbox your application will use.

```shell
ssp run rerank.py:score --input '[1, 3, 2]'
```

Put it in a group, together with any other values you want to test with it.

```shell
ssp group create descending ranker=rerank.py:score top_k=10
```

Create an experiment that sends 20% of eligible subjects to the group, and start it.

```shell
ssp experiment create ranking --group descending=0.2 --eligibility 'context.country == "US"'
ssp experiment start ranking
```

Assign subjects and read the group's parameters in your application. Your current code
is the default, which control receives.

```python
import statespace

experiment = statespace.experiment("ranking")
group = experiment.assign("user-42", context={"country": "US"})

# The group's ranker, or your current rerank function for control.
rank = group.function("ranker", rerank)
ranked = rank(items)[: group.value("top_k", 20)]

# Record what the user did, from this process or any other.
experiment.log("user-42", "click")
```

Compare the outcome across groups.

```shell
ssp experiment results ranking --outcome click
```

## Groups

A group is a named set of parameters. Each parameter is a function or a value.

```shell
ssp group create concise prompt=prompts/concise.md model='"claude-sonnet-5-5"' temperature=0.2
```

A function comes from source code. `ssp` builds it into a sandboxed WebAssembly component.

```shell
ssp group create fast ranker=rerank.py:score     # Python
ssp group create bm25 ranker=bm25.ts:rank        # TypeScript or JavaScript
ssp group create native ranker=./ranker:Score    # a Go module or a Rust crate
ssp group create compiled ranker=rank.cpp:rank   # C or C++
```

A value comes from a JSON file, a text file, or a JSON literal. Quote strings as JSON.

```shell
ssp group create tuned sampling=sampling.json prompt=prompt.md top_k=20 verbose=true label='"v2"'
```

Updating a group merges parameters and creates its next version. `NAME=` removes one.

```shell
ssp group update fast top_k=50 verbose=
```

Unchanged files produce no new version, because versions follow source content.

```shell
ssp group show fast
```

## Experiments

An experiment compares groups against control on the subjects its eligibility rule admits.

```shell
ssp experiment create ranking --group fast=0.1 --group bm25=0.1 --group both=0.1
```

Edits change a draft. The running version changes only when you start the draft.

```shell
ssp experiment update ranking --group fast=0.3 --group bm25=0.1
ssp experiment start ranking
```

Starting pins the newest version of each group. Subjects keep their group when weights change.

```shell
ssp experiment show ranking
```

Stop an experiment to send every subject back to your defaults.

```shell
ssp experiment stop ranking
```

Restart an earlier published version to roll back.

```shell
ssp experiment start ranking --version 3
```

Summarize an outcome, or a numeric field of it, for each group against control.

```shell
ssp experiment results ranking --outcome purchase --metric value
```

## Data

Create a read-only PostgreSQL credential and query every run and outcome with SQL.

```shell
ssp database credential create --name analysis
psql "$DATABASE_URL" -c "SELECT group_name, outcome_name, data FROM statespace.logs LIMIT 10"
```

Erase one subject's data from an experiment.

```shell
ssp experiment erase-subject ranking user-42
```

## Keys

Create a key for an application. It can read configurations and send events.

```shell
ssp key create --name production --preset runtime
```

Create a key for an agent or CI job that manages experiments.

```shell
ssp key create --name agent --preset admin --expires-in-days 30
```

Set it as `SSP_API_KEY` wherever `ssp` or an SDK runs.

```shell
export SSP_API_KEY=ssp_key_...
```

## Agents

Every command is noninteractive, except `login`, and prints one JSON document. Errors
print `{"error": "..."}` and exit with status 1.

```shell
ssp experiment results ranking --outcome click | jq '.groups[] | {group, mean, effect}'
```

This repository includes an agent skill that teaches the full loop: change a function,
test it, run an experiment, and read the result.

```shell
cp -r skills/statespace ~/.claude/skills/
```

## SDKs

Read groups in your application with the [Python](https://github.com/statespace-tech/python-sdk),
[TypeScript](https://github.com/statespace-tech/typescript-sdk), or
[Go](https://github.com/statespace-tech/go-sdk) SDK. Each assigns a subject to the same
group.

## License

Apache-2.0
