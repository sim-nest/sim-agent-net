#!/bin/sh
set -eu

profile="${1:-providers-ci}"

log() {
    printf 'ai-sanity %s: %s\n' "$profile" "$*"
}

have() {
    command -v "$1" >/dev/null 2>&1
}

count_models() {
    body_file="$1"
    if have python3; then
        python3 - "$body_file" <<'PY'
import json
import sys

with open(sys.argv[1], "rb") as handle:
    data = json.load(handle)

if isinstance(data, dict):
    models = data.get("data")
    if models is None:
        models = data.get("models")
elif isinstance(data, list):
    models = data
else:
    models = []

if not isinstance(models, list):
    models = []

print(len(models))
PY
    else
        grep -Eo '"(id|name)"[[:space:]]*:' "$body_file" | wc -l | tr -d ' '
    fi
}

probe_http() {
    provider="$1"
    endpoint="$2"
    path="$3"
    shift 3

    if ! have curl; then
        log "$provider skipped unavailable"
        return 0
    fi

    body="$(mktemp)"
    if curl -fsS --max-time "${AI_SANITY_TIMEOUT_SECONDS:-2}" "$@" \
        "${endpoint%/}$path" -o "$body" >/dev/null 2>&1; then
        models="$(count_models "$body" 2>/dev/null || printf unknown)"
        rm -f "$body"
        log "$provider status=ok models=$models redacted=true"
    else
        rm -f "$body"
        log "$provider skipped unavailable"
    fi
}

providers_ci() {
    cargo test -p sim-lib-agent-runner-http --test provider_probe
    cargo test -p sim-lib-agent --features runner-http provider_profiles_returns_profile_table
    cargo test -p sim-lib-agent --features runner-http provider_probe_loopback_discovers_models_without_live_provider
    cargo test -p sim-lib-agent --features runner-http native_openai_runner_posts_json_decodes_usage_and_keeps_tools_enabled
    cargo test -p sim-lib-agent --features runner-http native_anthropic_runner_posts_messages_with_required_headers
    cargo test -p sim-lib-agent --features runner-http native_local_openai_runners_reflect_provider_defaults
    cargo test -p sim-lib-agent --features runner-ollama a6_phase1_ollama_runner_posts_chat_payload_and_decodes_response
    log "mock provider matrix ok"
}

providers_local() {
    probe_http \
        ollama \
        "${AI_SANITY_OLLAMA_ENDPOINT:-http://127.0.0.1:11434}" \
        "/api/tags"
    probe_http \
        lm-studio \
        "${AI_SANITY_LM_STUDIO_ENDPOINT:-http://127.0.0.1:1234/v1}" \
        "/models"
    probe_http \
        lemonade \
        "${AI_SANITY_LEMONADE_ENDPOINT:-http://127.0.0.1:13305/v1}" \
        "/models"
}

hosted_openai() {
    endpoint="${AI_SANITY_OPENAI_ENDPOINT:-https://api.openai.com/v1}"
    api_key_env="${AI_SANITY_OPENAI_API_KEY_ENV:-OPENAI_API_KEY}"
    api_key="$(printenv "$api_key_env" 2>/dev/null || true)"
    if [ -z "$api_key" ]; then
        log "openai skipped missing $api_key_env"
        return 0
    fi
    probe_http openai "$endpoint" "/models" \
        -H "Authorization: Bearer $api_key"
}

hosted_anthropic() {
    endpoint="${AI_SANITY_ANTHROPIC_ENDPOINT:-https://api.anthropic.com/v1}"
    api_key_env="${AI_SANITY_ANTHROPIC_API_KEY_ENV:-ANTHROPIC_API_KEY}"
    api_key="$(printenv "$api_key_env" 2>/dev/null || true)"
    if [ -z "$api_key" ]; then
        log "anthropic skipped missing $api_key_env"
        return 0
    fi
    probe_http anthropic "$endpoint" "/models" \
        -H "x-api-key: $api_key" \
        -H "anthropic-version: 2023-06-01"
}

providers_hosted() {
    hosted_openai
    hosted_anthropic
}

case "$profile" in
    providers-ci)
        providers_ci
        ;;
    providers-local)
        providers_local
        ;;
    providers-hosted)
        providers_hosted
        ;;
    *)
        printf 'usage: %s [providers-ci|providers-local|providers-hosted]\n' "$0" >&2
        exit 64
        ;;
esac
