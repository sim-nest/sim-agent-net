# sim-lib-server

In one line: The part of SIM that serves evaluation and agents to callers and streams the replies back.

## What it gives you

This crate lets SIM act as a service. A caller can ask it to evaluate work, receive replies as a stream, and run agent loops, all across the runtime. It provides the places where work can run, whether locally, as a coroutine, through a pipeline, in a loop, or spread across a fabric, along with the routing, connections, and conversational drivers that carry requests in and answers out. By default it stays entirely in-process and opens no network at all, which keeps a plain setup simple and closed. When you actually want to accept requests over the network, an HTTP transport is available as an explicit opt-in, and inbound triggers are added the same deliberate way.

## Why you will be glad

- You can turn SIM into a running service that answers requests and streams results back.
- The safe default opens no network, so nothing is exposed until you choose to expose it.
- Network access and inbound triggers are opt-in, giving you clear control over the surface you offer.

## Where it fits

This crate is SIM's serving layer. It connects the runtime to callers, whether those callers sit in the same process or arrive over a transport, and it is the natural target for agent loops that need to run and reply continuously. Other crates supply the specific gateways and models; this one provides the general machinery for serving and streaming underneath them.
