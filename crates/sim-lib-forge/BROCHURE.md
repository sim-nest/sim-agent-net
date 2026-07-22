# sim-lib-forge

In one line: It compiles a plain-English task into a reusable, checked model program you can trust and cache instead of re-prompting.

## What it gives you

FORGE turns prose into a named, verified artifact. You hand it a task in ordinary language; it lifts that prose into a BRIDGE packet, confirms the packet is well-formed and types its own output, then checks the answer contract with real verifiers before the result is trusted. A task that passes becomes a golden artifact with a stable identity, so the next caller fetches the compiled program instead of asking a model to interpret the same words again. It also routes work to a cheap model first and only escalates when a check fails, and it can measure whether compiling actually helped.

## Why you will be glad

- A lifted task stays a candidate until its packet, its output shape, and its answer verifiers all agree.
- Repeated work points at a stable golden artifact, so the same intent is not re-prompted or re-worded every run.
- A cheap model can do the job safely because a real checker, not blind trust, decides whether to accept or escalate.
- You can see, in numbers, the accuracy and cost of running raw prose versus a compiled, cached, downshifted intent.

## Where it fits

This is the FORGE layer on top of the BRIDGE packet codec and runtime guard. It uses BRIDGE packet identity as the compiled program and the BRIDGE checker as the trust boundary, adding the lift, verification, reusable intent library, cost-aware routing, evaluation harness, and the `sim forge` command. It stays honest that the compiler is itself a model: nothing is trusted until it checks, and precision and speed come from the envelope and the cache, not from expecting the model to be reliable.
