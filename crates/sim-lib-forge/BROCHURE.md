# sim-lib-forge

In one line: It turns reusable model tasks into named packet artifacts that can be checked before they are trusted.

## What it gives you

This crate defines the artifact that FORGE stores after plain prose becomes a BRIDGE packet, and it provides the guarded one-shot lift that creates the first candidate. The record keeps the intent name, version, source content, packet content, semantic probes, verifier identities, compiler provenance, and approval state together in one small package. That makes a compiled task something SIM can catalog, inspect, compare, and reuse without asking a model to lift the same prose again.

## Why you will be glad

- A lifted prompt stays a candidate until its packet and return shape actually check.
- Human approval is separate from automated verification, so trusted reuse has a clear line.
- Repeated work can point at stable content ids instead of depending on fresh model wording.

## Where it fits

This FORGE library sits above the BRIDGE packet codec and runtime guard, using their packet identity as the compiled program while adding the reusable intent record and lift gate that verification and routing layers build on.
