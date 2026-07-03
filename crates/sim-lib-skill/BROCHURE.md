# sim-lib-skill

In one line: The system that describes each thing an agent can do and lets it call that ability like any other command.

## What it gives you

A skill is any capability an agent can reach for: a tool, a model, a stored memory, a retriever, a judge that scores answers, a router that picks a path, and more. This crate gives every skill a clear description that pairs a stable name with the shape of what it takes in and gives back, along with rules about caching, recording, and privacy, and the backend that actually runs it. Skills are gathered into a registry and then behave like ordinary callable objects, so an agent invokes an ability the same simple way whether it lives in a fixture, a web service, a local program, or another assistant. Optional pieces select which backends and integrations you want.

## Why you will be glad

- Every ability an agent has is described in one consistent form, so its inputs and outputs are clear.
- An agent calls a tool, a model, or a memory the same way, no matter where that ability really runs.
- Rules for caching, recording, and privacy travel with each skill, so handling stays consistent.

## Where it fits

This crate is how abilities enter an agent's reach in SIM. It turns described capabilities into callable runtime objects and registers the commands to install, list, inspect, and call them. The specific backends, a local program, a web service, a protocol server, live in sibling crates and slot in behind these descriptions, so the agent sees one uniform set of skills.
