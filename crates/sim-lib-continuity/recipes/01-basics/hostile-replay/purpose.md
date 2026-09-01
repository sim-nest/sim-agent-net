# Hostile replay

This network-free specimen starts with an empty derived cache and rebuilds the
same continuity state from the fenced journal contract. Crate tests extend the
trace with duplicate, reordered, stale, cancellation, and post-cancel events.
