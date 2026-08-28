# Statespace agent instructions

Statespace is a headless A/B testing platform. Use the `ssp` CLI to inspect the account and operate experiments. Use a Statespace SDK to create experiments, assign subjects, and record outcomes at runtime. Query results directly with DuckDB.

## Install the CLI

Install the current macOS or Linux release.

```bash
curl -fsSL https://statespace.com/install | bash
```

The installed command is `ssp`.

## Authenticate

Sign in with GitHub or Google. The first login creates a free account.

```bash
ssp login
```

Use `--no-open` when the environment cannot open a browser.

```bash
ssp login --no-open
```

Show the authenticated account, enforced plan limits, usage, and DuckDB URL.

```bash
ssp account
```

Remove the saved account session.

```bash
ssp logout
```

The CLI stores the account session locally. Do not copy this account credential into application code.

Each account has one database. Users do not create, list, select, or configure account databases.

## Manage database tokens

Database tokens are capabilities that can be shared with applications, people, or agents.

- A `read-only` token can query the account database.
- A `read-write` token can query the database, assign subjects, and record outcomes.
- Neither token can change experiment definitions or manage other tokens.

Create a read-write token for an application.

```bash
ssp token create -n production --access read-write
```

Create a read-only token for an analyst or coding agent.

```bash
ssp token create -n analyst --access read-only
```

List active tokens. This command shows token IDs and prefixes, but it does not show token secrets.

```bash
ssp token list
```

Revoke a shared token by ID.

```bash
ssp token revoke --id tok_123
```

Store each token secret in a secret manager or environment variable. Do not put it in source control, command history, logs, or URLs. Create separate tokens for separate applications and recipients so that each token can be revoked independently.

## Run experiments

Put the complete experiment definition in a YAML file for the runtime SDK.

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

The control group is added automatically when the definition does not contain one. It receives the remaining weight and an empty configuration.

Use the Python or TypeScript SDK to load the definition and pass it to `statespace.init(...)`. The SDK creates and starts an absent experiment at runtime.

Inspect and control experiments created by an SDK.

```bash
ssp experiment list
ssp experiment show -n rank-v2
ssp experiment start -n rank-v2
ssp experiment stop -n rank-v2
ssp experiment delete -n rank-v2
```

Do not create or edit experiment definitions through the CLI.

## Control assignment

Set initial traffic in the experiment file. Use stratified assignment when each completed block must contain the configured group proportions.

```yaml
name: checkout-v2
description: Test a compact checkout flow.
assignment:
  unit: account_id
  method: stratified
  block_size: 10
  strata:
    - country
traffic: 0.10
layer: checkout
groups:
  - name: treatment
    weight: 0.30
    config:
      checkout: compact
```

Each group weight multiplied by `block_size` must be a whole number. The runtime must provide every declared stratum when it requests an assignment.

Increase traffic for a running experiment. Traffic cannot decrease after the experiment starts.

```bash
ssp experiment traffic set \
  -n checkout-v2 \
  --traffic 0.25
```

Create an exclusion layer before you create experiments that use it.

```yaml
name: checkout
description: Keep checkout experiments mutually exclusive.
assignment: account_id
holdout: 0.05
```

Manage exclusion layers through the experiment command.

```bash
ssp experiment layer create --file layer.yaml
ssp experiment layer list
ssp experiment layer show -n checkout
ssp experiment layer update --file layer.yaml
ssp experiment layer delete -n checkout
```

The sum of running experiment traffic in a layer cannot exceed `1 - holdout`. An assignment value can enter only one experiment in the layer.

## Use an SDK at runtime

Use a read-write database token in the runtime application.

```bash
export STATESPACE_TOKEN=ssp_rw_...
```

For Python, use [`statespace-tech/python-sdk`](https://github.com/statespace-tech/python-sdk). Load or define an experiment, pass it to `statespace.init(...)`, assign subjects with `run.get_config(...)`, and record outcomes with `run.log(...)`.

For TypeScript, use [`statespace-tech/ts-sdk`](https://github.com/statespace-tech/ts-sdk). Follow that repository for its runtime API.

You can define groups in YAML or with the public `Experiment` and `Group` models. Read the assigned configuration from the SDK and provide an explicit default for each value.

## Query results

Use DuckDB 2.0 or later directly through the standard Quack protocol. Use either a read-only or read-write database token.

```sql
ATTACH 'quack:atlas-search.db.statespace.app:443' AS statespace (
  TOKEN 'ssp_ro_...'
);

SELECT
  group_name,
  count(*) AS samples,
  avg(data.relevance::DOUBLE) AS mean_relevance
FROM statespace.logs
WHERE experiment_name = 'rank-v2'
GROUP BY group_name;
```

The remote account database is read-only. A read-write token writes assignments and outcomes through the Statespace API, not through arbitrary SQL statements.

## Output and errors

The CLI prints requested resources as YAML on standard output. It prints diagnostics on standard error and returns a nonzero exit status for validation or server errors.

Do not parse undocumented human-readable diagnostics. Depend only on documented YAML fields.

## Repository maintenance

Keep commands under `ssp token` and `ssp experiment`. Keep `ssp account` as a read-only command with no subcommands. Keep internal administration hidden from public help. Keep commands noninteractive unless the command explicitly requests an interactive mode. Never print a stored token secret.

Run these checks before a release:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --locked
cargo +1.85.0 check --locked
cargo package --locked
sh -n install.sh
shellcheck install.sh
```
