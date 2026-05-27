// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

enum 71500 "InvoiceKit Status"
{
    Extensible = false;

    value(0; "Not Sent")
    {
        Caption = 'Not sent';
    }
    value(1; Queued)
    {
        Caption = 'Queued';
    }
    value(2; Transmitted)
    {
        Caption = 'Transmitted';
    }
    value(3; Rejected)
    {
        Caption = 'Rejected';
    }
}
