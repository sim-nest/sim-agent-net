.PHONY: ai-sanity ai-sanity-ci ai-sanity-local ai-sanity-hosted

ai-sanity: ai-sanity-ci

ai-sanity-ci:
	./scripts/ai-sanity.sh providers-ci

ai-sanity-local:
	./scripts/ai-sanity.sh providers-local

ai-sanity-hosted:
	./scripts/ai-sanity.sh providers-hosted
