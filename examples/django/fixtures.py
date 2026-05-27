# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 The InvoiceKit Authors
#
# T-1401 demo fixtures: three German XRechnung-shaped CommercialDocuments.
# Identical shape to the FastAPI demo so the cross-framework gates
# exercise the same canonicalize output.

from __future__ import annotations

from typing import Any


_SELLER: dict[str, Any] = {
    "name": "Acme GmbH",
    "tax_ids": [{"scheme": "vat", "value": "DE123456789"}],
    "address": {
        "lines": ["Hauptstraße 42"],
        "city": "Berlin",
        "postal_code": "10115",
        "country": "DE",
    },
}

_BUYER: dict[str, Any] = {
    "name": "Beispielkunde AG",
    "tax_ids": [{"scheme": "vat", "value": "DE987654321"}],
    "address": {
        "lines": ["Friedrichstraße 10"],
        "city": "München",
        "postal_code": "80331",
        "country": "DE",
    },
}


def _meta(name: str) -> dict[str, Any]:
    return {"tenant_id": "tenant-demo-django", "trace_id": f"trace-django-{name}"}


FIXTURES: dict[str, dict[str, Any]] = {
    "basic": {
        "schema_version": "1.0",
        "id": "doc-de-django-basic-2026-0001",
        "document_type": "invoice",
        "issue_date": "2026-05-27",
        "due_date": "2026-06-26",
        "document_number": "RE-DJ-2026-0001",
        "currency": "EUR",
        "supplier": _SELLER,
        "customer": _BUYER,
        "payment_instructions": [],
        "lines": [
            {
                "id": "L1",
                "description": "Software-Lizenz Q3/2026",
                "quantity": "1",
                "unit_price": "1000.00",
                "line_extension_amount": "1000.00",
                "tax_category": "S",
                "extensions": [],
            }
        ],
        "tax_summary": [
            {
                "category_code": "S",
                "taxable_amount": "1000.00",
                "tax_amount": "190.00",
                "tax_rate": "19.00",
            }
        ],
        "monetary_total": {
            "line_extension_amount": "1000.00",
            "tax_exclusive_amount": "1000.00",
            "tax_inclusive_amount": "1190.00",
            "payable_amount": "1190.00",
        },
        "extensions": [],
        "meta": _meta("basic"),
    },
    "with-allowance": {
        "schema_version": "1.0",
        "id": "doc-de-django-allowance-2026-0002",
        "document_type": "invoice",
        "issue_date": "2026-05-27",
        "due_date": "2026-06-26",
        "document_number": "RE-DJ-2026-0002",
        "currency": "EUR",
        "supplier": _SELLER,
        "customer": _BUYER,
        "payment_instructions": [],
        "lines": [
            {
                "id": "L1",
                "description": "Beratungsleistung März 2026",
                "quantity": "10",
                "unit_price": "150.00",
                "line_extension_amount": "1500.00",
                "tax_category": "S",
                "extensions": [],
            },
            {
                "id": "L2",
                "description": "Mengenrabatt 10%",
                "quantity": "-1",
                "unit_price": "150.00",
                "line_extension_amount": "-150.00",
                "tax_category": "S",
                "extensions": [],
            },
        ],
        "tax_summary": [
            {
                "category_code": "S",
                "taxable_amount": "1350.00",
                "tax_amount": "256.50",
                "tax_rate": "19.00",
            }
        ],
        "monetary_total": {
            "line_extension_amount": "1350.00",
            "tax_exclusive_amount": "1350.00",
            "tax_inclusive_amount": "1606.50",
            "payable_amount": "1606.50",
        },
        "extensions": [],
        "meta": _meta("with-allowance"),
    },
    "reverse-charge": {
        "schema_version": "1.0",
        "id": "doc-de-django-rc-2026-0003",
        "document_type": "invoice",
        "issue_date": "2026-05-27",
        "due_date": "2026-06-26",
        "document_number": "RE-DJ-2026-0003",
        "currency": "EUR",
        "supplier": _SELLER,
        "customer": {
            **_BUYER,
            "tax_ids": [{"scheme": "vat", "value": "ATU12345678"}],
            "address": {
                "lines": ["Stephansplatz 1"],
                "city": "Wien",
                "postal_code": "1010",
                "country": "AT",
            },
        },
        "payment_instructions": [],
        "lines": [
            {
                "id": "L1",
                "description": "Wartungsvertrag Q3/2026",
                "quantity": "1",
                "unit_price": "5000.00",
                "line_extension_amount": "5000.00",
                "tax_category": "AE",
                "extensions": [],
            }
        ],
        "tax_summary": [
            {
                "category_code": "AE",
                "taxable_amount": "5000.00",
                "tax_amount": "0.00",
                "tax_rate": "0.00",
            }
        ],
        "monetary_total": {
            "line_extension_amount": "5000.00",
            "tax_exclusive_amount": "5000.00",
            "tax_inclusive_amount": "5000.00",
            "payable_amount": "5000.00",
        },
        "extensions": [],
        "meta": _meta("reverse-charge"),
    },
}
