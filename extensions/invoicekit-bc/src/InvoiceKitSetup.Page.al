// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

page 71500 "InvoiceKit Setup"
{
    PageType = Card;
    SourceTable = "InvoiceKit Setup";
    Caption = 'InvoiceKit Setup';
    ApplicationArea = All;
    UsageCategory = Administration;
    InsertAllowed = false;
    DeleteAllowed = false;

    layout
    {
        area(Content)
        {
            group(Connection)
            {
                Caption = 'Connection';

                field("Sidecar URL"; Rec."Sidecar URL")
                {
                    ApplicationArea = All;
                    ToolTip = 'Specifies the base URL of the InvoiceKit sidecar that owns validation, transmission, and evidence bundle storage.';
                }
                field("API Key"; Rec."API Key")
                {
                    ApplicationArea = All;
                    ToolTip = 'Specifies the optional bearer token sent to the InvoiceKit sidecar.';
                }
            }
        }
    }

    trigger OnOpenPage()
    begin
        Rec.EnsureExists();
    end;
}
