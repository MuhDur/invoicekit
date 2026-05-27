# InvoiceKit Python SDK

Python package for the InvoiceKit Engine ABI.

The package is built with PyO3 and maturin. It exposes the same canonical JSON
byte contract documented in `crates/invoicekit-ffi/ABI.md`; structured invoice
model helpers are added by later SDK beads.

```python
import invoicekit

request = b'{"abi_version":1,"operation":"unknown","payload":{}}'
result = invoicekit.engine_process_json(request)

assert result.status == 1
assert b'"status":"error"' in result.bytes
result.free()
```

Build a local wheel:

```bash
uvx maturin build --manifest-path bindings/python/Cargo.toml
```
