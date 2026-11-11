# Fixtures of the document readers

1. **`liteparse/`:** copied verbatim from graal's `test/data/liteparse-corpus` (`cicd/hello.html`, `cicd/images/sample.svg`) with the snapshots graal's `liteparse_corpus_test.clj` holds them to (`snapshot/`). Source: llamaindex/liteparse_cicd_data at `e4cdc87c`, MIT (graal's corpus README).
2. **`supervisor-sandbox.html`:** a copy of `private/docs/supervisor-sandbox.html`, since the CI mirror carries `rust/` alone.
3. **`with-media/`:** a page holding an inline SVG diagram and a PNG beside it, for `dash doc import`.
