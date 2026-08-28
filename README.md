<br>

<div align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="./assets/header-dark.png">
    <source media="(prefers-color-scheme: light)" srcset="./assets/header-light.png">
    <img src="./assets/header-light.png" alt="Statespace" width="420">
  </picture>
</div>

<div align="center">

<br>

[![License](https://img.shields.io/badge/license-MIT-007ec6?style=flat-square)](https://github.com/statespace-tech/cli/blob/main/LICENSE)

</div>

---

**Website: [https://statespace.com](https://statespace.com/)**

**Docs for agents:** [AGENTS.md](https://github.com/statespace-tech/cli/blob/main/AGENTS.md)

---

Statespace is a headless A/B testing platform for coding agents. Define product experiments and analyze their results with SQL.

## Install

Install the release binary on macOS or Linux.

```bash
curl -fsSL https://statespace.com/install | bash
```

## View the account

Show the signed-in account, plan limits, usage, and DuckDB URL.

```bash
ssp account
```

Sign-up creates one account database. The CLI and API select it automatically.

## Manage tokens

Create separate tokens for applications and analysts. A read-write token can assign subjects and record logs, while a read-only token can only query the database.

```bash
ssp token create -n production --access read-write
ssp token create -n analyst --access read-only
```

List token metadata without revealing token secrets, or revoke one token by ID.

```bash
ssp token list
ssp token revoke --id tok_123
```

## Run an experiment

Install the [Python SDK](https://github.com/statespace-tech/python-sdk). The SDK creates an absent experiment and rejects a conflicting definition with the same name.

```bash
uv add statespace-sdk
```

Define the treatment in application configuration. The SDK adds an empty control group with the remaining weight.

```yaml
name: rank-v2
description: Test reciprocal rank fusion.
assignment: user_id
groups:
  - name: treatment
    weight: 0.2
    config:
      reranker: rrf
```

Run the experiment and record one arbitrary JSON document for each observation.

```python
import statespace

experiment = statespace.load("experiment.yaml")

with statespace.init(**experiment) as run:
    reranker = run.get_config("u_42").get("reranker", None)
    run.log({"relevance": 0.7})
```

## Operate experiments

Inspect experiments created by the SDK.

```bash
ssp experiment list
ssp experiment show -n rank-v2
```

Increase live traffic, stop collection, or delete an experiment.

```bash
ssp experiment traffic set -n rank-v2 --traffic 0.5
ssp experiment stop -n rank-v2
ssp experiment start -n rank-v2
ssp experiment delete -n rank-v2
```

## Query results

Use DuckDB 2.0 or later directly through the [Quack protocol](https://duckdb.org/docs/current/quack/overview).

```bash
duckdb -c "ATTACH 'quack:acme.db.statespace.app:443' AS statespace (
        TOKEN 'ssp_ro_7j...'
      );
      SELECT
        group_name,
        count(*) AS samples,
        avg(data.relevance::DOUBLE) AS relevance
      FROM statespace.logs
      WHERE experiment_name = 'rank-v2'
      GROUP BY group_name"
```

Use DuckDB statistical aggregates to estimate the treatment effect.

```bash
duckdb -c "ATTACH 'quack:acme.db.statespace.app:443' AS statespace (
        TOKEN 'ssp_ro_7j...'
      );
      WITH results AS (
        SELECT
          data.relevance::DOUBLE AS relevance,
          CASE WHEN group_name = 'treatment' THEN 1.0 ELSE 0.0 END AS treatment
        FROM statespace.logs
        WHERE experiment_name = 'rank-v2'
      )
      SELECT
        count(*) AS observations,
        regr_intercept(relevance, treatment) AS control_relevance,
        regr_slope(relevance, treatment) AS treatment_effect,
        regr_r2(relevance, treatment) AS explained_variance
      FROM results"
```

## License

MIT
