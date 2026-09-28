# orqa:infra: skills, offline checks and PR evidence without credentials

Research for harness-bsg.20, 2026-09-28. harness-bsg.7 made orqa:infra a shipped Area label. Its Implement, Fix and Address sessions write Terraform, plus whatever else the Ticket needs (Docker, Kubernetes/Helm, CI config), and they never use a credential. Anything that needs one becomes Manual work.

This builds on `docs/research/label-skills.md` (branch `research/label-skills`):
- §1 lists the constraints a label's skill must survive;
- §4 has the Terraform facts: plan needs remote state and credentials, and validate needs `init -backend=false`.

It also builds on `docs/research/skill-alternatives.md` (branch `research/skill-alternatives`), which covers what rules a skill out for an unwatched Stage. None of that is repeated here.

**Checked against**, all on 2026-09-28:
- **Skill repos.** 14 repos shallow-cloned at HEAD as the Skill manifest does (`src/skills/manifest.rs:716-742`), with every candidate's SKILL.md and bundled files read. Each candidate folder was grepped for:
  - symlinks;
  - `disable-model-invocation` and `allow_implicit_invocation`;
  - `../` references;
  - asks;
  - network use;
  - commands that need a cluster or a credential.
- **Metadata.** Repo metadata from `gh api repos/<o>/<r>`, and install counts from the skills.sh search API (`GET https://skills.sh/api/search?q=<term>`, about 35 terms).
- **Tools run here.** terraform 1.16.3, tflint 0.64.0, trivy 0.74.0, checkov 3.3.20, hadolint 2.15.1, kubeconform 0.8.0, actionlint 1.7.12, shellcheck 0.11.0, helm 4.3.0 and kubectl 1.36.1. They ran on a fixture with planted problems.
  - The checks in §3 ran with the network cut by pointing `HTTPS_PROXY` at a closed port.
  - The `terraform test`, `graph` and `plan` runs in §1 came after an offline `init -plugin-dir`.
  - The machine has no cloud credentials: no `AWS_*` variables, no `~/.aws`.

- **V**: verified, meaning read in the primary source or run here.
- **U**: unverified.
- A **manifest source** is `owner/repo/path-to-skill-folder` (`parse_source`, `src/skills/manifest.rs:389`).

## Summary

1. **Skills.** Every area has a skill that survives, and every one needs the same Stage override (§2.5).

   | Area | Pick | Runner-up |
   |---|---|---|
   | Terraform | hashicorp `terraform-style-guide`, plus `terraform-test` for mocked plan-mode tests | antonbabenko `terraform-skill` |
   | Docker | docker `docker-build-strategies` | awesome-copilot `multi-stage-dockerfile` |
   | K8s/Helm | lukasniessen `kubernetes-skill` | jeffallan `kubernetes-specialist`, plus wshobson `helm-chart-scaffolding` |
   | CI | addyosmani `ci-cd-and-automation`, plus awesome-copilot `github-actions-hardening` | wshobson `github-actions-templates` |

2. **Offline checks.** Everything in the default set below runs with no credentials and no cluster (§3), but several need a download the first time:

   | Tool | Checks | Needs the network once for |
   |---|---|---|
   | `terraform fmt`, `validate` | formatting, config errors | providers (validate only) |
   | `terraform test` with `mock_provider` | the module's behaviour | providers |
   | tflint | Terraform lint | cloud rulesets |
   | trivy config | misconfigurations in Terraform, Dockerfiles, Kubernetes and Helm | nothing, with `--skip-check-update` |
   | hadolint | Dockerfiles | nothing |
   | `helm lint`, `helm template` piped into kubeconform | Helm charts, Kubernetes schemas | schemas |
   | actionlint (plus shellcheck) | GitHub Actions workflows | nothing |

3. **Network is not the same as credentials.** A codex Stage (Implement or Review) has no network (workspace-write), and the claude Review's sandbox pre-allows no domains. So anything a check downloads (providers, tflint plugins, kubeconform schemas) has to be fetched before the Stage starts. The aws provider alone is 650 MB. Which step fetches it is open (§6).

4. **Findings.** Output from terraform validate, tflint, trivy and hadolint maps straight onto `- (severity) path:line — problem`. checkov and actionlint give a line but no severity. kubeconform and helm lint give no line. See §3.3.

5. **Strongest evidence without a plan.** `terraform test` with `mock_provider` runs the module in plan and apply mode with no credentials (V, run with the aws provider). `terraform plan` fails without credentials even with `-refresh=false` and an empty local state (V).

6. **What an infra PR carries** (§4):
   - a Mermaid `flowchart` of the resources the diff touches, marked add, change, replace or destroy, and labelled "from the diff, not a plan";
   - the replacement and data-loss risks;
   - rollout and rollback notes per kind;
   - the offline checks' commands and output;
   - the Manual work filed, or a pointer to the plan CI posts (Atlantis, HCP Terraform).

7. **Always Manual work** (§5):
   - plan, apply and destroy;
   - `init` against the real backend, and state migration;
   - every `terraform state`, `import`, `force-unlock` and workspace command;
   - kubectl or helm against a cluster;
   - `docker push` and registry login;
   - creating or rotating secret values;
   - repo secrets, variables and environments.

   `moved`, `removed` and `import` blocks turn most state moves into code that the Stage can write. They take effect only at the next plan and apply.

## 1. The line between Stage work and Manual work

| Action | Needs | Stage? | Status |
|---|---|---|---|
| `terraform fmt -check -recursive` | nothing, not even init | yes | V, run |
| `terraform init -backend=false` | registry network for public providers and modules; **a credential** for private-registry modules and providers ("you must authenticate … to use artifacts in your organization's private registry", [HCP private registry](https://developer.hashicorp.com/terraform/cloud-docs/registry/using)) | yes for public sources; Manual work for private ones | V run; private-registry V (docs). Private git module sources over SSH are U. |
| `terraform init -plugin-dir=<dir>` | a pre-filled plugin directory; fully offline | yes | V: `Installed hashicorp/aws v5.100.0 (unauthenticated)`. A plugin *cache* plus a lock file still queried registry.terraform.io and failed offline (V). |
| `terraform validate` | init first; without it: `Missing required provider … run terraform init` | yes | V |
| `terraform test` with `mock_provider` | init first; "without creating infrastructure or requiring credentials" ([mocking](https://developer.hashicorp.com/terraform/language/tests/mocking)); Terraform ≥ 1.7 (CHANGELOG v1.7) | yes | V: an S3 module with a declared `backend "s3"` passed `command = apply` against `mock_provider "aws" {}` with no credentials and no backend |
| `terraform graph` | a backend initialised; after `init -backend=false` it fails: `Backend initialization required` | only with a scratch `backend_override.tf` pointing at a local path, never committed | V: DOT output for aws resources with no credentials |
| `terraform plan` | provider credentials, even with `-refresh=false` and an empty local state; plus the real state for a meaningful diff | **Manual work** | V: `Error: No valid credential sources found … provider "aws"` |
| `terraform apply`, `destroy` | credentials and state | **Manual work** | V (label-skills §4) |
| `terraform state mv`, `rm`, `list`, `show`, `pull`; CLI `import`; `force-unlock`; `taint`; `workspace new`/`select`; `init -migrate-state` | the backend (state), which means credentials | **Manual work** | U as runs; follows from state living in the backend |
| `moved {}` (≥ 1.1), `removed {}` (≥ 1.7), `import {}` (≥ 1.5) blocks | nothing to write them; they act at the next plan and apply | yes, as code | V: [refactoring](https://developer.hashicorp.com/terraform/language/modules/develop/refactoring) ("Terraform v1.1 and later"); CHANGELOG v1.5 and v1.7. `removed` with `lifecycle { destroy = false }` drops an object from state "without destroying the actual resource" ([removed](https://developer.hashicorp.com/terraform/language/block/removed)). |
| `terraform providers lock -platform=…` | registry network, no credentials | yes, where the Stage has network | U as run. An offline `-plugin-dir` init writes a lock file with checksums for the local platform only, and warns "Terraform running on another platform will fail to install these providers" (V). A Stage must not commit that lock file. |
| `helm lint`, `helm template`, `helm install --dry-run` | nothing for a chart without dependencies; no kubeconfig | yes | V on helm 4.3.0 with `KUBECONFIG=/nonexistent`: `--dry-run` is deprecated for `--dry-run=client` and printed `STATUS: pending-install`. `helm dependency build` needs the network (U). |
| `kubectl apply --dry-run=client` | **an API server**: `failed to download openapi … connection refused`; with `--validate=false`, `unable to recognize … server API group list` | no; not an offline check | V, kubectl 1.36.1 |
| `kubectl apply`, `diff`, `--dry-run=server`, `rollout`; `helm install`, `upgrade`, `rollback` | a cluster and its credentials | **Manual work** | V (kubectl, above); `helm rollback` "rolls back a release" in a cluster ([helm rollback](https://helm.sh/docs/helm/helm_rollback/)) |
| `docker build` | a Docker daemon, plus base-image pulls (network; a credential for private registries) | optional, off by default | U as run (the daemon was not relied on). docker `docker-build-strategies`' `scripts/verify-build.sh:24` runs it (V). |
| `docker push`, `docker login` | registry credentials | **Manual work** | U as run |
| Secret values: create, rotate, put in a secret manager, a Kubernetes Secret with real data, `gh secret set`/`gh variable set` | a credential or the value itself | **Manual work** | The Stage writes references only: `sensitive`/`ephemeral` variables (ephemeral ≥ 1.10, write-only attributes ≥ 1.11, CHANGELOGs V), `${{ secrets.X }}`, ExternalSecret-style manifests |
| Pinning an action to a SHA (`git ls-remote` on a public repo) | network, no credentials | yes, where the Stage has network | U |

**`gh` is authenticated in every Stage.** create-pr pushes and opens the PR with it (`skills/create-pr/SKILL.md`). So "never use credentials" has to name the gh calls it forbids:
- `gh secret`;
- `gh variable`;
- `gh workflow run`;
- `gh api` writes to repo settings.

Otherwise an Implement session could "helpfully" set a repo secret through a credential the Stage already holds.

## 2. Implement skills

Every folder below passed the checks in label-skills §1:
- no symlinks;
- no `disable-model-invocation` or `allow_implicit_invocation: false`;
- `../` only inside the folder, or inside HCL or YAML examples;
- no required sibling skill or plugin agent.

The exceptions are noted in each row. All V.

Repo metadata (gh api) and the commit read:

| Repo | License | Stars | Last push | Commit read |
|---|---|---|---|---|
| hashicorp/agent-skills | MPL-2.0 | 877 | 2026-09-24 | 516354c |
| antonbabenko/terraform-skill | Apache-2.0 (the SKILL.md frontmatter; GitHub shows NOASSERTION because of a copyright preamble) | 2,387 | 2026-07-03 | 0a3a4a6 |
| docker/skills | Apache-2.0 | 373 | 2026-09-25 | ddbf34b |
| github/awesome-copilot | MIT | 39,467 | 2026-09-27 | 6c4d33b |
| lukasniessen/kubernetes-skill | MIT | 438 | 2026-09-13 | 34f93c1 |
| jeffallan/claude-skills | MIT | 11,657 | 2026-08-07 | 882ef55 |
| wshobson/agents | MIT | 40,048 | 2026-09-28 | 9b15b34 |
| addyosmani/agent-skills | MIT | 99,587 | 2026-09-26 | 2686b62 |

"Inst." in the tables below is skills.sh installs on 2026-09-28. A row's SKILL.md is at `https://github.com/<owner>/<repo>/blob/HEAD/<path>/SKILL.md`.

### 2.1 Terraform

| Skill | Manifest source | Inst. | Verdict |
|---|---|---|---|
| **hashicorp `terraform-style-guide`** (pick) | `hashicorp/agent-skills/plugins/terraform/skills/terraform-style-guide` | 11,502 | First-party. Covers file layout, naming, variables and outputs, `for_each`, version pinning, and a review checklist, plus `SECURITY.md`. No asks, no network, no credentials. Its "run before committing" block is `fmt` then `validate` (:292-295), which misses the `init -backend=false` that validate needs. It says to commit `.terraform.lock.hcl` (:286); see the lock-file row in §1. It names tflint and checkov/tfsec (:298-299). |
| **hashicorp `terraform-test`** (pick, paired) | `hashicorp/agent-skills/plugins/terraform/skills/terraform-test` | 8,069 | Writes `.tftest.hcl`. **Its default test mode is apply, which creates real resources** (:28, :234). The same file says "Default to plan … Use mocks for external dependencies — faster and no credentials needed" (:427-428). The Stage override keeps it to `command = plan` or apply against `mock_provider` only. |
| hashicorp `refactor-module` | `hashicorp/agent-skills/plugins/terraform/skills/refactor-module` | 5,894 | Useful for its `moved` blocks (:305-325). Its prerequisites (`terraform state list`/`show -json`, :26, :288-296), `state mv` (:331-334) and `plan`/`apply` (:515-521) are all Manual work. Links sibling skills by raw URL, for reading only (:549-550). An add-on for refactor Tickets, not a default. |
| antonbabenko `terraform-skill` (runner-up) | `antonbabenko/terraform-skill/skills/terraform-skill` | 6,464 | Richer: Terraform and OpenTofu, identity churn, `moved`/`import`/`removed`, write-only attributes, native tests, CI. Its "Response Contract" demands assumptions, a validation plan and **rollback notes** (:14-22), which is PR material (§4). Needs overrides: `plan -out` in the validation plan (:21); "Never recommend direct production apply without a reviewed plan artifact and approval" (:24); "Terraform MCP" schema lookups (:118), which is the repo-root `mcp.json` running `docker run hashicorp/terraform-mcp-server` (network, outside the folder, optional); terraform-ls needs init (:285). Destroy needs "explicit confirmation" (:26), but the Stage never destroys. |
| wshobson `terraform-module-library` | `wshobson/agents/plugins/cloud-infrastructure/skills/terraform-module-library` | 14,715 | Most installs, but its only test pattern is Terratest `InitAndApply`/`Destroy` (:240-241), which needs a real cloud. Adds nothing over hashicorp's skills. |
| jeffallan `terraform-engineer` | `jeffallan/claude-skills/skills/terraform-engineer` | 4,948 | **Rejected.** Its core loop is `plan -out` (:27), then "ask for explicit approval" before `apply` (:28): a hard gate plus credentials. |
| terramate `terraform-best-practices` | `terramate-io/agent-skills/skills/terraform-best-practices` | 377 | Clean reference (37 rules, MIT), but low adoption and no push since 2026-02. |
| hashicorp `terraform-search-import`, `terraform-stacks` | `hashicorp/agent-skills/plugins/terraform/skills/…` | 4,579 / 5,383 | **Rejected.** search-import queries the live cloud (:13, :81). stacks is HCP-only and reads the TFC token from `~/.terraform.d/credentials.tfrc.json` (`references/api-monitoring.md:32`). |
| akin-ozer `terraform-generator` / `-validator` | `akin-ozer/cc-devops-skills/devops-skills-plugin/skills/…` | 559 / 561 | **Rejected.** The generator requires its sibling by plugin name (`Skill(devops-skills:terraform-validator)`, :24). Both make WebSearch or the Context7 MCP "REQUIRED" (:17, :63, :77-79). The validator installs checkov itself and runs `init` with the backend (:85, :137). |

### 2.2 Dockerfiles

| Skill | Manifest source | Inst. | Verdict |
|---|---|---|---|
| **docker `docker-build-strategies`** (pick) | `docker/skills/skills/docker-build-strategies` | 177 | First-party, Apache-2.0. Covers multi-stage builds, cache mounts, BuildKit secrets, `.dockerignore`, a non-root user and image size. No asks, no network in the text. `scripts/verify-build.sh` runs `docker build` (daemon, image pulls); it is optional. Its private-registry example (`ssh-add`, `--secret src=$HOME/.npmrc`, :80-90) is for the user's builds, not the Stage's. **Very new: 177 installs.** |
| docker `docker-project-foundations` | `docker/skills/skills/docker-project-foundations` | 160 | First Dockerization: `.dockerignore`, Dockerfile, `compose.yaml`. The same npmrc note applies. Pair it with the pick for "containerize X" Tickets. |
| awesome-copilot `multi-stage-dockerfile` (runner-up) | `github/awesome-copilot/skills/multi-stage-dockerfile` | 27,152 | 46 lines, a converted Copilot prompt. "Scan the final image" (:36) needs a built image. |
| affaan-m `docker-patterns` | `affaan-m/ecc/skills/docker-patterns` | 12,366 | Usable, but about 165 of its 520 lines (:275-439) document ECC's own installer harness. |
| sickn33 `docker-expert` | `sickn33/agentic-awesome-skills/skills/docker-expert` | 27,058 | **Rejected.** "Stop and ask for clarification" (:418); step 0 may "recommend switching and stop" (:16-23); runs `docker build`, `run` and `scout` (:58-63); an aggregator's copy. |
| akin-ozer `dockerfile-generator` | `akin-ozer/cc-devops-skills/devops-skills-plugin/skills/dockerfile-generator` | 620 | **Rejected.** "Use AskUserQuestion if information is missing" (:114), plus Context7 and web search (:141-148). |

### 2.3 Kubernetes and Helm

| Skill | Manifest source | Inst. | Verdict |
|---|---|---|---|
| **lukasniessen `kubernetes-skill`** (pick) | `lukasniessen/kubernetes-skill` (SKILL.md is at the repo root) | 1,008 | Covers failure modes: security defaults, resources, network exposure, RBAC, fragile rollouts, API drift. It has Helm and Kustomize references and conditional EKS, GKE, AKS, OpenShift and GitOps files. Its output contract includes **rollback notes** (:100). Its validate step offers `kubectl apply --dry-run=server` or `kubectl diff` (both need a cluster) *or* kubeconform (:84-89); the override keeps kubeconform and `helm template`. **Install shape:** with an empty path, `add` takes the clone root as the one skill and copies all of it except `.git`: 75 files, `.github/` and `docs/` included (`add`, `manifest.rs:443-473`; `copy_dir` :948; V by reading, not run). |
| jeffallan `kubernetes-specialist` (runner-up) | `jeffallan/claude-skills/skills/kubernetes-specialist` | 13,265 | Broad, with Helm and GitOps references. Its Validate step runs `kubectl rollout status`, `kubectl get pods -w` (which never exits), `describe` and `rollout undo` against a live cluster (:34, :214-232). The override replaces that step. |
| wshobson `helm-chart-scaffolding` (Helm add-on) | `wshobson/agents/plugins/kubernetes-operations/skills/helm-chart-scaffolding` | 11,132 | Chart layout, values and helpers. `helm lint`/`template` are offline. Its `helm install --dry-run --debug` (:60; `scripts/validate-chart.sh:108`) is offline too on Helm 4 (§1, V). |
| wshobson `k8s-manifest-generator`, `k8s-security-policies` | `wshobson/agents/plugins/kubernetes-operations/skills/…` | 10,388 / 13,908 | Usable templates, but troubleshooting is kubectl against a cluster (:47-61). |
| wshobson `gitops-workflow` | same pack | 10,782 | **Rejected.** Setup runs `kubectl apply` of Argo CD and reads the admin secret (:36-42). |
| akin-ozer `k8s-yaml-generator` | `akin-ozer/cc-devops-skills/…` | 576 | **Rejected.** Context7 (:82-98) and `kubectl apply --dry-run=server` (:193). |

### 2.4 CI (GitHub Actions)

| Skill | Manifest source | Inst. | Verdict |
|---|---|---|---|
| **addyosmani `ci-cd-and-automation`** (pick) | `addyosmani/agent-skills/skills/ci-cd-and-automation` | 39,113 | A quality-gate pipeline, Actions config, preview deploys. No asks. Secrets appear only as `${{ secrets.X }}` (:111-137). It mentions `debugging-and-error-recovery` (:188) but does not require it. Examples lean toward JavaScript. |
| **awesome-copilot `github-actions-hardening`** (pick, paired) | `github/awesome-copilot/skills/github-actions-hardening` | 1,585 | Covers `${{ }}` injection, `pull_request_target`, SHA pinning (:97-102), least-privilege `permissions:`, OIDC, and has a severity table (:129-130). How to resolve a tag to a SHA is not said (U). |
| wshobson `github-actions-templates` | `wshobson/agents/plugins/cicd-automation/skills/github-actions-templates` | 16,069 | Its templates contradict the hardening skill: static `AWS_ACCESS_KEY_ID` secrets instead of OIDC (:143-144), and actions pinned by tag. |
| dotnet `authoring-github-workflows` | `dotnet/skills/.agents/skills/authoring-github-workflows` | 1,928 | **Rejected.** Downloads actionlint (linux_amd64 only) at run time (:73-84), and points outside its folder (:11, :128). It is that repo's internal skill. |

### 2.5 The override every pick needs

No pick asks the user or runs a hard gate once plan, apply and destroy are removed. Several put `plan` or cluster calls in their "validate" step:
- terraform-skill :21;
- kubernetes-skill :84-89;
- kubernetes-specialist :34;
- terraform-test's default apply mode.

So the label's guidance line (or a fixed line in the Stage skills for orqa:infra) has to say, in effect:

> Validate with offline commands only (§3). Never run `terraform plan`, `apply`, `destroy`, `state`, `import`, `force-unlock` or `workspace`; `init` with a backend; `kubectl` or `helm` against a cluster; `docker push` or `login`; `gh secret`, `gh variable`, `gh workflow run`; or anything that reads a secret value. Write `terraform test` files with `mock_provider` only. Where a skill says to run one of these, file it as Manual work instead.

Softening those steps is not enough: jeffallan `terraform-engineer` (:27-28) makes plan-then-approve-then-apply its core loop.

## 3. Offline checks

### 3.1 Where a Stage can reach the network

| Stage session | How Orqadence starts it | Network |
|---|---|---|
| Implement on claude | `--permission-mode plan` with a plan hook first (`src/orchestrator/stage.rs:966-983`); Fix and Address on claude start with `--permission-mode auto --add-dir <run dir>` and no sandbox (`app.rs:110-113`, `stage.rs:986-990`) | Allowed, subject to auto mode's classifier. Whether the classifier lets a registry or GitHub download through is U. |
| Implement on codex (codex runs no Fix or Address, `stage.rs:988`) | `--sandbox workspace-write --add-dir <run dir>` (`app.rs:166-169`) | **Off.** "the default `workspace-write` sandbox mode keeps network access turned off unless you enable it" ([Codex approvals & security](https://learn.chatgpt.com/docs/agent-approvals-security), V). |
| Review on claude | Bash sandbox on, `failIfUnavailable`, `allowUnsandboxedCommands: false` (`app.rs:90-108`) | "Claude Code pre-allows no domains by default". In auto mode Claude names the hosts a command needs, for the classifier ([sandboxing](https://code.claude.com/docs/en/sandboxing), V). U in a Stage. |
| Review on codex | `--sandbox workspace-write` (`app.rs:159`) | Off, as above. |

So every download a check needs has to happen before the Stage, into a place the tool reads offline:
- providers: `terraform init -plugin-dir`, or a CLI-config `filesystem_mirror`, which is U;
- tflint plugins: `TFLINT_PLUGIN_DIR`;
- kubeconform schemas: `-schema-location` pointing at a local tree;
- trivy: `--skip-check-update`, which uses its embedded checks.

### 3.2 The tools

Run here on the fixture with the network cut (V unless marked):

| Tool | Command | Needs installed | First-run network | Exit on findings | Location and severity | Formats |
|---|---|---|---|---|---|---|
| terraform fmt | `terraform fmt -check -diff -recursive` | terraform | none; no init | 3 | file plus unified diff; no severity | none; it is a fixer, so the Stage just runs `terraform fmt` |
| terraform validate | `terraform init -backend=false -input=false` (or `-plugin-dir=<dir>`), then `terraform validate -json` | terraform plus providers (aws alone: 650 MB) | providers from registry.terraform.io, unless `-plugin-dir` | 1 | `diagnostics[].range.filename`, `.range.start.line`, `severity` error or warning | `-json` |
| terraform test | `terraform test` over `tests/*.tftest.hcl` with `mock_provider` | as validate | as validate | 1 on failure (U; 0 on pass, V) | run name and assertion message, no line | `-json` (U) |
| tflint | `tflint --format=json` (`compact` for humans) | tflint (52 MB); aws/google/azurerm rulesets via `tflint --init` into `TFLINT_PLUGIN_DIR` | the bundled `terraform` ruleset needs none. `--init` calls the GitHub API: offline it fails with `Failed to fetch GitHub releases`; unauthenticated calls are limited to 60/h, so set `GITHUB_TOKEN`. `deep_check` uses AWS credentials and is off by default. | 2 (1 is a tool or HCL error, not a finding) | `file:line:col`, severity error, warning or notice, rule id | json, sarif, checkstyle, junit, compact |
| trivy config | `trivy config --skip-check-update --format json .` | trivy (161 MB); renders Helm charts itself | a fresh cache downloads a 234 KB checks bundle from `mirror.gcr.io`. Offline it falls back to the embedded checks. `--skip-check-update` never tries. No vulnerability DB for config scans. | 0 by default; `--exit-code 1` | `Target` plus `CauseMetadata.StartLine`, severity CRITICAL…LOW. Some findings have no line (for example DS-0002, no USER). Helm lines refer to the rendered manifest, not the template. | json, sarif, table, github… |
| checkov | `checkov -d . -o json --skip-download --quiet` | Python plus a 168 MB venv; Helm needs `helm` on PATH (U) | `pip install` once; `--skip-download` stops Prisma guide calls | 1 (`--soft-fail` gives 0) | `file_path`, `file_line_range`; **no severity** (`severity: None`; severity "requires API key", README) | json, sarif, junitxml… |
| hadolint | `hadolint -f json Dockerfile` | one 102 MB binary; ShellCheck built in for RUN lines | none | 1 | `file:line`, level error, warning, info or style, rule DLxxxx | json, sarif, checkstyle, codeclimate… |
| helm lint / template | `helm lint --strict chart`; `helm template chart \| kubeconform …` | helm (63 MB) | none without chart dependencies | lint exits 1 only on `[ERROR]` | text, **no line**. `lint --strict` passed a string `replicaCount`; the kubeconform pipe caught it. | text |
| kubeconform | `kubeconform -strict -summary -output json -schema-location '<local tree template>'` | one 13 MB binary plus a schema tree for the target Kubernetes version | by default fetches schemas from `raw.githubusercontent.com` each run: offline it fails with `failed downloading schema … giving up`, exit 1. `-cache` helps only for kinds already fetched. A local `-schema-location` is fully offline. CRDs need their own schemas or `-ignore-missing-schemas` (U). | 1 | filename, kind/name and a JSON path (`/spec/replicas: got string, want null or integer`); **no line, no severity** | json, junit, tap |
| actionlint | `actionlint -format '{{json .}}' .github/workflows/*.yml` | one 5.8 MB binary; runs shellcheck on `run:` scripts only if `shellcheck` is on PATH | none. Outside a git repo, pass the files explicitly (exit 3 otherwise). | 1 | `file:line:col` plus `[kind]`; **no severity** | template-driven json |

Sample lines (V, verbatim):

```text
tflint:     main.tf:42:19: Error - "t1.2xlarge" is an invalid value as instance_type (aws_instance_invalid_type)
trivy:      tf/main.tf 36 HIGH AWS-0107 Security groups should not allow unrestricted ingress to SSH or RDP
hadolint:   {"code":"DL3007","column":1,"file":"Dockerfile","level":"warning","line":1,"message":"Using latest is prone to errors…"}
checkov:    Check: CKV_AWS_24 … FAILED for resource: aws_security_group.web  File: /tf/main.tf:30-38
kubeconform: k8s/deploy.yaml - Deployment web is invalid: … at '/spec/replicas': got string, want null or integer
actionlint: .github/workflows/ci.yml:9:30: "github.event.issue.title" is potentially untrusted. avoid using it directly in inline scripts. … [expression]
tf test:    run "bucket_is_private"... pass   Success! 1 passed, 0 failed.
```

**Gaps the fixture showed** (V):
- `terraform validate` is shallow. It missed an invalid `instance_type` and untyped variables, which tflint caught, and it stopped at its first error.
- hadolint missed the missing `USER`; trivy (DS-0002) and checkov (CKV_DOCKER_3) caught it.
- Nothing flagged `actions/checkout@v2` as unpinned. actionlint only said its runner is too old, and trivy does not scan workflows. SHA pinning is the hardening skill's job, not a check's.

**A default set.**
- **In the set:** `terraform fmt`, `validate` and `test` (mocked); tflint with its bundled ruleset; `trivy config --skip-check-update` (one binary covers Terraform, Dockerfiles, Kubernetes and Helm); hadolint; `helm lint` plus `helm template | kubeconform` against local schemas; actionlint plus shellcheck.
- **Left out:** checkov, which trivy covers with severities, from one binary with no Python.

The Stage runs only the tools that match the files the diff touches.

### 3.3 Mapping onto Findings

The Review's line is `- (high|medium|low) path:line — problem` (`skills/stage-review/SKILL.md:29-33`).

| Tool | Maps? | Severity rule |
|---|---|---|
| terraform validate | yes | error → high, warning → medium |
| tflint | yes | error → high, warning → medium, notice → low |
| trivy config | yes; findings without `StartLine` become file-level, and Helm lines point at the rendered manifest | CRITICAL/HIGH → high, MEDIUM → medium, LOW/UNKNOWN → low |
| hadolint | yes | error → high, warning → medium, info/style → low |
| checkov | line yes, severity no | one fixed level (medium) or a check-id table (U) |
| actionlint | line yes, severity no | `[expression]` on untrusted input and `[syntax-check]` → high, other kinds → medium, `[shellcheck]` by its SC level |
| kubeconform, helm lint | no line | invalid or `[ERROR]` → high, `[WARNING]` → medium; the agent finds the line from kind, name and path (U), or reports the file |
| terraform fmt | n/a | fix it; never a Finding |
| terraform test | n/a | a failing run is a failed check, not a Finding |

## 4. What an infra PR carries without a plan

getsentry `pr-writer`'s rubric (label-skills §2.5) asks for the operator effect and failure modes of an operational change. For infra, that means the following six items.

1. **Resources touched, as a Mermaid `flowchart`.**
   - Nodes are resource addresses (or Kubernetes `kind/name`), edges are references, and a `classDef` marks each node's change. `classDef` and `:::` are long-standing flowchart syntax ([Mermaid flowchart](https://mermaid.js.org/syntax/flowchart.html), V), and GitHub renders Mermaid in PR bodies (label-skills §2.1).
   - Edges can come from `terraform graph` (DOT, V with no credentials; it needs the local-backend override in §1) or from reading the diff.
   - The change kind comes from the diff, so the diagram must say so. Only a plan knows what really changes.

   ```mermaid
   flowchart LR
     %% from the diff, not a plan
     bucket["aws_s3_bucket.logs"]:::moved
     pab["aws_s3_bucket_public_access_block.logs"]:::add
     pab --> bucket
     classDef add fill:#d4f7d4,stroke:#2a7a2a
     classDef change fill:#fff3c4,stroke:#9a7a00
     classDef replace fill:#ffd9b3,stroke:#b35900
     classDef destroy fill:#f7d4d4,stroke:#a02a2a
     classDef moved fill:#dde7ff,stroke:#2a4aa0
   ```

2. **Replacement and data-loss risk.** When an argument "cannot be updated in-place due to remote API limitations, Terraform destroys the existing object and then creates a new replacement" ([lifecycle](https://developer.hashicorp.com/terraform/language/meta-arguments/lifecycle), V). Without a plan, the PR lists:
   - each changed argument that the provider docs mark as forcing replacement;
   - any stateful resource that could be replaced (buckets, databases, volumes);
   - whether `prevent_destroy` or `create_before_destroy` guards it.

   Each item is marked "unverified until a plan".

3. **Rollout and rollback, per kind.** The Kubernetes pick's output contract asks for "rollback/recovery notes (rollout undo, revision history, data safety)" (kubernetes-skill SKILL.md:100, V), and so does the Terraform runner-up's Response Contract (terraform-skill :22, V). The Terraform pick does not, so the label's guidance line has to ask for them.

   | Kind | Rollout | Rollback |
   |---|---|---|
   | Terraform | apply order across roots or workspaces; `moved`/`removed`/`import` blocks act at the next apply | revert the PR and apply. A destroyed stateful object does not come back, and `removed { lifecycle { destroy = false } }` is how to let go of one without deleting it (§1). |
   | Kubernetes | "a rollout is triggered only when the pod template changes" (Deployment docs) | `kubectl rollout undo deployment/<name>`, within `revisionHistoryLimit` ([Deployments](https://kubernetes.io/docs/concepts/workloads/controllers/deployment/), V) |
   | Helm | chart version and values change | `helm rollback <RELEASE> [REVISION]` ([helm rollback](https://helm.sh/docs/helm/helm_rollback/), V) |
   | Docker | a new tag or digest | redeploy the previous digest |
   | CI | takes effect on merge | revert the PR |

   Every rollback command here needs a cluster or credentials, so it is written down for the operator, never run.

4. **The offline checks' output.** For each check: the command, its result, and the Findings fixed. This goes in a collapsed `<details>` block, as create-pr's "Testing" section already asks for "the exact commands … and their result" (`skills/create-pr/SKILL.md:60`).

5. **`terraform test` results** from the mocked runs, when the Ticket added tests.

6. **The plan: Manual work, or CI's.**
   - If the repo already plans on PRs, the PR says the plan is in CI and stops there:
     - Atlantis autoplans "on any new pull request or new commit" ([autoplanning](https://www.runatlantis.io/docs/autoplanning), V);
     - HCP Terraform "performs a speculative plan when a pull request is opened … posts a link to the plan in the pull request" ([VCS runs](https://developer.hashicorp.com/terraform/cloud-docs/run/ui), V).
   - Signs to look for: `atlantis.yaml`, a `cloud {}` block or `backend "remote"`, a workflow that runs `terraform plan` (U as a detection rule).
   - Otherwise the PR lists the Manual work filed (plan, then apply, then any secret), each with its prompt file.
   - A saved plan holds sensitive values "in cleartext" ([plan](https://developer.hashicorp.com/terraform/cli/commands/plan), V), so Manual work should post the plan's text summary, never attach the `.tfplan` file.

## 5. Always Manual work

| Action | Why |
|---|---|
| `terraform plan`, `apply`, `destroy`, `-replace` | provider credentials and the real state (§1, V) |
| `terraform init` against the real backend; `init -migrate-state` or `-reconfigure` to move state | backend credentials |
| `terraform state mv`/`rm`/`pull`/`push`, CLI `import`, `force-unlock`, `taint`, `workspace new`/`select` | state lives in the backend. Prefer `moved`/`removed`/`import` blocks written by the Stage, then Manual work only for the plan and apply that act on them. |
| `init` when modules or providers come from a private registry or private git | "you must authenticate" (§1). Here even `validate` and `test` become Manual work; the PR says the checks could not run. |
| `kubectl apply`/`diff`/`rollout`, `--dry-run=server`; `helm install`/`upgrade`/`rollback`; `helm diff` | a cluster |
| `docker push`, `docker login`; building from private base images or private package feeds | registry credentials |
| Secret values: create, rotate, store (cloud secret manager, Vault, Kubernetes Secret, `kubeseal` against the cluster's key) | the value or a credential |
| GitHub repo secrets, variables, environments, branch protection, OIDC trust on the cloud side (unless it is Terraform the Stage writes) | gh is authenticated, but this is exactly the credential use the label forbids (§1) |
| DNS delegation, certificate issuance, quota requests, enabling cloud APIs, billing | cloud console or account actions |
| Anything the ticket calls "run it", "check it's up" or "verify in staging" | a live environment |

## 6. Open for later decisions

- **Pre-fetching.** Who downloads the providers, tflint plugins and kubeconform schemas, and when: `orqa init` for orqa:infra, preflight, or a first Stage with network (claude Implement)? Where do they live: `.orqadence-local/`, a user cache? The aws provider alone is 650 MB. A codex Stage cannot fetch them itself (§3.1).
- **Lock files.** An offline `-plugin-dir` init writes a single-platform `.terraform.lock.hcl`. The Stage must not commit one it did not intend to change. A provider upgrade is either `terraform providers lock` with several `-platform`s (network) or Manual work.
- **The gh denylist** in §2.5 is new: every other label can use gh freely.
- **Which checks run where.** Implement runs them as "the repo's checks" (create-pr step 2 and stage-implement step 4). The Review could run them for Findings. Or an extra Stage could (harness-bsg.8 decides whether labels add Stages). The list could live in the label's guidance line or in the Target repo's AGENTS.md.
- **Adoption risk.** The Docker picks are first-party but brand new (177 and 160 installs). The Kubernetes pick installs as a whole repo (75 files). Offering a runner-up in the label's skill picker covers both.
- **harness-bsg.9** (the PR template) takes §4 as the orqa:infra section.

## Sources

- **Orqadence:**
  - `src/skills/manifest.rs` (`parse_source` :389, `add` :443-473, `fetch` :716, `skill_in` :760, `copy_dir` :948);
  - `src/orchestrator/app.rs:83-175` (how each App is started and sandboxed) and `src/orchestrator/stage.rs:964-993` (`stage_args`: which args each Stage gets);
  - `skills/stage-review/SKILL.md:15,29-33`;
  - `skills/stage-implement/SKILL.md:25`;
  - `skills/create-pr/SKILL.md:17-33,60`;
  - harness-bsg.7's resolution (bd).
- **Skill repos**, cloned at HEAD on 2026-09-28 (commits in §2):
  - [hashicorp/agent-skills](https://github.com/hashicorp/agent-skills)
  - [antonbabenko/terraform-skill](https://github.com/antonbabenko/terraform-skill)
  - [docker/skills](https://github.com/docker/skills)
  - [github/awesome-copilot](https://github.com/github/awesome-copilot)
  - [lukasniessen/kubernetes-skill](https://github.com/lukasniessen/kubernetes-skill)
  - [jeffallan/claude-skills](https://github.com/jeffallan/claude-skills)
  - [wshobson/agents](https://github.com/wshobson/agents)
  - [addyosmani/agent-skills](https://github.com/addyosmani/agent-skills)
  - [terramate-io/agent-skills](https://github.com/terramate-io/agent-skills)
  - [akin-ozer/cc-devops-skills](https://github.com/akin-ozer/cc-devops-skills)
  - [affaan-m/ecc](https://github.com/affaan-m/ecc)
  - [sickn33/agentic-awesome-skills](https://github.com/sickn33/agentic-awesome-skills)
  - [dotnet/skills](https://github.com/dotnet/skills)
  - [callstackincubator/agent-skills](https://github.com/callstackincubator/agent-skills)
- **Terraform:**
  - [init](https://developer.hashicorp.com/terraform/cli/commands/init)
  - [plan](https://developer.hashicorp.com/terraform/cli/commands/plan)
  - [mocking](https://developer.hashicorp.com/terraform/language/tests/mocking)
  - [refactoring / moved](https://developer.hashicorp.com/terraform/language/modules/develop/refactoring)
  - [removed](https://developer.hashicorp.com/terraform/language/block/removed)
  - [lifecycle](https://developer.hashicorp.com/terraform/language/meta-arguments/lifecycle)
  - [HCP private registry](https://developer.hashicorp.com/terraform/cloud-docs/registry/using)
  - [HCP VCS runs](https://developer.hashicorp.com/terraform/cloud-docs/run/ui)
  - CHANGELOG.md on the v1.5, v1.7, v1.10 and v1.11 branches of [hashicorp/terraform](https://github.com/hashicorp/terraform)
- **Others:**
  - [Atlantis autoplanning](https://www.runatlantis.io/docs/autoplanning)
  - [Kubernetes Deployments](https://kubernetes.io/docs/concepts/workloads/controllers/deployment/)
  - [helm rollback](https://helm.sh/docs/helm/helm_rollback/)
  - [Mermaid flowchart](https://mermaid.js.org/syntax/flowchart.html)
  - [Codex approvals & security](https://learn.chatgpt.com/docs/agent-approvals-security)
  - [Claude Code sandboxing](https://code.claude.com/docs/en/sandboxing)
- **Tools run:** terraform 1.16.3, tflint 0.64.0 (`docs/user-guide/plugins.md`, `cmd/cli.go`), trivy 0.74.0, checkov 3.3.20 (README), hadolint 2.15.1, kubeconform 0.8.0, actionlint 1.7.12 (`docs/usage.md`), shellcheck 0.11.0, helm 4.3.0, kubectl 1.36.1.
- **skills.sh search API**, 2026-09-28. Terms included terraform, opentofu, docker, dockerfile, container, kubernetes, k8s, helm, github actions, ci-cd, devops, infrastructure and iac.
