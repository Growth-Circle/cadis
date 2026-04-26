# cadis-protocol

Typed protocol contract for CADIS clients and `cadisd`.

This crate owns protocol versioning, request and event envelopes, request/event
type names, content routing kinds, risk classes, and approval payload shapes.
It must remain independent from daemon internals, UI frameworks, and provider
implementations.

