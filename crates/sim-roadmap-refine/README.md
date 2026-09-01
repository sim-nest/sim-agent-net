# sim-roadmap-refine

`sim-roadmap-refine` is the pure admission boundary for replacing one grounded
roadmap leaf with a smaller finite tree. Profiles are derived from admitted
phase facts and exact grounding; proposals cannot submit ranks or counts.

`apply_refinement` either returns exactly one content-addressed successor and a
verifiable descent certificate, or returns a structured refusal without
mutating or minting anything. The crate owns no model, runner, journal,
process, source adapter, or dependency compiler.
