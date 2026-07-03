# Planning Repair Trace

This recipe records deterministic goal decomposition and plan repair as SIM
data. The setup quotes a goal, the ordered subtasks produced by `decompose`, a
bounded repair decision, and the final subtask order after the repair.

The fixture uses a fake runner and synthetic observations only. It gives recipe
browsers a stable planning example without network access or live model calls.
