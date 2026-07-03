# sim-mcp-server

In one line: A ready-to-run program that lets other assistants reach SIM through the common Model Context Protocol.

## What it gives you

This crate is a standalone program that puts SIM on the other end of the Model Context Protocol, the shared way assistants discover and call tools. You start it, it reads its options, and it builds a running SIM with the protocol's codec and library already in place, then serves that protocol over a standard input and output channel. The result is a single command that exposes SIM's tools and skills to any assistant able to speak this protocol, with no extra assembly on your part. It is the packaged, launch-and-go form of SIM's protocol support, meant to be pointed at and used rather than built up from parts.

## Why you will be glad

- One command turns SIM into a protocol server that other assistants can connect to right away.
- The runtime arrives with the protocol codec and library already installed, so there is nothing to wire up.
- It speaks over a plain input and output channel, which is easy to launch and connect to from a host.

## Where it fits

This crate is the finished, runnable front for SIM's protocol support. The listing and library pieces that shape and describe what SIM offers live in sibling crates; this one binds them into an executable and serves them over the wire. When you want an assistant elsewhere to use SIM's tools, this is the program you actually run.
