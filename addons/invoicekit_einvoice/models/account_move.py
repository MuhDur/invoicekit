# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 The InvoiceKit Authors
"""Extends `account.move` with InvoiceKit transmission fields.

Imported by Odoo at addon load time; under non-Odoo test runs
(pytest from the sidecar-client tests) `odoo` isn't importable
and the module gracefully no-ops so the sidecar client stays
unit-testable.
"""
from __future__ import annotations

try:
    from odoo import _, api, fields, models  # type: ignore[import-not-found]
    from odoo.exceptions import UserError  # type: ignore[import-not-found]
except ImportError:  # pragma: no cover — non-Odoo dev/test path
    _ODOO_AVAILABLE = False
else:
    _ODOO_AVAILABLE = True


if _ODOO_AVAILABLE:
    from .sidecar_client import InvoiceKitSidecar, InvoiceKitSidecarError

    INVOICEKIT_STATE = [
        ("not_sent", "Not sent"),
        ("queued", "Queued"),
        ("transmitted", "Transmitted"),
        ("rejected", "Rejected"),
    ]

    class AccountMove(models.Model):  # type: ignore[misc, no-any-unimported]
        _inherit = "account.move"

        invoicekit_state = fields.Selection(
            INVOICEKIT_STATE,
            string="InvoiceKit Status",
            default="not_sent",
            tracking=True,
            copy=False,
            help="Last known transmission state reported by the InvoiceKit sidecar.",
        )
        invoicekit_submission_id = fields.Char(
            string="InvoiceKit Submission ID",
            copy=False,
            readonly=True,
        )
        invoicekit_evidence_bundle_url = fields.Char(
            string="Evidence Bundle URL",
            copy=False,
            readonly=True,
        )

        def action_send_via_invoicekit(self) -> dict[str, str]:
            """Server action: POST the move to the sidecar's /v1/transmit."""
            self.ensure_one()
            if self.state != "posted":  # Odoo's own posted check
                raise UserError(_("Post the invoice before sending via InvoiceKit."))
            settings = self.env["res.config.settings"].sudo()
            base_url = settings.get_param(
                "invoicekit.sidecar_url", default="http://127.0.0.1:8088"
            )
            api_key = settings.get_param("invoicekit.api_key", default=False) or None
            payload = self._invoicekit_serialize()
            client = InvoiceKitSidecar(base_url=base_url, api_key=api_key)
            try:
                receipt = client.transmit(payload)
            except InvoiceKitSidecarError as exc:
                self.invoicekit_state = "rejected"
                raise UserError(_("InvoiceKit transmission failed: %s") % exc) from exc
            self.write(
                {
                    "invoicekit_state": _map_state(receipt.state),
                    "invoicekit_submission_id": receipt.submission_id,
                    "invoicekit_evidence_bundle_url": receipt.evidence_bundle_url or False,
                }
            )
            return {
                "type": "ir.actions.client",
                "tag": "display_notification",
                "params": {
                    "type": "success",
                    "title": _("InvoiceKit"),
                    "message": _("Sent (submission id %s)") % receipt.submission_id,
                    "sticky": False,
                },
            }

        def _invoicekit_serialize(self) -> dict[str, object]:
            """Build the JSON payload the sidecar's /v1/transmit accepts."""
            self.ensure_one()
            return {
                "tenant_id": str(self.company_id.id),
                "trace_id": f"odoo-{self.id}",
                "document": {
                    "document_number": self.name,
                    "issue_date": str(self.invoice_date or fields.Date.today()),
                    "currency": self.currency_id.name,
                    "supplier": _party_dict(self.company_id.partner_id),
                    "customer": _party_dict(self.partner_id),
                    "lines": [_line_dict(line) for line in self.invoice_line_ids],
                    "totals": {
                        "tax_exclusive": str(self.amount_untaxed),
                        "tax_inclusive": str(self.amount_total),
                        "payable": str(self.amount_residual),
                    },
                },
            }

    def _party_dict(partner: object) -> dict[str, object]:
        return {
            "name": getattr(partner, "name", ""),
            "vat": getattr(partner, "vat", "") or "",
            "country": (getattr(partner, "country_id", None) or _Null()).code or "",
            "street": getattr(partner, "street", "") or "",
            "city": getattr(partner, "city", "") or "",
            "zip": getattr(partner, "zip", "") or "",
        }

    def _line_dict(line: object) -> dict[str, object]:
        return {
            "description": getattr(line, "name", ""),
            "quantity": str(getattr(line, "quantity", 0)),
            "unit_price": str(getattr(line, "price_unit", 0)),
            "line_total": str(getattr(line, "price_subtotal", 0)),
        }

    def _map_state(state: str) -> str:
        return {
            "accepted": "transmitted",
            "queued": "queued",
            "rejected": "rejected",
        }.get(state, "queued")

    class _Null:
        code = ""
