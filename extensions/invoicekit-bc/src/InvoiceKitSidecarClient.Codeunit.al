// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

codeunit 71500 "InvoiceKit Sidecar Client"
{
    procedure SendSalesInvoice(var SalesHeader: Record "Sales Header")
    var
        Setup: Record "InvoiceKit Setup";
        HttpClient: HttpClient;
        Content: HttpContent;
        ContentHeaders: HttpHeaders;
        Response: HttpResponseMessage;
        Payload: Text;
        ReceiptJson: JsonObject;
        SubmissionId: Text;
        State: Text;
        EvidenceUrl: Text;
        ErrorText: Text;
    begin
        SalesHeader.TestField("Document Type", SalesHeader."Document Type"::Invoice);
        SalesHeader.TestField("No.");

        Setup.EnsureExists();
        Setup.Get('SETUP');
        Setup.TestField("Sidecar URL");

        Payload := BuildPayload(SalesHeader);
        Content.WriteFrom(Payload);
        Content.GetHeaders(ContentHeaders);
        if ContentHeaders.Contains('Content-Type') then
            ContentHeaders.Remove('Content-Type');
        ContentHeaders.Add('Content-Type', 'application/json');

        if Setup."API Key" <> '' then
            HttpClient.DefaultRequestHeaders().Add('Authorization', 'Bearer ' + Setup."API Key");

        if not HttpClient.Post(NormalizeBaseUrl(Setup."Sidecar URL") + '/v1/transmit', Content, Response) then begin
            RecordFailure(SalesHeader, 'InvoiceKit sidecar was unreachable.');
            exit;
        end;

        if not Response.IsSuccessStatusCode() then begin
            Response.Content().ReadAs(ErrorText);
            RecordFailure(SalesHeader, CopyStr(ErrorText, 1, MaxStrLen(SalesHeader."InvoiceKit Last Error")));
            exit;
        end;

        Response.Content().ReadAs(Payload);
        if not ReceiptJson.ReadFrom(Payload) then begin
            RecordFailure(SalesHeader, 'InvoiceKit sidecar returned invalid JSON.');
            exit;
        end;

        if not TryReadRequiredText(ReceiptJson, 'submission_id', SubmissionId) then begin
            RecordFailure(SalesHeader, 'InvoiceKit sidecar response is missing submission_id.');
            exit;
        end;

        if not TryReadRequiredText(ReceiptJson, 'state', State) then begin
            RecordFailure(SalesHeader, 'InvoiceKit sidecar response is missing state.');
            exit;
        end;

        EvidenceUrl := ReadOptionalText(ReceiptJson, 'evidence_bundle_url');

        SalesHeader.Validate("InvoiceKit Status", MapState(State));
        SalesHeader.Validate("InvoiceKit Submission ID", CopyStr(SubmissionId, 1, MaxStrLen(SalesHeader."InvoiceKit Submission ID")));
        SalesHeader.Validate("InvoiceKit Evidence URL", CopyStr(EvidenceUrl, 1, MaxStrLen(SalesHeader."InvoiceKit Evidence URL")));
        SalesHeader.Validate("InvoiceKit Last Error", '');
        SalesHeader.Modify(true);
    end;

    procedure BuildPayload(SalesHeader: Record "Sales Header"): Text
    var
        SalesLine: Record "Sales Line";
        Root: JsonObject;
        Document: JsonObject;
        Lines: JsonArray;
        Line: JsonObject;
        Payload: Text;
    begin
        Root.Add('tenant_id', CompanyName());
        Root.Add('trace_id', Format(CreateGuid(), 0, 4));

        Document.Add('document_number', SalesHeader."No.");
        Document.Add('issue_date', Format(SalesHeader."Document Date", 0, 9));
        Document.Add('currency', SalesHeader."Currency Code");
        Document.Add('sell_to_customer_no', SalesHeader."Sell-to Customer No.");
        Document.Add('bill_to_customer_no', SalesHeader."Bill-to Customer No.");
        Document.Add('amount_including_vat', Format(SalesHeader."Amount Including VAT", 0, 9));

        SalesLine.SetRange("Document Type", SalesHeader."Document Type");
        SalesLine.SetRange("Document No.", SalesHeader."No.");
        if SalesLine.FindSet() then
            repeat
                Clear(Line);
                Line.Add('line_no', SalesLine."Line No.");
                Line.Add('type', Format(SalesLine.Type));
                Line.Add('description', SalesLine.Description);
                Line.Add('quantity', Format(SalesLine.Quantity, 0, 9));
                Line.Add('unit_price', Format(SalesLine."Unit Price", 0, 9));
                Line.Add('line_amount', Format(SalesLine."Line Amount", 0, 9));
                Lines.Add(Line);
            until SalesLine.Next() = 0;

        Document.Add('lines', Lines);
        Root.Add('document', Document);
        Root.WriteTo(Payload);
        exit(Payload);
    end;

    local procedure TryReadRequiredText(Json: JsonObject; Name: Text; var Value: Text): Boolean
    var
        Token: JsonToken;
    begin
        if not Json.Get(Name, Token) then
            exit(false);

        if Token.AsValue().IsNull() then
            exit(false);

        Value := Token.AsValue().AsText();
        exit(Value <> '');
    end;

    local procedure ReadOptionalText(Json: JsonObject; Name: Text): Text
    var
        Token: JsonToken;
    begin
        if not Json.Get(Name, Token) then
            exit('');

        if Token.AsValue().IsNull() then
            exit('');

        exit(Token.AsValue().AsText());
    end;

    local procedure MapState(State: Text): Enum "InvoiceKit Status"
    begin
        case LowerCase(State) of
            'accepted', 'transmitted':
                exit("InvoiceKit Status"::Transmitted);
            'rejected':
                exit("InvoiceKit Status"::Rejected);
            'queued':
                exit("InvoiceKit Status"::Queued);
        end;

        exit("InvoiceKit Status"::Queued);
    end;

    local procedure NormalizeBaseUrl(BaseUrl: Text): Text
    begin
        while CopyStr(BaseUrl, StrLen(BaseUrl), 1) = '/' do
            BaseUrl := CopyStr(BaseUrl, 1, StrLen(BaseUrl) - 1);

        exit(BaseUrl);
    end;

    local procedure RecordFailure(var SalesHeader: Record "Sales Header"; Message: Text)
    begin
        SalesHeader.Validate("InvoiceKit Status", SalesHeader."InvoiceKit Status"::Rejected);
        SalesHeader.Validate("InvoiceKit Last Error", CopyStr(Message, 1, MaxStrLen(SalesHeader."InvoiceKit Last Error")));
        SalesHeader.Modify(true);
    end;
}
