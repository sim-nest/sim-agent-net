# Retrieval Radar Standard Crate

This recipe records a deterministic Retrieval Radar explanation over a standard
crate. The fake runner loads a local index, ranks three operation hints, and
stores the cassette hash used by replay checks.

The fixture is synthetic and offline. It gives recipe browsers a stable Radar
trace with confidence-scored hints and no live model or network access.
