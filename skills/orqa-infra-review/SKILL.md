---
name: orqa-infra-review
description: Orqadence's review for orqa:infra Tickets. Runs the offline infrastructure checks (terraform, tflint, trivy, hadolint, helm, kubeconform, actionlint) on the files a Ticket's diff touches and turns every result into a Finding. Loaded by the Review Stage for orqa:infra's Extra review, not by hand.
---

# Infra review

You run the offline checks on the infrastructure files the Ticket's diff touches. Every result is a Finding: the review cannot change code, and the Fix fixes what fails. Your Findings are the checks' results only, formatting included; every item you report is a Finding, so the Review Stage keeps its style-only ones, and you add none of your own judgment.

There is no network, and the Worktree is read-only. Before you started, the Orchestrator ran `fetch.sh` (beside this file) with network into the **Cache** Input: `providers/`, `tflint/` and `kubeconform/` under it. Every other tool runs offline as it is.

## Which checks

List the files `git -C <Worktree> diff --name-only <base>...HEAD` touches, and run only the checks that match them:

- `*.tf`, `*.tf.json`, `*.tfvars`, `*.tftest.hcl`, `.terraform.lock.hcl`: in each Terraform root (the folder holding the file; for a `tests/` folder, its parent): terraform fmt, terraform validate, terraform test, tflint, trivy config.
- `Dockerfile`, `Dockerfile.*`, `*.Dockerfile`: hadolint, trivy config.
- a file in a Helm chart (the nearest folder above it with a `Chart.yaml`): helm lint, helm template piped into kubeconform, trivy config on the chart.
- another `*.yaml` or `*.yml` with a `kind:` line, outside `.github/`: kubeconform, trivy config.
- `.github/workflows/*.yml` or `*.yaml`: actionlint, with shellcheck on PATH so it checks the `run:` scripts too.

Nothing matches: write the Findings heading with no items.

## Run them offline

Copy the committed tree once, so relative module sources resolve, and run every check in the copy: its paths are the Worktree's paths.

```bash
mkdir -p "$TMPDIR/infra" && git -C <Worktree> archive HEAD | tar -xf - -C "$TMPDIR/infra"
```

In each touched Terraform root, inside the copy:

1. `terraform fmt -check -recursive`: each file it lists is one Finding.
2. `terraform init -backend=false -input=false -plugin-dir=<Cache>/providers`. If it fails on a checksum the lock file lacks, delete the copy's `.terraform.lock.hcl` and run it again. If it still fails (a module from a registry or git needs the network), list the root's validate and tests under Not run with init's last error line.
3. `terraform validate -json`.
4. `terraform test -filter=<file>` for each `.tftest.hcl` that declares `mock_provider`. A test file without one would call real providers: never run it, list it under Not run.
5. `TFLINT_PLUGIN_DIR=<Cache>/tflint tflint --format=json`, with `--config` the root's `.tflint.hcl`, else the one at the top of the copy, if any. If a plugin it names is not in the cache, run it again with `--config` an empty file, so only its bundled ruleset runs, and list the plugins under Not run.
6. `trivy config --skip-check-update --cache-dir "$TMPDIR/trivy" --format json <root>`.

For Dockerfiles, charts and manifests:

- `hadolint -f json <Dockerfile>`, and `trivy config --skip-check-update --cache-dir "$TMPDIR/trivy" --format json` on each touched Dockerfile, chart and manifest.
- `helm lint --strict <chart>`, and `helm template <chart> | kubeconform ... -` with the flags below.
- `kubeconform -strict -summary -output json -cache <Cache>/kubeconform -ignore-missing-schemas <files>`, for its default Kubernetes version. A resource it skips, or whose schema fails to download (`failed downloading schema`: offline, a kind with no cached schema, such as a CRD), is not checked: list it under Not run.
- `actionlint -format '{{json .}}' <workflow files>`.

A check that stops with a tool error, not with results, goes under Not run with its last error line.

### The Fetch Input

With a **Fetch** Input (`not run: <error>`), the fetch failed and the user chose to run without it. Skip terraform validate, the mocked tests and kubeconform, and run tflint with only its bundled ruleset. List each of them, and tflint's plugins, under Not run with that error. With no **Cache** Input at all, do the same, with the reason "no Cache".

## Findings

Write each result in the Review Stage's shape, `- (severity) path:line — tool: message`, with the path relative to the Worktree:

- terraform fmt: `(low)` on the file, no line: `- (low) infra/main.tf — run terraform fmt`.
- terraform validate: error `high`, warning `medium`.
- terraform test: a failing mocked run is `(high)` on its `.tftest.hcl` file: `- (high) infra/tests/bucket.tftest.hcl — terraform test: run "bucket_is_private": <message>`.
- tflint: error `high`, warning `medium`, notice `low`.
- trivy config: CRITICAL and HIGH `high`, MEDIUM `medium`, LOW and UNKNOWN `low`. A result with no `StartLine` is on the file alone. A Helm result's line is in the rendered manifest: find it in the template, or report the template's file alone.
- hadolint: error `high`, warning `medium`, info and style `low`.
- actionlint: `[expression]` on untrusted input and `[syntax-check]` `high`, `[shellcheck]` by its SC level (error `high`, warning `medium`, info and style `low`), every other kind `medium`.
- kubeconform: invalid `high`. helm lint: `[ERROR]` `high`, `[WARNING]` `medium`. Neither gives a line: find it from the kind, name and JSON path (for a chart, in the template that renders that resource), or report the file alone.

## Not run

After the Findings, when any check was skipped, add a `## Not run` section to the result file: one line per skipped check, `- <check> on <path>: <why>`, never starting with `- (`. For example:

```
## Not run

- kubeconform on k8s/cert.yaml, Certificate web-tls: no schema (a CRD)
- terraform validate on infra/: Fetch not run: <error>
```

When every check ran, leave the section out.
