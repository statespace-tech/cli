---
name: statespace
description: A/B test whole versions of an app in production with the Statespace `ssp` CLI. Use when asked to test a code change on live traffic, compare a branch against production, roll out a change gradually, or read experiment results.
---

# Statespace

Statespace measures the effect of a code change on live traffic. A **variant** is a signed
image of an app, usually built from a branch. An **experiment** gives a share of the app's
subjects to each variant and the same share to **control**, a copy of the deployed image
pinned when the experiment started. Everyone else gets the **default**, the deployed app.
`ssp proxy` routes each subject; arms record every line they print.

The CLI changes things; SQL reads data. Every `ssp` command except `ssp proxy` and `ssp
controller` prints JSON. Errors print `{"error": "..."}` and exit with status 1.

## The loop

1. Read what has been tried and what happened before changing anything. Use a token with
   the `analyst` preset as `DATABASE_URL`.

   ```shell
   ssp experiment list --app <app>
   psql "$DATABASE_URL" -c "SELECT * FROM <app>.notes ORDER BY timestamp DESC LIMIT 20"
   psql "$DATABASE_URL" -c "SELECT data->>'event' AS event, count(*) FROM <app>.output
                            WHERE data IS NOT NULL GROUP BY 1 ORDER BY 2 DESC LIMIT 20"
   ```

2. Make one change on a branch. If it measures something new, print a JSON line where it
   happens, with the subject from the `x-statespace-subject` request header:
   `{"event": "<what happened>", "subject": "<subject id>", ...fields}`.

3. Check the branch's image starts and answers, with Docker.

   ```shell
   docker build -t <app>:mine . && docker run --rm -p 8000:8000 <app>:mine
   ```

4. Push the branch. CI builds and pushes its image, and publishes it signed with a key
   that only CI holds:

   ```shell
   ssp variant create <variant> --app <app> --image <registry>/<app>:<commit>
   ```

5. Create or update the experiment, with a note that states the hypothesis. Start small:
   `--split` is each arm's share of all the app's subjects, and control gets the same.

   ```shell
   ssp experiment create <experiment> --app <app> --split <variant>=0.02 \
     --note "<why this should move the outcome>"
   ssp experiment start <experiment> --note "<what this run tests>"
   ```

6. Check that the arms run before waiting for outcomes.

   ```shell
   psql "$DATABASE_URL" -c "SELECT * FROM <app>.hosts"
   psql "$DATABASE_URL" -c "SELECT variant, event, data, timestamp FROM <app>.lifecycle
                            ORDER BY timestamp DESC LIMIT 20"
   ```

7. Wait for outcomes, then compare the arms and decide: raise the split, publish a fix, or
   stop. Leave a note with the evidence either way.

   ```shell
   psql "$DATABASE_URL" -c "SELECT * FROM <app>.sample_ratio WHERE experiment = '<experiment>'"
   psql "$DATABASE_URL" -c "SELECT variant, count(DISTINCT subject_id) AS subjects,
       count(DISTINCT subject_id) FILTER (WHERE data->>'event' = '<event>') AS converted
     FROM <app>.logs WHERE experiment = '<experiment>' GROUP BY variant"
   ssp experiment update <experiment> --split <variant>=0.1 && ssp experiment start <experiment>
   ssp experiment stop <experiment> --note "<result and decision>"
   ```

## Rules

- Change one idea per variant, so a result says what caused it.
- Compare a variant with its experiment's control arm, never with the default: both arms
  are new containers with the same share of subjects.
- An arm shares the app's database and services with the default. Never change the
  database schema in a variant; land migrations in the deployed code first.
- Never raise the split without comparing the arms. A difference within noise is not
  evidence.
- If `sample_ratio` reports a mismatch, subjects did not split as the shares say, so the
  comparison cannot be trusted. Look for `variant-unavailable` runs and the arm's
  `lifecycle` and `stderr` lines, and fix the arm before reading outcomes.
- Rates such as errors need a denominator: divide by the arm's rows in `requests`.
- `ssp experiment start` fails when the app's other experiments already hold the subjects
  it asks for, and warns when proxies run fewer arms than it needs.
- Shipping a winner means merging its branch and deploying it as usual. Statespace does not
  deploy the default.
- `ssp app delete`, `ssp experiment delete`, and `ssp token revoke --all` need `--yes` and
  cannot be undone. Ask before running them.
