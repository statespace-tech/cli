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

Statespace is a headless A/B testing platform for coding agents. Define product experiments and analyze the results with SQL.

## Install

Install the release binary on macOS or Linux.

```bash
curl -fsSL https://statespace.com/install | bash
```

## Quickstart

### Create it

Create a project with a globally unique name. The command returns a default read-write token once.

```bash
ssp project create --name atlas-search
```

Save the experiment definition as `experiment.yaml`. The control group is added automatically.

```yaml
name: rank-v2
description: Test reciprocal rank fusion.
assignment: subject_id
groups:
  - name: treatment
    weight: 0.2
    config:
      reranker: rrf
```

Create and start the experiment.

```bash
ssp experiment create --project atlas-search --file experiment.yaml
ssp experiment start --project atlas-search --name rank-v2
```

### Run it

Install the [Python SDK](https://github.com/statespace-tech/python-sdk) or [TypeScript SDK](https://github.com/statespace-tech/ts-sdk).

```bash
# Python
uv add statespace-sdk

# TypeScript
npm install @statespace/sdk
```

Run the configured experiment and record its result.

```python
from statespace import Experiment

experiment = Experiment.connect(
    "atlas-search/rank-v2",
    token="ssp_rw_7j...",
)

samples = [
    ("u_42", "query goes here"),
    ("u_73", "another query goes here"),
    ("u_91", "final query goes here"),
]

for subject_id, query in samples:
    run = experiment.run(subject_id)
    reranker = run.config.get("reranker", None)
    search(query, reranker=reranker)
    run.outcome("relevance", value=0.7)

experiment.close()
```

### Analyze it

Use your coding agent to query the experiment through DuckDB v2.0 and the [Quack protocol](https://duckdb.org/docs/current/quack/overview).

```bash
duckdb -c "ATTACH 'quack:atlas-search.db.statespace.app:443' AS statespace (
        TOKEN 'ssp_rw_7j...'
      );
      SELECT
        group_name,
        count(*) AS samples,
        avg(value) AS mean_relevance
      FROM statespace.outcomes
      WHERE experiment_name = 'rank-v2'
        AND name = 'relevance'
      GROUP BY group_name"
```

Estimate the treatment effect with DuckDB [statistical aggregate functions](https://duckdb.org/docs/lts/sql/functions/aggregates).

```bash
duckdb -c "ATTACH 'quack:atlas-search.db.statespace.app:443' AS statespace (
        TOKEN 'ssp_rw_7j...'
      );
      WITH results AS (
        SELECT
          value,
          CASE WHEN group_name = 'treatment' THEN 1.0 ELSE 0.0 END AS treatment
        FROM statespace.outcomes
        WHERE experiment_name = 'rank-v2'
          AND name = 'relevance'
      )
      SELECT
        count(*) AS observations,
        regr_intercept(value, treatment) AS control_relevance,
        regr_slope(value, treatment) AS treatment_effect,
        regr_r2(value, treatment) AS explained_variance
      FROM results"
```

## License

MIT
