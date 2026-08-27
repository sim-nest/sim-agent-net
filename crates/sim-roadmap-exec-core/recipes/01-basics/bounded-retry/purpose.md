# Admit one bounded retry

Classify failure ownership, require resolved effects and unchanged protected
identities, then admit at most one policy-named retry step. The caller owns every
subsequent repetition.

This is a sandbox descriptor because retry admission is a typed execution-core
transition over protected identities and effect evidence. Focused crate tests
execute accepted and hostile transitions.
