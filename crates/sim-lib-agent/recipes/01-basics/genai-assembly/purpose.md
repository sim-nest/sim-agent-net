This recipe assembles one checked generative exchange from reusable runtime
parts. The host places a model target at `model-site:genai`; the recipe builds a
BRIDGE ASK packet, sends it through the late-bound placement, requires a JSON
answer, and admits only a `core/String` reply.

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
