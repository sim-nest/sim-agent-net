# Runtime Operation Edit Loop

This recipe records a small runtime-operation edit as deterministic SIM data.
The fake runner carries the Rust docs note, Codec Prism round trip, generated
docs check, validation command, and pin plan as a single replayable trace.

The fixture uses synthetic descriptors only. It exercises the edit loop without
writing source files, contacting a model, or using network access.
