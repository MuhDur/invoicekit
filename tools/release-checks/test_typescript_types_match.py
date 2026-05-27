"""T-012 release-check: schemas and generated TypeScript stay in lockstep.

This is a coarse meta-check that runs alongside the SPDX/IR/capabilities
gates. The real assertion lives in `.github/workflows/typescript-types.yml`
(which actually runs `node scripts/generate.mjs --check` and `tsc --noEmit`
in CI); this Python gate is a fast pre-flight that catches the most
likely drift mode: a new schema was committed without re-running the
generator.
"""

from __future__ import annotations

from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
SCHEMA_DIR = REPO / "schemas"
TS_PKG = REPO / "bindings" / "typescript-types"
TS_GEN = TS_PKG / "src" / "generated"
TS_INDEX = TS_PKG / "src" / "index.ts"


def _schema_module_name(schema_path: Path) -> str:
    name = schema_path.name
    name = name[: -len(".schema.json")] if name.endswith(".schema.json") else name[: -len(".json")]
    # Mirror the slugifier in scripts/generate.mjs.
    out: list[str] = []
    prev_underscore = False
    for ch in name:
        if ch.isalnum():
            out.append(ch)
            prev_underscore = False
        elif not prev_underscore:
            out.append("_")
            prev_underscore = True
    return "".join(out).strip("_")


def test_typescript_types_package_exists() -> None:
    """The @invoicekit/types package must exist as a binding."""
    assert (TS_PKG / "package.json").exists(), f"missing {TS_PKG/'package.json'}"
    assert TS_GEN.exists(), f"missing {TS_GEN}"
    assert TS_INDEX.exists(), f"missing {TS_INDEX}"


def test_every_schema_has_a_generated_dts() -> None:
    schemas = sorted(SCHEMA_DIR.glob("*.json"))
    assert schemas, "no schemas committed under schemas/"
    missing: list[str] = []
    for schema in schemas:
        mod = _schema_module_name(schema)
        dts = TS_GEN / f"{mod}.d.ts"
        if not dts.exists():
            missing.append(
                f"{schema.name} → expected {dts.relative_to(REPO)} (run `bun --cwd bindings/typescript-types run generate`)"
            )
    assert not missing, "\n".join(missing)


def test_every_generated_dts_is_reexported_by_index() -> None:
    index = TS_INDEX.read_text(encoding="utf8")
    missing: list[str] = []
    for dts in sorted(TS_GEN.glob("*.d.ts")):
        mod = dts.name.removesuffix(".d.ts")
        if f"./generated/{mod}.js" not in index:
            missing.append(
                f"{dts.relative_to(REPO)} not re-exported by {TS_INDEX.relative_to(REPO)}"
            )
    assert not missing, "\n".join(missing)


def test_no_orphaned_generated_dts() -> None:
    schemas = {_schema_module_name(p) for p in SCHEMA_DIR.glob("*.json")}
    orphans = [
        dts.relative_to(REPO).as_posix()
        for dts in sorted(TS_GEN.glob("*.d.ts"))
        if dts.name.removesuffix(".d.ts") not in schemas
    ]
    assert not orphans, (
        "extra .d.ts files in src/generated/ have no schema (delete them):\n"
        + "\n".join(orphans)
    )
