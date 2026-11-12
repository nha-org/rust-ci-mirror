# Fixtures of the document readers

1. **`liteparse/`:** copied verbatim from graal's `test/data/liteparse-corpus` (`cicd/hello.html`, `cicd/images/sample.svg`) with the snapshots graal's `liteparse_corpus_test.clj` holds them to (`snapshot/`). Source: llamaindex/liteparse_cicd_data at `e4cdc87c`, MIT (graal's corpus README).
2. **`supervisor-sandbox.html`:** a copy of `private/docs/supervisor-sandbox.html`, since the CI mirror carries `rust/` alone.
   **`rfc-inbox.html`:** a copy of `private/docs/rfc-inbox.html`, an RFC with each part `dash doc import` keeps: sections, writer notes, a decision table, diagrams (task editor-in-rfcs).
3. **`with-media/`:** a page holding an inline SVG diagram and a PNG beside it, for `dash doc import`.
4. **Binary files are never here:** a PDF, image or model is listed in the crate's `fixtures.txt` and fetched through `dash_meter::fixtures`' sha256-checked cache (gate rule `no_binary_fixture_is_tracked_under_crates`). `with-media/page.html`'s PNG is liteparse's `hello.png`, copied beside it as `dot.png` by the test.
