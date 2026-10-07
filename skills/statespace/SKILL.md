---
name: statespace
description: Run A/B tests on functions, prompts, and configuration in production with the Statespace `ssp` CLI. Use when asked to test a code change on live traffic, compare variants, roll out a change gradually, or read experiment results.
---

# Statespace

Statespace measures the effect of a change on live traffic. A **group** is a named set of
parameters (functions or JSON values). An **experiment** sends a share of eligible
subjects to each group; the rest, **control**, run the application's existing code. The
application reads parameters with an SDK and logs outcomes such as clicks or purchases.

Every `ssp` command prints JSON. Errors print `{"error": "..."}` and exit with status 1.

## The loop

1. Read the current results before changing anything.

   ```shell
   ssp experiment list
   ssp experiment results <experiment> --outcome <outcome>
   ```

2. Write one candidate change as a function. It takes one JSON value and returns one.
   Use the standard library or pure-language packages; the sandbox has no network,
   files, or native extensions.

3. Test it locally with representative inputs.

   ```shell
   ssp run rerank.py:score --input '[1, 3, 2]'
   ```

4. Create or update the group, then the experiment's draft.

   ```shell
   ssp group create <group> ranker=rerank.py:score top_k=20
   ssp group update <group> ranker=rerank.py:score
   ssp experiment create <experiment> --group <group>=0.05
   ```

5. Start with a small weight. `start` publishes the draft as a new immutable version.

   ```shell
   ssp experiment start <experiment>
   ```

6. Wait for outcomes, then decide: raise the weight, revise the group, or stop.

   ```shell
   ssp experiment results <experiment> --outcome <outcome>
   ssp experiment update <experiment> --group <group>=0.2 && ssp experiment start <experiment>
   ssp experiment stop <experiment>
   ```

## Values

`NAME=VALUE` resolves, in order, to: a function's source (`file.py:entry`, `file.ts:entry`,
`file.cpp:entry`, `./go-or-rust-project:Entry`); a `.json` file; any other file as text; or
a JSON literal. `ssp` builds every function itself; prebuilt `.wasm` files are rejected. Quote string literals as JSON: `model='"claude-sonnet-5-5"'`. An empty
value in `group update` removes a parameter.

## Rules

- Change one idea per group. Combine parameters in one group only to test them together.
- Each parameter must have the same type in every group of an experiment.
- Never raise a weight without reading `results`. An `effect` interval that includes 0 is
  not evidence of a difference.
- Check the `statespace.error` outcome. It counts parameters that fell back to the
  default, such as a function that failed or timed out.
- `ssp experiment delete` and `ssp token revoke-all` need `--yes` and cannot be undone.
  Ask before running them.
