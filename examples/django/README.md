# examples/django

Reference Django demo for InvoiceKit. Canonicalises three German XRechnung
fixtures through the InvoiceKit Rust engine via the same ctypes bridge the
FastAPI demo uses — no maturin or pyo3 install required.

## 5-minute setup

```bash
# 1. Clone the repo
git clone https://github.com/MuhDur/invoicekit.git
cd invoicekit

# 2. Build the InvoiceKit cdylib (~30s)
cargo build -p invoicekit-ffi --locked

# 3. Install + run the demo
cd examples/django
python3 -m venv .venv
.venv/bin/pip install -r requirements.txt
INVOICEKIT_FFI_LIB=$(realpath ../../target/debug/libinvoicekit_ffi.so) \
  .venv/bin/python manage.py runserver
# → http://127.0.0.1:8000
```

## Endpoints

| Method | Path                                              | Purpose                                            |
|--------|---------------------------------------------------|----------------------------------------------------|
| GET    | `/`                                               | List available fixtures.                           |
| GET    | `/healthz`                                        | Liveness probe.                                    |
| POST   | `/canonicalize/{basic\|with-allowance\|reverse-charge}` | Canonicalise the named XRechnung fixture.   |

## Smoke test

```bash
INVOICEKIT_FFI_LIB=$(realpath ../../target/debug/libinvoicekit_ffi.so) \
  .venv/bin/pytest tests/test_smoke.py -v
```

Uses `django.test.Client` so the gate stays fast.

## Files

| Path                           | Purpose                                            |
|--------------------------------|----------------------------------------------------|
| `demo/settings.py`             | Minimal Django settings (in-memory SQLite).        |
| `demo/urls.py`                 | URL conf for the three endpoints.                  |
| `demo/views.py`                | View functions backed by the bridge.               |
| `fixtures.py`                  | Three German XRechnung CommercialDocument dicts.   |
| `invoicekit_bridge.py`         | ctypes wrapper around `libinvoicekit_ffi`.         |
| `manage.py`                    | Standard Django management entrypoint.             |
| `tests/test_smoke.py`          | 6 smoke assertions covering every endpoint + 404.  |

## Architecture

Identical bridge to the FastAPI demo — only the web framework differs. The
bridge loads `libinvoicekit_ffi.so/.dylib/.dll` via `ctypes.CDLL` so the
demo runs without needing the pyo3-built `invoicekit` Python package on
PyPI.

## License

Apache-2.0.

Implemented by `invoices-t-1401-demo-django-5i5s`.
