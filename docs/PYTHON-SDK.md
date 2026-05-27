# Python SDK

The `invoicekit` Python package wraps the Engine ABI byte contract. It is built
with PyO3 and maturin and exposes the same six entry points documented in
`crates/invoicekit-ffi/ABI.md`.

From a clean clone, build a local wheel and run a smoke test:

```bash
CACHE_ROOT="${XDG_CACHE_HOME:-$HOME/.cache}/invoicekit-python"
mkdir -p "$CACHE_ROOT"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$CACHE_ROOT/cargo-target}"

OUT_DIR="$(mktemp -d "$CACHE_ROOT/wheels.XXXXXX")"
PYTHON_DIR="$(mktemp -d "$CACHE_ROOT/package.XXXXXX")"

uvx maturin build --manifest-path bindings/python/Cargo.toml --out "$OUT_DIR"
python3 -m pip install --no-deps --target "$PYTHON_DIR" "$OUT_DIR"/invoicekit-*.whl
PYTHONPATH="$PYTHON_DIR" python3 - <<'PY'
import invoicekit

request = b'{"abi_version":1,"operation":"unknown","payload":{}}'
result = invoicekit.engine_process_json(request)

assert invoicekit.engine_abi_version() == 1
assert invoicekit.engine_result_status(result) == 1
assert b'"status":"error"' in invoicekit.engine_result_bytes(result)
invoicekit.engine_result_free(result)
PY
```

The release wheel uses `abi3-py310`, so one binary supports CPython 3.10 and
newer on the target platform. Publishing to PyPI and the full
Linux/macOS/Windows wheel matrix are release gates.
