# cadis-core

Core CADIS request handling, session registry, event creation, and model routing.

This crate does not own sockets or terminal UI. `cadisd` owns transport and calls
into this runtime.
