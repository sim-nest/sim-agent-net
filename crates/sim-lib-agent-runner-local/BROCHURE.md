# sim-lib-agent-runner-local

In one line: The piece that lets SIM run a model right on your own machine, in the same process.

## What it gives you

This crate installs a model runner that lives inside SIM itself, registered under a local placement so agents can send work to it. Out of the box it needs no model files and makes no outside contact, which makes it a safe, predictable default for trying things and for tests where you want the same answer every time. When you do want real on-device inference, an optional build brings in native model support, kept fenced off in one clearly marked spot. Another optional build can host a model packaged as a sandboxed guest, loaded only after capability checks pass. The runner announces what it offers, including whether it can stream and whether replies can be reused.

## Why you will be glad

- A local model choice is available with no downloads and no network, ideal as a starting point.
- Predictable default behavior makes tests repeatable instead of flaky.
- On-device inference and sandboxed model guests are opt-in, so you add weight only when you need it.

## Where it fits

This is the in-process member of SIM's family of model runners. It plugs into the placement catalog so an agent can direct work to a model on the same machine, the same way it would direct work to a web-hosted or subprocess model. It keeps native and sandboxed inference behind clear capability gates, so the core stays clean and you stay in control of what runs.
