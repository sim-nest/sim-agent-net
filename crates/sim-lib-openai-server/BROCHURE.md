# sim-lib-openai-server

In one line: A gateway that lets tools built for OpenAI's API talk to SIM instead.

## What it gives you

Many programs already know how to speak to OpenAI's web interface. This crate lets SIM answer on that same interface, so those programs can point at SIM without being rewritten. It handles the familiar endpoints for chat and responses, for embeddings, and for audio, image, and vector-store requests, translating between OpenAI's message format and SIM's own. Answers can stream back as they are produced, past responses can be fetched again by their id, and a stored response can be replayed or branched for inspection. The model listing reports the models SIM actually has available, so local, fixture, and remote model entries all show up together when they are installed.

## Why you will be glad

- Existing OpenAI-compatible apps and libraries can use SIM with little or no change.
- One listing shows every model SIM can reach, so callers see local and remote options side by side.
- Saved responses can be fetched, replayed, and branched, which makes reviewing and debugging far easier.

## Where it fits

This crate is SIM's front door for the OpenAI-style world. It sits at the network edge, receives requests in that widely used shape, and hands them to SIM's agents and models through the runtime's own contracts. It lets SIM join an existing ecosystem of clients rather than asking every one of them to learn something new.
