# license-header — Apache 2.0 SPDX header gate for Rust sources

A repository-internal Python script that asserts every Rust source file carries the
canonical Apache 2.0 SPDX header, and can insert it where missing. This is
developer/CI tooling, not part of any shipped artifact.

## What it does

`check_headers.py` walks every `.rs` file under the scan roots `crates/`,
`bindings/`, `services/`, and `bridges/` (relative to the repository root) and
checks that each file begins with the two-line header:

```
// SPDX-License-Identifier: Apache-2.0
// Copyright {year} The InvoiceKit Authors
```

The first line must be exactly the SPDX identifier line; the second must match
`// Copyright <4-digit-year> The InvoiceKit Authors`. Files whose first line
contains the marker `@generated` are skipped.

Two modes:

- `--check` (default): prints each offending path with its reason
  (`missing-spdx`, `missing-copyright`, `read-error`) to stderr and exits `1`
  if any file is missing or malformed; exits `0` when all files pass.
- `--fix`: inserts the canonical header in place. When the SPDX line is absent
  it prepends the full two-line header plus a trailing blank line; when only the
  copyright line is missing it inserts that line after the existing SPDX line.
  The fix is idempotent. `--fix` prints the files it changed and always exits `0`.

`--repo-root` overrides the scanned root (defaults to the script's grandparent
directory). Argument errors exit `2` via `argparse`.

The script only enforces the presence and shape of this header. It does not
verify SPDX semantics against a license database, does not scan non-Rust files,
and does not edit anything in `--check` mode.

## Usage / CI

Run directly:

```
python3 tools/license-header/check_headers.py --check
python3 tools/license-header/check_headers.py --fix
```

Unit tests under `tests/` (run with `pytest tools/license-header/tests -q`)
cover header acceptance, the failure reasons, the `--fix` insertion path,
idempotency, and the `@generated` skip.

In CI the `License header` workflow (`.github/workflows/license-header.yml`,
triggered on push and pull requests to `main`) runs the header-tool tests with
pytest and then runs `python3 tools/license-header/check_headers.py --check` as
a gate. The same workflow also runs unrelated checks from `tools/release-checks`
and `tools/conformance-corpus`.

## License

Apache-2.0.
