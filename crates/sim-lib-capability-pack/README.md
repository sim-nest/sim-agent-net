# sim-lib-capability-pack

This crate turns a room-like runtime composition into immutable, versioned
data. A `PackDir` supplies objects by canonical SHA-256 content id; `resolve`
produces one dependency-first closure and intersects authority at every import
edge. `validate` checks every Index route, Shape, effect, output, disclosure,
specimen, and manual fallback before `load` can call a host adapter.

Register `CapabilityPack` with `register_citizens` for Lisp read-construct and
Shape access. The crate owns no package-manager, SDK, kernel, or bootloader
behavior.
