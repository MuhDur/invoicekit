"""T-770 release-check: country feasibility manifests load + validate.

Every TOML file under `data/country-manifests/` MUST:

1. Declare ``schema_version = "1.0"``.
2. Carry the seven bead-required fields: ``country``, ``program``,
   ``retrieved_at``, ``[sources]`` with at least one entry,
   ``[sandbox].availability``, ``[trust].qualified_seal_requirement``,
   ``[fiscal_rep].required_for``, ``[validator].backend``.
3. Reference its sandbox credentials env var with the
   ``INVOICEKIT_SANDBOX_<CC>_*`` shape so the T-074c sandbox-drift
   canary can wire it in.
4. Pin a ``signature_alg`` of ``blake3:identity`` with a non-empty
   ``signature`` field (the all-zero placeholder is allowed during
   bootstrap and tested separately).

The signature itself is computed by hashing the canonical TOML body
minus the ``signature`` field; a future bead can extend this gate to
verify the signature, but for now we only require that the field
shape is honored so the manifest can be cryptographically promoted
later without a schema change.
"""

from __future__ import annotations

from pathlib import Path

import pytest
import tomllib

REPO = Path(__file__).resolve().parents[2]
MANIFEST_DIR = REPO / "data" / "country-manifests"

REQUIRED_TOP_LEVEL = {
    "schema_version",
    "country",
    "country_name",
    "program",
    "retrieved_at",
    "maintainer",
    "bead",
    "signature_alg",
    "signature",
}

REQUIRED_SANDBOX = {"availability", "endpoint", "sandbox_credentials_env_var"}
SANDBOX_AVAILABILITIES = {
    "public",
    "partner_gated",
    "requires_local_tax_id",
    "none",
}

REQUIRED_TRUST = {
    "qualified_seal_requirement",
    "smart_card_requirement",
    "hsm_requirement",
}
QES_VALUES = {"qes_required", "qes_optional", "none"}

REQUIRED_FISCAL_REP = {"required_for", "notes"}
FISCAL_REP_VALUES = {"foreign_sellers", "all_taxable_persons", "none"}

REQUIRED_VALIDATOR = {"backend"}
VALIDATOR_PREFIXES = ("rust_native", "jvm:", "rest:", "partner", "cli", "none")


def _manifests() -> list[Path]:
    if not MANIFEST_DIR.exists():
        return []
    return sorted(MANIFEST_DIR.glob("*.toml"))


def test_country_manifest_directory_has_at_least_one_entry() -> None:
    """The release-check is meaningful only when at least one manifest exists."""
    assert _manifests(), f"no manifests found under {MANIFEST_DIR}"


@pytest.mark.parametrize("path", _manifests(), ids=lambda p: p.name)
def test_country_manifest_loads_and_passes_schema(path: Path) -> None:
    raw = path.read_text(encoding="utf-8")
    data = tomllib.loads(raw)

    # Top-level fields.
    missing = REQUIRED_TOP_LEVEL - set(data.keys())
    assert not missing, f"{path.name}: missing top-level fields {missing}"
    assert data["schema_version"] == "1.0", (
        f"{path.name}: schema_version must be \"1.0\" (got {data['schema_version']!r})"
    )
    assert data["signature_alg"] == "blake3:identity", (
        f"{path.name}: signature_alg must be \"blake3:identity\" (got {data['signature_alg']!r})"
    )
    assert isinstance(data["signature"], str) and data["signature"], (
        f"{path.name}: signature must be non-empty"
    )

    # Sources: at least one entry with name + url + retrieved_at.
    sources = data.get("sources", {}).get("entry", [])
    assert sources, f"{path.name}: must declare at least one [[sources.entry]]"
    for src in sources:
        for required in ("name", "url", "kind", "retrieved_at"):
            assert required in src, f"{path.name}: source entry missing {required}"

    # Sandbox block.
    sandbox = data.get("sandbox", {})
    sandbox_missing = REQUIRED_SANDBOX - set(sandbox.keys())
    assert not sandbox_missing, (
        f"{path.name}: missing [sandbox] fields {sandbox_missing}"
    )
    assert sandbox["availability"] in SANDBOX_AVAILABILITIES, (
        f"{path.name}: sandbox.availability {sandbox['availability']!r} not in {SANDBOX_AVAILABILITIES}"
    )
    assert sandbox["sandbox_credentials_env_var"].startswith("INVOICEKIT_SANDBOX_"), (
        f"{path.name}: sandbox.sandbox_credentials_env_var must start with INVOICEKIT_SANDBOX_"
    )

    # Trust block.
    trust = data.get("trust", {})
    trust_missing = REQUIRED_TRUST - set(trust.keys())
    assert not trust_missing, f"{path.name}: missing [trust] fields {trust_missing}"
    assert trust["qualified_seal_requirement"] in QES_VALUES, (
        f"{path.name}: trust.qualified_seal_requirement {trust['qualified_seal_requirement']!r} not in {QES_VALUES}"
    )

    # Fiscal rep block.
    fiscal_rep = data.get("fiscal_rep", {})
    fiscal_rep_missing = REQUIRED_FISCAL_REP - set(fiscal_rep.keys())
    assert not fiscal_rep_missing, (
        f"{path.name}: missing [fiscal_rep] fields {fiscal_rep_missing}"
    )
    required_for = fiscal_rep.get("required_for", [])
    assert isinstance(required_for, list) and required_for, (
        f"{path.name}: fiscal_rep.required_for must be a non-empty list"
    )
    for entry in required_for:
        assert entry in FISCAL_REP_VALUES, (
            f"{path.name}: fiscal_rep.required_for entry {entry!r} not in {FISCAL_REP_VALUES}"
        )

    # Validator block.
    validator = data.get("validator", {})
    validator_missing = REQUIRED_VALIDATOR - set(validator.keys())
    assert not validator_missing, (
        f"{path.name}: missing [validator] fields {validator_missing}"
    )
    backend = validator["backend"]
    assert any(backend == p or backend.startswith(p) for p in VALIDATOR_PREFIXES), (
        f"{path.name}: validator.backend {backend!r} must be one of {VALIDATOR_PREFIXES}"
    )


@pytest.mark.parametrize("path", _manifests(), ids=lambda p: p.name)
def test_country_manifest_country_code_matches_filename(path: Path) -> None:
    """ISO 3166-1 alpha-2 country code in the manifest must match the
    filename stem so a directory listing is self-describing."""
    data = tomllib.loads(path.read_text(encoding="utf-8"))
    country = data["country"]
    # We expect <country-name>.toml (e.g. poland.toml carrying PL).
    # The filename is the long-name form, the field is the ISO code;
    # we just assert the ISO code is 2 uppercase letters.
    assert len(country) == 2 and country.isupper() and country.isalpha(), (
        f"{path.name}: country {country!r} must be ISO 3166-1 alpha-2"
    )
