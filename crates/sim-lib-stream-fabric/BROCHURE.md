# sim-lib-stream-fabric

In one line: The layer that lets SIM place work on other machines and share streaming results without minding where they run.

## What it gives you

This crate lets SIM spread evaluation across a group of machines while the code asking for that work stays unaware of where it happens. It carries streams between peers, packing them into bounded messages for the trip and unpacking them on arrival, with control signals to open, advance, pause, cancel, and report on each stream. Its central idea is simple: a finished reply is saved by the exact question that produced it, so any machine that already holds that answer can serve it, and asking becomes a matter of finding who holds a given result rather than tracking a routing map. Because a saved answer never changes, the record of who holds what needs no central agreement and heals itself by replaying its log. Sensitive inputs are hidden before work is placed elsewhere.

## Why you will be glad

- Work can run on whichever machine is able to take it, without your code pinning down a location.
- An answer computed once is reused across the whole group, so the same expensive work is not repeated.
- The group needs no central coordinator to agree on who has what, which keeps it simple and resilient.

## Where it fits

This crate is SIM's location-transparent evaluation layer for streams, the surface that server and agent code aim at instead of wiring to a specific machine. It stays in library space and adds no new low-level frame kinds or kernel hooks. When you want a fleet of SIM nodes to share streaming work and reuse each other's results, this is the piece that makes them act as one.
