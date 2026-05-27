// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

codeunit 71510 "InvoiceKit Sidecar Tests"
{
    Subtype = Test;

    [Test]
    procedure SetupRecordIsCreated()
    var
        Setup: Record "InvoiceKit Setup";
    begin
        Setup.EnsureExists();
        Require(Setup.Get('SETUP'), 'InvoiceKit setup record should exist.');
        Require(Setup."Sidecar URL" = 'http://127.0.0.1:8088', 'Default sidecar URL should be local loopback.');
    end;

    [Test]
    procedure PayloadIncludesInvoiceIdentity()
    var
        SalesHeader: Record "Sales Header";
        SidecarClient: Codeunit "InvoiceKit Sidecar Client";
        Payload: Text;
    begin
        SalesHeader.Init();
        SalesHeader."Document Type" := SalesHeader."Document Type"::Invoice;
        SalesHeader."No." := 'INV-BC-001';
        SalesHeader."Sell-to Customer No." := 'C-1000';
        SalesHeader."Bill-to Customer No." := 'C-1000';
        Payload := SidecarClient.BuildPayload(SalesHeader);

        Require(StrPos(Payload, 'INV-BC-001') > 0, 'Payload should include the sales invoice number.');
        Require(StrPos(Payload, 'tenant_id') > 0, 'Payload should include the tenant id field.');
        Require(StrPos(Payload, 'trace_id') > 0, 'Payload should include a trace id.');
    end;

    local procedure Require(Condition: Boolean; FailureMessage: Text)
    begin
        if not Condition then
            Error(FailureMessage);
    end;
}
