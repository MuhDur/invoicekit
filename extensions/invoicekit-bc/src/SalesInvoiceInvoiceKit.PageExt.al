// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

pageextension 71500 "InvoiceKit Sales Invoice" extends "Sales Invoice"
{
    layout
    {
        addlast(General)
        {
            field("InvoiceKit Status"; Rec."InvoiceKit Status")
            {
                ApplicationArea = All;
                ToolTip = 'Shows the most recent InvoiceKit sidecar transmission status.';
            }
            field("InvoiceKit Submission ID"; Rec."InvoiceKit Submission ID")
            {
                ApplicationArea = All;
                ToolTip = 'Shows the submission id returned by the InvoiceKit sidecar.';
            }
            field("InvoiceKit Evidence URL"; Rec."InvoiceKit Evidence URL")
            {
                ApplicationArea = All;
                ToolTip = 'Shows the evidence bundle receipt URL returned by the InvoiceKit sidecar.';
            }
            field("InvoiceKit Last Error"; Rec."InvoiceKit Last Error")
            {
                ApplicationArea = All;
                ToolTip = 'Shows the most recent InvoiceKit sidecar error.';
            }
        }
    }

    actions
    {
        addlast(Processing)
        {
            action(SendViaInvoiceKit)
            {
                Caption = 'Send via InvoiceKit';
                Image = SendTo;
                ApplicationArea = All;
                Enabled = Rec."InvoiceKit Status" <> Rec."InvoiceKit Status"::Transmitted;
                ToolTip = 'Sends this sales invoice to the configured InvoiceKit sidecar.';

                trigger OnAction()
                var
                    SidecarClient: Codeunit "InvoiceKit Sidecar Client";
                begin
                    SidecarClient.SendSalesInvoice(Rec);
                    CurrPage.Update(false);
                end;
            }
        }
    }
}
