---
name: statespace
description: A/B test whole versions of an app in production with the Statespace `ssp` CLI. Use when asked to test a code change on live traffic, compare a branch against production, roll out a change gradually, or read experiment results.
---

# Statespace

Statespace measures the effect of a code change on live traffic. A **variant** is a signed,
ready-to-run build of an app, usually a branch. An **experiment** sends a share of the
app's subjects to each variant; the rest, **control**, get the deployed code. `ssp host`
runs control and the variants side by side and routes each subject to one of them. Apps
report outcomes, such as purchases, by printing a JSON line.

Every `ssp` command except `ssp host` prints JSON. Errors print `{"error": "..."}` and exit
with status 1.

## The loop

1. Read what has been tried and what happened before changing anything.

   ```shell
   ssp experiment list --app <app>
   ssp app outcomes <app>
   ssp experiment results <experiment> --outcome <outcome>
   psql "$DATABASE_URL" -c "SELECT * FROM <app>.notes ORDER BY timestamp DESC LIMIT 20"
   ```

2. Make one change on a branch. If it measures something new, print an outcome where it
   happens: `{"statespace": "<outcome>", "subject": "<subject id>", ...fields}`. The
   subject is the `x-statespace-subject` request header.

3. Try it locally against the deployed code, choosing the variant with a header.

   ```shell
   ssp host --local --command "<start command with {port}>" --variant mine=<branch checkout>
   curl -H 'x-statespace-variant: mine' localhost:8080/<path>
   ```

4. Push the branch. CI builds it like production and publishes it, signed with a key that
   only CI holds:

   ```shell
   ssp variant create <variant> <build directory> --app <app>
   ```

5. Create or update the experiment, with a note that states the hypothesis. Start small.

   ```shell
   ssp experiment create <experiment> --app <app> --variant <variant>=0.5 --traffic 0.05 \
     --note "<why this should move the outcome>"
   ssp experiment start <experiment> --note "<what this run tests>"
   ```

6. Check that the variant runs before waiting for outcomes.

   ```shell
   ssp app hosts <app>
   ssp variant logs <variant> --app <app>
   ```

7. Wait for outcomes, then decide: raise `--traffic`, publish a fix, or stop. Leave a note
   with the evidence either way.

   ```shell
   ssp experiment results <experiment> --outcome <outcome> --metric <field>
   ssp experiment update <experiment> --traffic 0.2 && ssp experiment start <experiment>
   ssp experiment stop <experiment> --note "<result and decision>"
   ```

## Rules

- Change one idea per variant, so a result says what caused it.
- A variant shares the app's database and services with control. Never change the
  database schema in a variant; land migrations in the deployed code first.
- Never raise traffic without reading `results`. An `effect` interval that includes 0 is
  not evidence of a difference.
- If `results` reports `sample_ratio.mismatch`, subjects did not split as the weights
  say, so the effects cannot be trusted. Check `unavailable` and `ssp variant logs` for a
  variant that fails, and fix it before reading outcomes.
- Check `ssp variant logs` and the `metrics` view for errors and latency before trusting
  outcomes. A failing variant sends its subjects to control.
- `ssp experiment start` fails when the app's other experiments already hold the traffic
  it asks for, or when hosts cannot run more variants. Stop an experiment or ask for less.
- Shipping a winner means merging its branch and deploying it as usual. Statespace does not
  deploy control.
- `ssp app delete`, `ssp experiment delete`, and `ssp token revoke-all` need `--yes` and
  cannot be undone. Ask before running them.
