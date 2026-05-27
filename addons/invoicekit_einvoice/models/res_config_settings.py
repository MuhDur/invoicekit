# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 The InvoiceKit Authors
"""System-level settings for the InvoiceKit sidecar.

Surfaces the sidecar URL + API key under
*Invoicing → Configuration → InvoiceKit*.
"""
from __future__ import annotations

try:
    from odoo import fields, models  # type: ignore[import-not-found]
except ImportError:  # pragma: no cover — non-Odoo dev/test path
    _ODOO_AVAILABLE = False
else:
    _ODOO_AVAILABLE = True


if _ODOO_AVAILABLE:

    class ResConfigSettings(models.TransientModel):  # type: ignore[misc, no-any-unimported]
        _inherit = "res.config.settings"

        invoicekit_sidecar_url = fields.Char(
            string="InvoiceKit Sidecar URL",
            config_parameter="invoicekit.sidecar_url",
            default="http://127.0.0.1:8088",
            help="Base URL of the InvoiceKit sidecar that owns the Peppol "
            "transmission + evidence-bundle layer.",
        )
        invoicekit_api_key = fields.Char(
            string="InvoiceKit API Key",
            config_parameter="invoicekit.api_key",
            help="Optional bearer token sent with sidecar requests. Leave "
            "blank when the sidecar runs on the same host without auth.",
        )
