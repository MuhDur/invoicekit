# conformance-corpus/synthetic

Reserved for the synthetic conformance corpus per `plans/PLAN.md` §4.1 /
Track 10. Populated by the corpus beads, not by T-001.

Synthetic fixtures are fictional and public. Each fixture directory must include
`metadata.json` validated by `tools/conformance-corpus/validate_fixture_metadata.py`.
The `examples/` directory contains small T-120 samples that prove the metadata
contract before the larger generated corpus lands.

The `cii-d16b/` directory contains the `invoices-h4b3` synthetic CII corpus.
Regenerate it with:

```bash
cargo run -p invoicekit-format-cii --example generate_cii_corpus
```

The generator creates missing files, accepts identical existing files, and
fails instead of overwriting changed fixture data.
