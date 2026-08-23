This recipe assembles one checked generative exchange from reusable runtime
parts. The host places a model target at `model-site:genai`; the recipe builds a
BRIDGE ASK packet, sends it through the late-bound placement, requires a JSON
answer, and admits only a `core/String` reply.

This is the canonical cross-family provider specimen. Its source stays
unchanged for direct API seats, subscription CLI broker seats, local daemon
seats, and extension adapters; only seat data, wire, redacted principal, and
declared capability expectations vary at the host boundary.

Hosts can change the target without changing this recipe. A local adapter can
cross a subprocess boundary:

```lisp
(runner/place
  "model-site:genai"
  (runner/process
    :command "my-local-model-adapter --model ./model.gguf"
    :protocol json-stdio
    :model "local/model"))
```

Or the target can be a loopback Ollama service:

```lisp
(runner/place "model-site:genai" (runner/ollama :model "qwen3.5:4b"))
```

Provider targets use their provider credential environment names:

```lisp
(runner/place "model-site:genai" (runner/openai :model "gpt-5-mini"))
```

Required environment name: `OPENAI_API_KEY`.

```lisp
(runner/place
  "model-site:genai"
  (runner/anthropic :model "claude-sonnet-latest"))
```

Required environment name: `ANTHROPIC_API_KEY`.

```lisp
(runner/place
  "model-site:genai"
  (runner/openai-compatible
    :endpoint "https://provider.example/v1"
    :model "provider/model"
    :api-key-env "PROVIDER_API_KEY"))
```

Required environment name: `PROVIDER_API_KEY`.
