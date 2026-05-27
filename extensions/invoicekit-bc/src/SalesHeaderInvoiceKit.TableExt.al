// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

tableextension 71500 "InvoiceKit Sales Header" extends "Sales Header"
{
    fields
    {
        field(71500; "InvoiceKit Status"; Enum "InvoiceKit Status")
        {
            Caption = 'InvoiceKit Status';
            DataClassification = CustomerContent;
            InitValue = "Not Sent";
        }
        field(71501; "InvoiceKit Submission ID"; Text[100])
        {
            Caption = 'InvoiceKit Submission ID';
            DataClassification = CustomerContent;
            Editable = false;
        }
        field(71502; "InvoiceKit Evidence URL"; Text[250])
        {
            Caption = 'InvoiceKit Evidence URL';
            DataClassification = CustomerContent;
            Editable = false;
            ExtendedDatatype = URL;
        }
        field(71503; "InvoiceKit Last Error"; Text[250])
        {
            Caption = 'InvoiceKit Last Error';
            DataClassification = CustomerContent;
            Editable = false;
        }
    }
}
