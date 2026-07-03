# sim-lib-agent

In one line: The part of SIM that lets an assistant use tools, remember things, and pick which model answers.

## What it gives you

This is the home base for agents inside SIM. It brings together the pieces an assistant needs to do real work: the tools it can call, the memory it keeps between turns, the patterns that shape its behavior, and the fixtures used to try things out safely. It also decides where a model request actually runs, choosing from a catalog of placements you have set up, whether that is a model on your own machine or one reached across a network. When a request has been answered once, it can keep that reply and hand back the same result for the same question, so you are not paying to ask twice. Everything stays fair and orderly when many requests arrive at once.

## Why you will be glad

- Your assistant can hold context and use tools instead of answering each message from a blank slate.
- Repeated questions come back from a saved reply, saving you time and cost.
- You choose where model work lands, on your hardware or elsewhere, without rewiring the assistant.

## Where it fits

This crate sits at the center of SIM's agent story. Other parts of SIM supply the actual models, servers, and skills; this one gathers them into a working agent and connects it to the place where a model reply is produced. If you are building an assistant on SIM, this is the piece that makes an agent an agent, and it keeps model choice and reply reuse out of the core runtime.
