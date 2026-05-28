# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 The InvoiceKit Authors
{
    "name": "InvoiceKit E-Invoicing",
    "version": "17.0.0.1.0",
    "category": "Accounting",
    "summary": "Send Odoo invoices through the InvoiceKit sidecar for"
               " Peppol/AS4 transmission and signed evidence bundles.",
    "description": (
        "Adds a 'Send via InvoiceKit' action to account.move that POSTs"
        " the Odoo invoice JSON to the InvoiceKit sidecar's /v1/transmit"
        " endpoint. The sidecar projects to UBL, validates, signs the"
        " evidence bundle, and submits via the configured partner Peppol"
        " AP. The receipt updates the move's invoicekit_state field and"
        " attaches the evidence bundle as an ir.attachment."
    ),
    "author": "The InvoiceKit Authors",
    "website": "https://github.com/MuhDur/invoicekit",
    "license": "Apache-2.0",
    "depends": ["account", "base_setup"],
    "data": [
        "security/ir.model.access.csv",
        "data/server_actions.xml",
        "views/account_move_views.xml",
        "views/res_config_settings_views.xml",
    ],
    "installable": True,
    "application": False,
    "auto_install": False,
}
