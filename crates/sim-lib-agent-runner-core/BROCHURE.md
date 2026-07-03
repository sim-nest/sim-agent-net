# sim-lib-agent-runner-core

In one line: The shared rulebook that lets any model, from any provider, plug into SIM the same way.

## What it gives you

This crate sets the common ground that every model connection in SIM agrees on. It describes what a request to a model looks like, what a finished reply looks like, and how progress can arrive piece by piece while an answer is still being written. It also describes how a model announces what it can do and how the system picks one model over another when several could answer. None of this is tied to a particular company or service. Because the shape is fixed and neutral, one place can swap a local model for a hosted one, or route between several, without any of the surrounding code needing to learn a new set of terms.

## Why you will be glad

- Every model speaks the same language to SIM, so switching providers does not ripple through your setup.
- Streaming replies arrive as they are produced, so you see progress instead of waiting for the whole answer.
- Selecting the right model for a task is a plain, described choice rather than a tangle of special cases.

## Where it fits

This is the foundation layer under every SIM model runner. The crates that actually talk to a service over the network, run a local subprocess, or host a model in the same process all build on the shapes defined here. It defines the contract and stays out of the way; the concrete work of reaching a model lives in the crates above it.
