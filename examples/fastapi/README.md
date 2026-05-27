# examples/fastapi

Reference FastAPI demo for InvoiceKit. Canonicalises three German XRechnung
fixtures through the InvoiceKit Rust engine via a ~80-line ctypes bridge —
no maturin or pyo3 install required.

## 5-minute setup

```bash
# 1. Clone the repo (no submodules)
git clone https://github.com/MuhDur/invoicekit.git
cd invoicekit

# 2. Build the InvoiceKit cdylib (~30s)
cargo build -p invoicekit-ffi --locked

# 3. Install + run the demo
cd examples/fastapi
python3 -m venv .venv
.venv/bin/pip install -r requirements.txt
INVOICEKIT_FFI_LIB=$(realpath ../../target/debug/libinvoicekit_ffi.so) \
  .venv/bin/uvicorn app:app --reload
# → http://127.0.0.1:8000
```

## Endpoints

| Method | Path                                  | Purpose                                                |
|--------|---------------------------------------|--------------------------------------------------------|
| GET    | `/`                                   | List available fixtures.                               |
| GET    | `/healthz`                            | Liveness probe.                                        |
| POST   | `/canonicalize/{basic\|with-allowance\|reverse-charge}` | Canonicalize the named XRechnung fixture through the Rust engine. |

## Smoke test

```bash
INVOICEKIT_FFI_LIB=$(realpath ../../target/debug/libinvoicekit_ffi.so) \
  .venv/bin/pytest tests/test_smoke.py -v
```

Bypasses uvicorn via `fastapi.testclient.TestClient` so the gate stays fast.

## Files

| Path                           | Purpose                                                   |
|--------------------------------|-----------------------------------------------------------|
| `app.py`                       | FastAPI app with `/canonicalize/{fixture}` endpoint.      |
| `fixtures.py`                  | Three German XRechnung CommercialDocument dicts.          |
| `invoicekit_bridge.py`         | ctypes wrapper around `libinvoicekit_ffi`.                |
| `tests/test_smoke.py`          | 6 smoke assertions covering every endpoint + 404 path.    |
| `requirements.txt`             | fastapi + httpx + pytest + uvicorn pins.                  |

## Architecture

The demo deliberately avoids `pip install invoicekit` (which would require
maturin + pyo3 setup). Instead the ~80-line `invoicekit_bridge.py` loads
the Rust cdylib via `ctypes.CDLL` and calls the Engine ABI directly. This
is the same approach `bindings/python/tests/test_engine_abi.py` takes for
the cross-language ABI golden test.

When the pyo3-built `invoicekit` Python package ships to PyPI, consumers
can drop the bridge and `from invoicekit import process_engine_abi_json`
without restructuring the rest of the demo.

## License

Apache-2.0.

Implemented by `invoices-t-1406-demo-fastapi-kqit`.
