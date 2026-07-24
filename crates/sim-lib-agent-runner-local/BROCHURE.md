# sim-lib-agent-runner-local

In one line: The piece that lets SIM run a model right on your own machine, in the same process.

## What it gives you

This crate installs a modeled local runner inside SIM, registered under a local placement so agents can send work to it. It needs no model files and makes no outside contact, which makes it a safe, predictable default for trying things and for tests where you want the same answer every time. Real local execution uses the subprocess runner, loopback HTTP runner profiles, or a sandboxed wasm model guest loaded only after capability checks pass. The runner announces what it offers, including whether it can stream and whether replies can be reused.

## Why you will be glad

- A modeled local choice is available with no downloads and no network, ideal as a starting point.
- Predictable default behavior makes tests repeatable instead of flaky.
- Process, loopback HTTP, and sandboxed model guests are opt-in, so you add weight only when you need it.

## Where it fits

This is the deterministic in-process member of SIM's family of model runners. It plugs into the placement catalog so an agent can direct work to a modeled site on the same machine, the same way it would direct work to a web-hosted, subprocess, loopback HTTP, or wasm-backed model. It keeps real local execution behind explicit runner and capability gates, so the core stays clean and you stay in control of what runs.
