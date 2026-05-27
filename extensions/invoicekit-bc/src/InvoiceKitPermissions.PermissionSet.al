// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

permissionset 71500 "INVOICEKIT CONNECTOR"
{
    Assignable = true;
    Caption = 'InvoiceKit Connector';

    Permissions =
        tabledata "InvoiceKit Setup" = RIMD,
        table "InvoiceKit Setup" = X,
        page "InvoiceKit Setup" = X,
        codeunit "InvoiceKit Sidecar Client" = X;
}
