# sim-lib-bridge

In one line: It keeps model-facing BRIDGE packets from leaving or entering SIM unless their local checks agree.

## What it gives you

This crate is the runtime guard around BRIDGE packets. Before a packet is sent, it stamps a stable identity, proves the line form can be read back, checks that every byte belongs to the packet, and runs the same receive checks the other side will use. When an answer comes back, it reads the final model content as another BRIDGE packet and checks the move, parts, capabilities, and declared return contract before accepting it.

## Why you will be glad

- A malformed packet is stopped before it reaches a model seat.
- Replies are judged against the request that asked for them, not loose text.
- Repeated model work can be replayed by content identity instead of run twice.

## Where it fits

This is the BRIDGE runtime member of the agent network. It uses the packet codec from the codec family and the provider-neutral runner contracts from the runner core, then targets the normal location-transparent evaluation surface. It does not own transports or profiles; those stay in the surrounding libraries.
