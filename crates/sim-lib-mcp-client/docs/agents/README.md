# Agent contract

Probe before the first application operation. Do not retry an application
operation to change eras. Preserve opaque `requestState` exactly, use a fresh
request id, and enforce input capability, round, byte, count, cancellation, and
deadline limits. Cache only complete codec-eligible non-effecting results.
