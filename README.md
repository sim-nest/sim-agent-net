# sim-agent-net

Turn SIM into a networked host for models and agents: expose its tools over MCP,
route language-model calls, and place work across machines -- all as ordinary
runtime objects.

## Try it

If you have the `sim` binary (`cargo install sim-nest --features serve-cli`),
start a Model Context Protocol server so an MCP client -- an editor, an agent --
can call SIM tools over stdio:

```shell
sim mcp
```

It serves the SIM tool surface over stdio for any MCP client to connect to. This
is the same surface built by the `sim-mcp-server` binary in this repo. Full
walkthrough of every surface: see sim-say.

## What's here

`sim-agent-net` is a SIM constellation repository: the server, agent, and
model-fabric substrate of the SIM runtime. It holds the library crates that
turn networked services, autonomous agents, and language-model access into
ordinary SIM runtime objects, plus the MCP and OpenAI-compatible gateway
surfaces and the `sim-mcp-server` binary.

SIM is an expandable Rust runtime built around a small protocol kernel
(`sim-kernel`) plus a large set of loadable libraries. The kernel defines
contracts; libraries provide behavior. Everything in this repository is library
behavior installed on top of the kernel, not kernel-special code.

## How it works

### Architecture rule: location-transparent eval

The single most important rule for everything in this repo: **the public
distributed eval surface is location-transparent through `realize` and
`EvalFabric`, never transport-specific APIs.** Server code and agent code target
the `EvalSite` / `FabricEvalSite` abstraction and the `server:realize` operation
so that a call resolves the same way whether it runs in-process, on a coroutine,
across a pipeline, or over a remote transport. New surfaces should route through
these abstractions rather than reaching for a concrete socket, HTTP client, or
process pipe directly.

The kernel owns the `realize` / `EvalFabric` / `Card` protocol types; this repo
provides the library sites and operations that implement and compose them
(`EvalSite`, `LocalEvalSite`, `CoroutineEvalSite`, `PipelineEvalSite`,
`LoopEvalSite`, `FabricEvalSite`, and the gitignored remote transports).

## Crates

Server and distributed-eval substrate:

- `sim-lib-server` -- server runtime, frame router, transports, REPL/trigger
  surfaces, and the `EvalSite` family that implements location-transparent
  `realize`. Exports `Server`, `ServerRuntime`, `FrameRouter`, `ServerAddress`,
  `EvalSite` and its concrete sites, transport adapters, and stream-frame
  helpers.
- `sim-lib-stream-fabric` -- event/frame adapters for remote STREAM realization.
  Converts `StreamValue` spines into chunk events or server stream frames and
  back, without adding new server frame kinds or kernel object hooks
  (`realize_stream_events`, `stream_realize_request`, `stream_to_frames`).
- `sim-table-remote` -- a remote table site (`RemoteTableSite`, `RemoteDir`)
  that projects a remote directory as a SIM table value over the same fabric.

Agent and model fabric:

- `sim-lib-agent-runner-core` -- provider-neutral runner contracts: the
  transcript objects (`ModelRequest`, `ModelResponse`, `ModelUsage`), the
  streaming surface (`ModelEvent`, `ModelEventSink`, `VecEventSink`), the
  routing/selection objects (`ModelCard`, `ModelBid`), and the executable
  `ModelRunner` trait.
- `sim-lib-agent` -- the agent runtime stack: agent and swarm objects,
  component installation, memory/tool/role support, the concrete runner
  installs, markets, debate flows, tool injection, model-privacy enforcement,
  and agent-as-model adapters. Re-exports the runner-core contracts.
- `sim-lib-agent-runner-http` -- HTTP/HTTPS-backed runners (`HttpRunner`) for
  OpenAI-compatible and Ollama endpoints, including SSE and NDJSON streaming.
- `sim-lib-agent-runner-process` -- local subprocess-backed runners
  (`ProcessRunner`, `ProcessProtocol::{JsonStdio, LineText}`) under capability
  checks, including incremental line-text streaming.
- `sim-lib-agent-runner-local` -- loadable local model placement sites. The
  deterministic default registers `model-site:local` with no model files;
  optional native and wasm paths stay capability-gated and isolated in this
  backend crate.

Skills, tools, and gateways:

- `sim-lib-skill` -- the skill object surface: `SkillCard`, `SkillCallable`,
  skill policy/privacy/role records, browse metadata publication, and optional
  MCP / OpenAI / runner projections behind features.
- `sim-lib-mcp` -- library-only MCP projection. Projects native browse Cards and
  optional `SkillCard` records into redacted `McpSurfaceCard` rows; routing,
  transport, and callable execution layer on top behind features.
- `sim-lib-openai-server` -- an OpenAI-compatible gateway skeleton: health
  route, OpenAI JSON transcript codec, model discovery, `POST /v1/responses`,
  `POST /v1/chat/completions`, `POST /v1/embeddings`, fixtures, SSE streaming
  projection, and stored-response replay/fork routes.
- `sim-lib-cookbook` -- runtime `cookbook:` operations over a shared recipe
  store; the recipe engine itself stays in the kernel-free `sim-cookbook`
  crate.

Binary:

- `sim-mcp-server` -- a stdio MCP server binary. HTTP transport is disabled in
  this binary.

## Model fabric contract

The model fabric turns language-model access into normal SIM runtime surfaces.
It is implemented across `sim-lib-agent-runner-core` (the stable contract) and
`sim-lib-agent` (concrete runners, markets, policy, injection), with HTTP and
process runners in their dedicated crates. Like everything here, model access
is ordinary capability-gated library behavior, not kernel-privileged.

### Runtime objects

The stable transcript contract is defined by `sim-lib-agent-runner-core` and
exposed as ordinary SIM values that can be encoded, decoded, browsed, and passed
through runtime dispatch.

Request/response objects:

- `ModelRequest` -- task expression, message transcript, and open extra fields.
- `ModelResponse` -- runner id, model name, structured content, stop reason,
  usage, and open extra fields.
- `ModelUsage` -- input/output token counts, latency, cost, and open extra
  fields.

Streaming/event objects:

- `ModelEvent` -- event kind, runner, model, span id, optional final response,
  and open extra fields. Helper constructors cover `start`, text delta, usage,
  error, tool-call, and synthesized final events.
- Runners emit through `ModelEventSink`. Live agent/server streaming adapts that
  sink with a stream-event sink that writes `StreamStart`, `StreamChunk`, and
  `StreamEnd` frames. Model-event streams are browseable as
  `stream/data/model-event` compatibility data. `VecEventSink` is the in-memory
  sink for tests and buffered callers.

Selection objects:

- `ModelCard` -- runner identity, model, provider, locality, open extension
  fields.
- `ModelBid` -- availability, reason, score, chosen model, open extension
  fields.

### Transcript codecs and projections

Runners do not all speak one remote wire format. The stable contract is that
installed runners can project into and out of the shared in-process request,
response, and event shapes:

- `codec:chat` -- installed runtime codec for normalized chat-style transcript
  values (the engine lives in the `sim-codec-chat` crate).
- `codec:openai` -- provider projection used by the HTTP runner for
  OpenAI-compatible request/response envelopes.
- `codec:ollama` -- provider projection used by the HTTP runner for Ollama
  `/api/chat` and NDJSON stream helpers.

### Runners

Installed runner surfaces:

- `runner/echo` -- mirrors inputs for smoke tests and contract inspection.
- `runner/fake` -- deterministic canned responses for tests.
- `runner/cassette` -- replays recorded request/response sessions (a fixture and
  provenance mechanism, distinct from the request-time model cache).
- `runner/process` -- shells out to external adapters under capability checks;
  line-text streaming reads stdout lines while the subprocess runs and emits
  incremental delta events.
- `runner/openai-compatible` -- targets OpenAI-style HTTP or HTTPS
  chat/completions APIs; HTTPS is enabled by the optional root feature
  `agent-runner-http-tls`; streaming consumes SSE `data:` chunks and emits live
  `model-event` deltas.
- `runner/ollama` -- targets native Ollama HTTP chat endpoints without a proxy;
  streaming consumes native NDJSON chunks and emits the same `model-event`
  schema.
- `runner/market` -- selects among candidate runners using cards and bids.
- `runner/agent` -- exposes an agent as a model endpoint.
- `runner/debate` -- coordinates multiple model passes into a debate result.

Runners are installed through agent component wiring, not kernel-special model
behavior. New runners implement the shared `ModelRunner` contract; new selection
policies work through cards and bids rather than closed enums.

### ModelSite placement

Model placement is a catalog lookup at realize time. `runner/place` registers an
in-process runner under a `model-site:*` key, while loadable local backends export
site values such as `model-site:local` and `model-site:local-wasm`. `model/at`
returns an `EvalFabric` that holds only the key, resolves the current site, and
delegates the request through the same `ModelRunner` transcript contract.

The same prompt graph can realize against fake, local, or remote placements by
changing only the key. `model/sites` and `model/site-card` expose placement cards
for browsing, and the OpenAI-compatible `GET /v1/models` route lists models from
registered runner cards, including the loadable local model card when present.

### Model cache

A `ModelRequest` may carry a `cache` policy map, treated as execution policy
rather than prompt content:

- Stable keys normalize map order and include runner id, selected model,
  normalized request content, result shape, request capabilities, and an
  optional `semantic-key`.
- Read-through, read-only, write-only, refresh, and disabled modes are
  supported. `ttl` uses the shared duration parser; stale entries are ignored.
- Hits return ordinary `model-response` transcripts with `cache-hit true`;
  misses return live responses with `cache-hit false` when caching is active.
- Tool continuations are not cached unless the request or cache policy marks them
  `idempotent true`.
- In-memory writes need no new capability; persistent writes through `path`,
  `file`, or `journal` require `ai-runner-cache`.
- `model/cached` wraps any placement or runner `EvalFabric` with an
  `EvalCassette`, so successful inference replies become content-addressed
  effects keyed by normalized prompt, model identity, parameters, result shape,
  and capabilities.

### Agent tool injection

When a started agent receives a `model-request` and has manifest runners, it
routes the request to its first manifest runner and can inject descriptors for
manifest tools into the request `tools` field before routing:

- Descriptors are generated from existing `Tool` metadata (name, description,
  args/result shape, category, required capabilities).
- Manifest tools are the default allow set; request fields `allowed-tools`,
  `denied-tools`, and `tool-policy` narrow it. Explicit transcript tools are
  preserved when allowed and non-conflicting; out-of-manifest or denied
  descriptors are rejected.
- Injection only describes tools; actual calls still use the bounded runner tool
  loop and `Tool::call_values`. Injected descriptors are recorded under
  `agent-tool-injection` and traced before runner execution, so a
  recorder/cassette journal captures the same descriptor-bearing request that
  reached the runner.

### Privacy policy

A `ModelRequest` may include a `privacy` policy, enforced at runner, market,
recorder, provider raw-log, and tool-loop boundaries:

- `local-only` -- rejects network-locality runners and remote-like component
  addresses before provider request encoding.
- `metadata-only` -- stores recorder trace payload hashes plus safe metadata
  (usage, model id, stop reason, cache flag, market routing decisions).
- `no-raw` -- suppresses raw provider payload capture even when
  `ai-runner-raw-log` is granted, and rejects `raw-ref` request content.
- `allow-tools` -- limits which declared tools may receive prompt-derived tool
  calls before `Tool::call_values` executes.

### Market and selection

Market runners apply privacy filters before candidate bidding or realization and
record privacy accept/reject decisions in `market-decision`. Market policy
supports `race`, `speculate`, `debate`, and `escalate` execution modes, each
recording accepted bids, executable candidates, branch runs, cancellations,
failures, fallback use, verification, selected runner, and selected response as
ordinary transcript data. The selection path:

1. request or agent policy describes desired capabilities;
2. available runner cards advertise capabilities and cost/latency hints;
3. market selection computes bids and executable candidates;
4. the selected execution mode runs, cancels, verifies, or escalates branches;
5. the winning response carries a codec-round-trippable `market-decision`.

### Agent-as-model

`runner/agent` lets an agent surface participate in the same selection and
invocation path as a direct model runner. A policy may route to a remote model,
a local process model, or another agent; with the `skill-runner` feature, also
to a model-role `SkillCard` projected through `skill/as-runner`. Cards and
bidding compare these options uniformly, and debate/market flows can combine
agent and direct model endpoints.

### Helper surfaces

- `model-policy` -- policy and preference object helpers.
- `runner/card` / `runner/cards` -- inspect one or enumerate registered runner
  cards.
- `runner/health` -- summarize runner health state.
- `skill/as-runner` -- with the `skill-runner` feature, project a model-role
  `SkillCard` into the external runner contract.

## Capabilities

Model and network access is capability-gated; it is not kernel-privileged.
Relevant capabilities exposed by `sim-lib-agent` include `ai-runner`,
`ai-runner-network`, `ai-runner-local`, `ai-runner-secret`, `ai-runner-cache`,
and `ai-runner-raw-log`. Request metadata stays explicit and object-visible
rather than hidden in side channels.

## Validation profiles

These commands run in the constellation workspace; only `sim-kernel` builds from a lone clone today (see `DEVELOPING.md` in `sim-sdk`). A single-repo build lands with the first crates.io publish.

```bash
cargo fmt --check && cargo test --workspace && cargo clippy --workspace -- -D warnings && cargo doc --workspace --no-deps
cargo run -p xtask -- simdoc --check
```

The model-fabric validation profiles are scripted and environment-gated:

- **CI profile** (`./scripts/validate.sh`, `make ai-sanity`, `make
  ai-sanity-ci`) stays network-free and uses mock, fake, codec, and
  loopback-free runner coverage only. It requires no model daemon, network,
  local-runner, or secret capability.
- **Local smoke** (`make ai-sanity-local`) probes the documented Ollama target
  matrix plus an optional loopback OpenAI-compatible endpoint and skips
  unavailable targets cleanly. It corresponds to granting `ai-runner`,
  `ai-runner-network`, and `ai-runner-local`; raw provider JSON stays off by
  default.
- **Hosted smoke** (`make ai-sanity-hosted`) calls an explicitly configured
  OpenAI-compatible HTTPS endpoint and checks that the secret value is not
  echoed in failure output. It corresponds to granting `ai-runner`,
  `ai-runner-network`, and `ai-runner-secret` with the `agent-runner-http-tls`
  feature enabled.

The default local runner target matrix:

| Target      | Model variable                    | Endpoint variable                    |
| ----------- | --------------------------------- | ------------------------------------ |
| `local-a`   | `AI_SANITY_LOCAL_A_MODEL`         | `AI_SANITY_LOCAL_A_ENDPOINT`         |
| `local-b`   | `AI_SANITY_LOCAL_B_MODEL`         | `AI_SANITY_LOCAL_B_ENDPOINT`         |
| `local-c`   | `AI_SANITY_LOCAL_C_MODEL`         | `AI_SANITY_LOCAL_C_ENDPOINT`         |
| `reserved`  | `AI_SANITY_RESERVED_MODEL`        | `AI_SANITY_RESERVED_ENDPOINT`        |

Operators override these with `AI_SANITY_TARGETS`,
`AI_SANITY_LOCAL_A_ENDPOINT`, `AI_SANITY_LOCAL_B_ENDPOINT`,
`AI_SANITY_LOCAL_C_ENDPOINT`, `AI_SANITY_RESERVED_ENDPOINT`, and the matching
`*_MODEL` variables. Optional local OpenAI-compatible smoke uses
`AI_SANITY_LOCAL_OPENAI_ENDPOINT`, `AI_SANITY_LOCAL_OPENAI_MODEL`, and
`AI_SANITY_LOCAL_OPENAI_API_KEY_ENV`. Hosted smoke uses
`AI_SANITY_HOSTED_ENDPOINT`, `AI_SANITY_HOSTED_MODEL`, and
`AI_SANITY_HOSTED_API_KEY_ENV` (default `PROVIDER_API_KEY`).

## Documentation lanes

`cargo run -p xtask -- simdoc` builds the public documentation lanes (API docs
under `target/doc/`, agent cards under `docs/agents/`, human docs under
`docs/humans/`, diagrams under `docs/diagrams/`) and the split contract files
under `docs/generated/`. Everything under `docs/` is generated and must not be
hand-edited.

## Relationship to the kernel

This repo depends on `sim-kernel` for the `realize` / `EvalFabric` / `Card`
protocol types, `Cx`, `Lib`/`Linker`/`LibManifest`, `Symbol`/`Value`/`Expr`,
capabilities, and stable ids. It keeps the kernel small: concrete runners,
markets, debate, gateways, MCP, and remote transports all stay here as library
behavior. Browse/help/test exposure is published as ordinary library metadata
(for example skill browse claims and `McpSurfaceCard` projection) rather than
through kernel changes.
