# sim-lib-provider

In one line: Discover and select every model-provider seat without coupling provider setup to an agent.

## What it gives you

Open family and seat cards describe provider identity, principals, endpoints,
harnesses, and limits without baking a vendor list into SIM. A narrow adapter
contract discovers available seats and opens a selected seat through the shared
model runner interface.

## Why you will be glad

- Provider setup can be reused by commands, services, and applications that do not assemble agents.
- New provider families fit the same records without changing a central enum.
- Execution continues through one provider-neutral model request and response contract.

## Where it fits

This crate sits above the model runner core and below concrete HTTP, process,
broker, and CLI adapters. It owns provider control records, not transport or
inference behavior.
