# sim-lib-provider-cli

In one line: Loadable provider command surface for the SIM bootloader.

## What it gives you

Operator-facing parsing and rendering for provider inventory and status without copying registry, authentication, probing, opening, or fan-out behavior into the bootloader. Output is bounded and safe for terminals and automation. CLI behavior stays a thin projection over one provider authority. Credentials and private transport details never enter inventory output. New provider adapters remain discoverable without bootloader changes. This crate is the command surface for sim-lib-provider. The bootloader loads it; provider policy and installed adapters remain with their existing owners. The contract keeps inputs, outputs, limits, and refusal cases explicit, so callers can compose the capability without acquiring unrelated host, transport, or product authority. Stable records make the result suitable for tests, inspection, and deterministic integration.

## Why you will be glad

- The public contract makes supported behavior, limits, and typed failures visible before integration.
- One owning crate prevents neighboring libraries from growing competing copies of the same policy.
- Deterministic records and checked tests keep adapters reviewable when implementations evolve.

## Where it fits

Within SIM, sim-lib-provider-cli owns only the focused contract described above. Adjacent runtime libraries, platform adapters, codecs, and user surfaces can build around it while retaining their own policy. That boundary keeps the kernel small, avoids competing implementations, and lets this capability evolve without forcing unrelated components to change.
