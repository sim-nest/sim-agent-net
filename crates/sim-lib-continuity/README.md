# sim-lib-continuity

Pure, codec-stable continuity planning for intermittent sessions. The crate
validates plans, reduces ordered events into intents, and persists accepted
transitions through a caller-provided fenced journal. It performs no host,
network, device, audio, model, view, or clock effects.
