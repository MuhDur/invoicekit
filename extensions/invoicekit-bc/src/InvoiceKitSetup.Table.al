// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

table 71500 "InvoiceKit Setup"
{
    Caption = 'InvoiceKit Setup';
    DataClassification = CustomerContent;

    fields
    {
        field(1; "Primary Key"; Code[10])
        {
            Caption = 'Primary Key';
            DataClassification = SystemMetadata;
        }
        field(10; "Sidecar URL"; Text[250])
        {
            Caption = 'Sidecar URL';
            DataClassification = CustomerContent;
            InitValue = 'http://127.0.0.1:8088';
        }
        field(20; "API Key"; Text[250])
        {
            Caption = 'API Key';
            DataClassification = EndUserPseudonymousIdentifiers;
            ExtendedDatatype = Masked;
        }
    }

    keys
    {
        key(PK; "Primary Key")
        {
            Clustered = true;
        }
    }

    procedure EnsureExists()
    begin
        if Get('SETUP') then
            exit;

        Init();
        "Primary Key" := 'SETUP';
        "Sidecar URL" := 'http://127.0.0.1:8088';
        Insert(true);
    end;
}
