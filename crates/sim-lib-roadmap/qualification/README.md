# Roadmap qualification corpus

This directory is the public, hermetic input corpus for roadmap qualification.
Every file is synthetic and MPL-2.0 repository material; no private roadmap
text is copied here. Tests consume these bytes directly with `include_str!`, so
the corpus is also usable by a native executor without Python plan types.

`source/` exercises exact public and private declarations, moved anchors,
truncation, collisions, changed files, and checked specimens. `v3/` exercises
the legacy v3 adapter's ordinary, delegated-source, review, resource, merged
closeout, malformed, completed, oversized, and recursive native cases.
