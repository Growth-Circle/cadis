# cadis-output-filter

Output filter pipeline for the C.A.D.I.S. tool runtime.

Strips noise from command output (ANSI codes, duplicate lines, verbose logs)
so agents receive only the signal they need. Inspired by RTK.

Includes semantic-boundary truncation that breaks at headings, code fences,
and function boundaries instead of raw byte limits. The `file.search` path
uses a trigram-based search index for large workspaces. Both features are
inspired by [QMD](https://github.com/tobi/qmd).
