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

---

Statespace is a headless A/B testing platform that lets your coding agent quickly set up experiments for product features, and then analyze the results with SQL.

## Install

Install the release binary on macOS or Linux.

```bash
curl -fsSL https://statespace.com/install | bash
```

## Quickstart

Sign in with GitHub or Google. Statespace creates a free account on your first login.

```bash
ssp login
```

Print the activation URL when the CLI cannot open a browser.

```bash
ssp login --no-open
```

Create a globally named project after authentication.

```bash
ssp project create --name atlas-search
ssp project list
```

Show the project, its database credentials, and its experiments.

```bash
ssp project show --name atlas-search
```

```yaml
id: atlas-search
url: quack:atlas-search.db.statespace.app:443
token: st_7JmK4qN9xR2vL8cW5pT6hY3s
experiments: []
```

Show the account plan and its enforced limits.

```bash
ssp account
```

### Experiment

Define the experiment in YAML.

```yaml
name: rank-v2
description: Test reciprocal rank fusion.
assignment_unit: subject_id
groups:
  - name: treatment
    weight: 0.2
    config:
      reranker: rrf
```

Statespace adds the required `control` group with the remaining weight and an empty configuration.

Create or replace the definition while the experiment is in `draft`.

```bash
ssp experiment create --project atlas-search --file experiment.yaml
ssp experiment update --project atlas-search --file experiment.yaml
```

Inspect and control the experiment lifecycle.

```bash
ssp experiment list --project atlas-search
ssp experiment show --project atlas-search --name rank-v2
ssp experiment start --project atlas-search --name rank-v2
ssp experiment stop --project atlas-search --name rank-v2
ssp experiment delete --project atlas-search --name rank-v2
```

## Run with Python

Add the Python SDK to the application that will run the experiment.

```bash
uv add statespace-sdk
```

Run the configured experiment and record its result.

```python
from statespace import experiment

with experiment("rank-v2", subject_id="u_42") as exp:
    r = exp.get("reranker", default=None)
    search("query goes here", reranker=r)
    exp.outcome("relevance", value=0.7)
```

## SQL

Query experiment outcomes directly through DuckDB v2's standard Quack protocol.

```bash
duckdb -c "ATTACH 'quack:atlas-search.db.statespace.app:443' AS statespace (
        TOKEN 'st_7JmK4qN9xR2vL8cW5pT6hY3s'
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

Estimate the treatment effect with DuckDB regression aggregates.

```bash
duckdb -c "ATTACH 'quack:atlas-search.db.statespace.app:443' AS statespace (
        TOKEN 'st_7JmK4qN9xR2vL8cW5pT6hY3s'
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
