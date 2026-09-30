#!/usr/bin/env bash
# orqa-infra-review's fetch.sh: fills ORQA_CACHE with what the offline review
# reads (Terraform providers, tflint plugins, kubeconform schemas) for the
# files the diff against ORQA_BASE touches. The Orchestrator runs it with
# network in the Ticket's worktree before each orqa:infra Extra review. It
# works on a temp copy of the committed tree and never writes the worktree.
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

roots='' charts='' manifests=''
for f in $(git diff --name-only "$ORQA_BASE...HEAD"); do
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
	git archive HEAD | tar -xf - -C "$copy"
	mkdir -p "$ORQA_CACHE/providers" "$ORQA_CACHE/tflint"
	for root in $(printf '%s' "$roots" | sort -u); do
		# The copy's lock file is thrown away, so the cache may disagree with it.
		(cd "$copy/$root" && TF_PLUGIN_CACHE_DIR="$ORQA_CACHE/providers" \
			TF_PLUGIN_CACHE_MAY_BREAK_DEPENDENCY_LOCK_FILE=true \
			terraform init -backend=false -input=false >/dev/null)
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
	# Downloads the schema of each kind it validates into the cache.
	kube() { kubeconform -strict -cache "$ORQA_CACHE/kubeconform" -ignore-missing-schemas "$@" 2>&1 || true; }
	out=''
	if [ -n "$manifests" ]; then out=$(kube $manifests); fi
	for chart in $(printf '%s' "$charts" | sort -u); do
		out="$out"$'\n'"$(helm template "$chart" 2>/dev/null | kube - || true)"
	done
	# An invalid manifest or a chart that fails is the review's to report;
	# only a schema that failed to download is this script's failure.
	if printf '%s\n' "$out" | grep 'failed downloading schema' >&2; then exit 1; fi
fi
