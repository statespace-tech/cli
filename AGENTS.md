# Statespace agent instructions

Statespace is a headless A/B testing platform. Use the `ssp` CLI to configure projects and experiments. Use a Statespace SDK to assign subjects and record outcomes at runtime. Query results directly with DuckDB.

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

Show the authenticated account and its enforced plan limits.

```bash
ssp account
```

Remove the saved account session.

```bash
ssp logout
```

The CLI stores the account session locally. Do not copy this account credential into application code.

## Manage projects

A project is the isolation boundary for experiments, tokens, and one queryable DuckDB database. Project names are globally unique.

Create a project.

```bash
ssp project create --name atlas-search
```

The command returns the database URL and one default read-write token. The token secret appears once.

List the projects owned by the account.

```bash
ssp project list
```

Show one project, its active token metadata, and its experiments.

```bash
ssp project show --name atlas-search
```

The CLI is stateless. Always identify a project with `--name` or `--project` as required by the command.

## Manage project tokens

Project tokens are capabilities that can be shared with applications, people, or agents.

- A `read-only` token can query the project database.
- A `read-write` token can query the database, assign subjects, and record outcomes.
- Neither token can change experiment definitions or manage other tokens.

Create a read-write token for an application.

```bash
ssp project token create \
  --project atlas-search \
  --name production \
  --access read-write
```

Create a read-only token for an analyst or coding agent.

```bash
ssp project token create \
  --project atlas-search \
  --name analyst \
  --access read-only
```

List active tokens. This command shows token IDs and prefixes, but it does not show token secrets.

```bash
ssp project token list --project atlas-search
```

Revoke a shared token by ID.

```bash
ssp project token revoke --project atlas-search --id tok_123
```

Store each token secret in a secret manager or environment variable. Do not put it in source control, command history, logs, or URLs. Create separate tokens for separate applications and recipients so that each token can be revoked independently.

## Define experiments

Put the complete experiment definition in a YAML file.

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

The control group is added automatically when the file does not contain one. It receives the remaining weight and an empty configuration.

Create an experiment.

```bash
ssp experiment create --project atlas-search --file experiment.yaml
```

Replace a draft definition atomically. A stopped experiment can change its description and group configuration, but not its assignment, group names, or weights.

```bash
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

Stop an experiment before you change its configuration. Do not create or edit individual groups through separate commands.

## Use an SDK at runtime

Use a read-write project token in the runtime application.

```bash
export STATESPACE_TOKEN=ss_rw_...
```

For Python, use [`statespace-tech/python-sdk`](https://github.com/statespace-tech/python-sdk). Connect to an experiment, create a run for each exposure, read `run.config`, and record results with `run.outcome(...)`. Use `Project.query(...)` for SQL.

For TypeScript, use [`statespace-tech/ts-sdk`](https://github.com/statespace-tech/ts-sdk). Follow that repository for its runtime and query APIs.

Do not define groups in application code. Read the assigned configuration from the SDK and provide an explicit default for each value.

## Query results

Use DuckDB 2.0 or later to connect through the standard Quack protocol. Use either a read-only or read-write project token.

```sql
ATTACH 'quack:atlas-search.db.statespace.app:443' AS statespace (
  TOKEN 'ss_ro_...'
);

SELECT
  group_name,
  count(*) AS samples,
  avg(value) AS mean_relevance
FROM statespace.outcomes
WHERE experiment_name = 'rank-v2'
  AND name = 'relevance'
GROUP BY group_name;
```

The remote project database is read-only. A read-write token writes assignments and outcomes through the Statespace API, not through arbitrary SQL statements.

## Output and errors

The CLI prints requested resources as YAML on standard output. It prints diagnostics on standard error and returns a nonzero exit status for validation or server errors.

Do not parse undocumented human-readable diagnostics. Depend only on documented YAML fields.

## Repository maintenance

Keep commands under `ssp project` and `ssp experiment`. Require an explicit project for project-scoped resources. Keep internal administration hidden from public help. Keep commands noninteractive unless the command explicitly requests an interactive mode. Never print a stored token secret.

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
