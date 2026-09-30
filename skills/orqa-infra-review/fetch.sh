#!/usr/bin/env bash
# orqa-infra-review's fetch.sh: fills ORQA_CACHE with what the offline review
# reads (Terraform providers and modules, tflint plugins, kubeconform schemas)
# for the files the diff against ORQA_BASE touches. The Orchestrator runs it with
# network in the Ticket's worktree before each orqa:infra Extra review. It
# only reads the worktree: Terraform inits in a temp copy of the committed
# tree, and kubeconform reads the manifests and charts where they are.
# Exit 0 is ready; on a failure the last lines of stderr say why.
set -euo pipefail
IFS=$'\n'
: "${ORQA_CACHE:?}" "${ORQA_BASE:?}"

# The nearest folder above $1 holding a Chart.yaml, if any.
chart_of() {
	local dir
	dir=$(dirname "$1")
	while [ ! -f "$dir/Chart.yaml" ]; do
		if [ "$dir" = . ]; then return 0; fi
		dir=$(dirname "$dir")
	done
	echo "$dir"
}

# Read first: a failing git in a for list would not stop the script.
touched=$(git -c core.quotePath=false diff --name-only "$ORQA_BASE...HEAD")
roots='' charts='' manifests=''
for f in $touched; do
	case $f in
	*.tf | *.tf.json | *.tfvars | *.tftest.hcl | *.terraform.lock.hcl)
		dir=$(dirname "$f")
		if [ "$(basename "$dir")" = tests ]; then dir=$(dirname "$dir"); fi
		if compgen -G "$dir/*.tf" >/dev/null || compgen -G "$dir/*.tf.json" >/dev/null; then
			roots="$roots$dir"$'\n'
		fi
		;;
	esac
	chart=$(chart_of "$f")
	if [ -n "$chart" ]; then
		charts="$charts$chart"$'\n'
	else
		case $f in
		.github/*) ;;
		*.yaml | *.yml)
			if [ -f "$f" ] && grep -q '^kind:' "$f"; then manifests="$manifests$f"$'\n'; fi
			;;
		esac
	fi
done

if [ -n "$roots" ]; then
	# ponytail: the whole committed tree is copied so relative module sources
	# resolve; copy only the roots and their local modules if repos get big.
	copy=$(mktemp -d)
	trap 'rm -rf "$copy"' EXIT
	head=$(git rev-parse HEAD)
	git archive "$head" | tar -xf - -C "$copy"
	mkdir -p "$ORQA_CACHE/providers" "$ORQA_CACHE/tflint"
	for root in $(printf '%s' "$roots" | sort -u); do
		# The copy's lock file is thrown away, so the cache may disagree with it.
		(cd "$copy/$root" && TF_PLUGIN_CACHE_DIR="$ORQA_CACHE/providers" \
			TF_PLUGIN_CACHE_MAY_BREAK_DEPENDENCY_LOCK_FILE=true \
			terraform init -backend=false -input=false >/dev/null)
		# The modules init installed (registry and git ones, which the offline
		# review cannot fetch) are kept as modules/<HEAD>/<root>/.terraform/modules:
		# keyed by commit, so another worktree's fetch never replaces the snapshot
		# this review reads, and a snapshot never holds another commit's modules.
		# A root with no module call writes none, and one already there is kept.
		# The copy lands by rename from a folder of its own, so a run killed
		# halfway leaves no half-copied entry.
		# ponytail: two fetches of one commit at once would leave a stray
		# folder inside the snapshot; each Ticket has its own HEAD, so none do.
		# ponytail: snapshots of old commits pile up; prune by age if the cache grows.
		modules=$copy/$root/.terraform/modules
		dest=$ORQA_CACHE/modules/$head/$root/.terraform/modules
		if [ -d "$modules" ] && [ ! -d "$dest" ]; then
			mkdir -p "$(dirname "$dest")"
			new=$(mktemp -d "$dest.XXXXXX")
			cp -Rf "$modules/." "$new"
			mv "$new" "$dest"
		fi
	done
	# tflint reads the .tflint.hcl in its folder; only a plugin with a source
	# is downloaded, from GitHub, which allows 60 calls an hour without a token.
	for dir in $(printf '.\n%s' "$roots" | sort -u); do
		if grep -Eqs '^[[:space:]]*source[[:space:]]*=' "$copy/$dir/.tflint.hcl"; then
			token=${GITHUB_TOKEN:-$(gh auth token 2>/dev/null || true)}
			(cd "$copy/$dir" && GITHUB_TOKEN=$token TFLINT_PLUGIN_DIR="$ORQA_CACHE/tflint" \
				tflint --init >/dev/null)
		fi
	done
fi

if [ -n "$charts$manifests" ]; then
	mkdir -p "$ORQA_CACHE/kubeconform"
	# Downloads the schema of each kind it validates into the cache. An invalid
	# manifest, a kind with no schema (a CRD) or a chart that fails to render is
	# the review's to report; a schema that failed to download or to reach the
	# cache, or a kubeconform that stopped before any resource, fails here.
	kube() {
		local out status=0
		out=$(kubeconform -strict -cache "$ORQA_CACHE/kubeconform" -ignore-missing-schemas "$@" 2>&1) || status=$?
		if printf '%s\n' "$out" | grep -E 'downloading schema|parsing schema from|write cache to disk' >&2; then
			return 1
		fi
		if [ "$status" != 0 ] && ! printf '%s\n' "$out" | grep -Eq ' (is invalid|failed validation): '; then
			printf 'kubeconform: %s\n' "$out" | tail -n 3 >&2
			return 1
		fi
	}
	if [ -n "$manifests" ]; then kube $manifests; fi
	for chart in $(printf '%s' "$charts" | sort -u); do
		if rendered=$(helm template "$chart" 2>/dev/null); then
			printf '%s\n' "$rendered" | kube -
		fi
	done
fi
