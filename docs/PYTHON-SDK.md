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

The release wheel uses `abi3-py310`, so one binary supports CPython
3.10 and newer on the target platform.

## Cross-platform wheel matrix

`.github/workflows/python-sdk.yml` builds the SDK on every PR and
every tag across three platforms and four CPython versions:

| Platform        | Target triple                 | Interpreters tested |
| --------------- | ----------------------------- | ------------------- |
| Linux x86_64    | `x86_64-unknown-linux-gnu`    | 3.10, 3.11, 3.12, 3.13 |
| macOS aarch64   | `aarch64-apple-darwin`        | 3.11, 3.12, 3.13 (3.10 not preinstalled on the runner image) |
| Windows x86_64  | `x86_64-pc-windows-msvc`      | 3.11, 3.12, 3.13 |

The abi3 contract means one wheel per platform covers every listed
interpreter; the matrix re-installs and runs `bindings/python/tests`
against each Python version on each platform to prove the contract
holds.

Platform constraints worth knowing:

- macOS runners do not ship CPython 3.10 by default. If a downstream
  consumer needs both a Linux 3.10 and a macOS 3.10 wheel, add an
  extra matrix entry that calls `actions/setup-python@v5` with
  `python-version: "3.10"`.
- The Windows runner builds with MSVC (`x86_64-pc-windows-msvc`),
  not GNU. Downstream packagers who need a `windows-x86_64-gnu`
  wheel should open a follow-up bead; today's audience runs official
  CPython builds, which link against MSVC.
- The Linux wheel is built on `ubuntu-latest` and is therefore
  `manylinux_2_28`-compatible by maturin's default behaviour.
  CentOS 7 / `manylinux2014` is intentionally out of scope.

## Publishing to PyPI

The `publish` job in `.github/workflows/python-sdk.yml` runs on every
`v*` tag. It downloads every per-platform wheel + the sdist from
the matrix, then uploads them via PyPI trusted publishing (preferred):

```
publish:
  if: startsWith(github.ref, 'refs/tags/v')
  environment:
    name: pypi
    url: https://pypi.org/p/invoicekit
```

### One-time operator setup

1. **Reserve the `invoicekit` project name on PyPI.** Sign in as the
   package owner and create the project. This claim is required
   before trusted publishing can be wired.
2. **Configure trusted publishing.** Under the project's *Publishing*
   tab on PyPI, add a trusted publisher for this repo with:
   - PyPI project name: `invoicekit`
   - Owner: this repo's GitHub owner
   - Repository name: `invoicekit`
   - Workflow filename: `python-sdk.yml`
   - Environment name: `pypi`
3. **Create the `pypi` environment in this repo.** Settings →
   Environments → New environment → `pypi`. Optionally require an
   approval reviewer so the publish step waits for a human.

If trusted publishing cannot be configured for operational reasons,
fall back to a long-lived `PYPI_API_TOKEN` secret and pass it via
the `password:` input on `pypa/gh-action-pypi-publish@release/v1`.
Trusted publishing is preferred because it rotates per-job and
never lives in the repo.

### Per-release operator checklist

1. Bump the workspace version and re-run `cargo publish --dry-run`
   on the dependent crates.
2. Cut a `v<MAJOR>.<MINOR>.<PATCH>` tag.
3. Watch the `python-sdk` workflow run; the `publish` job logs the
   PyPI URL on success. The version-gate step refuses to publish if
   the tag and wheel filename disagree.
4. Record the PyPI version URL in the GitHub release notes
   (`https://pypi.org/project/invoicekit/<VERSION>/`).

### Why one workflow per language

The workflow lives in `python-sdk.yml` (not `release.yml`) so the
Python publish step can be re-run independently when a wheel build
needs a fresh artifact without rolling the rest of the release.
Java's Maven Central publish (`java-sdk.yml`) and .NET's NuGet
publish (`dotnet-sdk.yml`) follow the same per-language split.
