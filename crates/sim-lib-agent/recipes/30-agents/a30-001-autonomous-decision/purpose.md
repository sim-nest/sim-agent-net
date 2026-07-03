# Autonomous Decision Trace

This recipe records a deterministic autonomous decision as SIM data. The setup
quotes an `agent-pattern` over a synthetic perception context, applies a
weighted policy score table, chooses an action, and keeps a ledger trace that
ties the inputs to the action record.

The fixture runs with a fake runner and synthetic inputs only. It is suitable
for browsing, recipe execution checks, and examples that need an autonomous
decision without live model access.
