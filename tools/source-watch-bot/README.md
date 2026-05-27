# InvoiceKit Source Watch

`source_watch.py` monitors authoritative e-invoicing source URLs, keeps a small
state cache, and opens a structured follow-up when a source changes.

The committed registry at `data/sources/official.toml` is signed with
`sha256:identity`: a deterministic digest over the canonical registry payload.
This matches the seed-manifest pattern used elsewhere in the repo. It catches
accidental tampering of the initial source list, but it is not a replacement for
future Sigstore or minisign-backed source registry signing.

Common commands:

```bash
python3 tools/source-watch-bot/source_watch.py verify
python3 tools/source-watch-bot/source_watch.py sign-registry
python3 tools/source-watch-bot/source_watch.py run --issue-backend dry-run
```

The scheduled workflow runs daily. On its first run it establishes a baseline in
the workflow cache; later runs compare against that state and open a GitHub
issue when a source hash changes.
