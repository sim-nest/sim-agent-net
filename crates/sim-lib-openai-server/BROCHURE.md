# sim-lib-openai-server

In one line: A gateway that lets OpenAI-shaped clients reach SIM's model and agent surface.

## What it gives you

Many programs already know the request and response shapes used by OpenAI-style model services. This crate gives SIM a gateway in that shape for chat, response, embedding, model listing, streaming, replay, and inspection flows. It also includes SIM fixture and subset routes for file records, audio transcription and speech, image references, and text-vector search, so tests and demos can exercise those object families without pretending to be a full media or multipart provider. Stored responses can be fetched again by id, replayed, or branched for inspection. The model listing reports the models SIM actually has available, so local, fixture, and remote model entries appear together when they are installed.

## Why you will be glad

- Clients that use the shipped OpenAI-shaped JSON flows can point at SIM without learning a separate gateway shape.
- The media and vector routes are honest fixtures and subsets, so tests get deterministic behavior instead of hidden provider assumptions.
- Saved responses can be fetched, replayed, and branched, which makes reviewing and debugging easier.

## Where it fits

This crate is SIM's network front door for OpenAI-shaped clients. It receives supported gateway requests, translates them into SIM's runtime contracts, and returns objects that those clients can consume. It connects familiar client tooling to SIM while keeping provider-specific behavior explicit.
