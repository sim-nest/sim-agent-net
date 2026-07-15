# sim-lib-forge

In one line: It gives reusable model tasks a named record that can be checked before it is trusted.

## What it gives you

This crate defines the artifact that FORGE stores after plain prose becomes a BRIDGE packet. The record keeps the intent name, version, source content, packet content, semantic probes, verifier identities, compiler provenance, and approval state together in one small package. That makes a compiled task something SIM can catalog, inspect, compare, and reuse without asking a model to lift the same prose again.

## Why you will be glad

- A lifted prompt stays a candidate until its checks actually pass.
- Human approval is separate from automated verification, so trusted reuse has a clear line.
- Repeated work can point at stable content ids instead of depending on fresh model wording.

## Where it fits

This is the first FORGE library in the agent network. It sits above the BRIDGE packet codec and runtime guard, using their packet identity as the compiled program while adding the reusable intent record that later lift, verification, and routing layers build on.
