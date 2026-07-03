# sim-lib-agent-runner-http

In one line: The connector that lets SIM reach models offered over the web, including OpenAI-style and Ollama services.

## What it gives you

This crate lets SIM talk to a model that lives behind a web address. You point it at a service, and it handles the back-and-forth: sending your request, reading the reply, and passing along partial text as it streams in so you do not wait for the whole answer. It understands the common shapes used by OpenAI-compatible endpoints and by Ollama, so many popular hosted and self-run model services work without special glue. It also takes care to keep sensitive parts of a request, such as keys and private content, from leaking into logs and traces. From SIM's point of view, a web-hosted model becomes just another runner it can call.

## Why you will be glad

- You can use hosted or self-run model services from SIM without writing network code yourself.
- Answers stream in as they are generated, so long replies feel responsive.
- Sensitive request details are kept out of logs, reducing the chance of an accidental leak.

## Where it fits

This is one of SIM's model runners, the kind that reaches a model across the network. It follows the shared runner contract, so anything that can call a runner can call a web-hosted model through this crate. Sibling crates cover models that run in the same process or as a local subprocess; this one covers the ones you reach over the web.
