# sim-lib-mcp

In one line: The piece that turns SIM's own tools and skills into safe listings other assistants can discover.

## What it gives you

The Model Context Protocol is a common way for assistants to find out what tools and resources are available to them. This crate takes SIM's internal catalog, its browse cards and its skill descriptions, and presents each one as a tidy, protocol-shaped listing. As it does so it strips out details that should not be shared, so what goes out is a redacted, presentable summary rather than raw internals. The result is a clean inventory of what SIM can offer to an outside assistant, described in terms that assistant already understands, without exposing the machinery behind each entry.

## Why you will be glad

- SIM's tools and skills become visible to any assistant that speaks this common protocol.
- Sensitive internals are filtered out before anything is shared, keeping private details private.
- You describe your capabilities once in SIM and they appear in a standard, discoverable form.

## Where it fits

This crate is the presentation layer for SIM's protocol-facing catalog. It focuses on one job: turning native cards and skills into redacted, standard listings. The work of routing calls, carrying them over a transport, and actually running the chosen tool belongs to the protocol layers built on top; this piece prepares the menu they hand out.
