// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

using System;
using SAPbouiCOM;

namespace InvoiceKit.B1Addon;

public sealed class SapInvoiceSnapshot
{
    public SapInvoiceSnapshot(
        string companyDatabase,
        string documentEntry,
        string documentNumber,
        string businessPartnerCode,
        string currency,
        string total)
    {
        CompanyDatabase = Require(companyDatabase, nameof(companyDatabase));
        DocumentEntry = Require(documentEntry, nameof(documentEntry));
        DocumentNumber = Require(documentNumber, nameof(documentNumber));
        BusinessPartnerCode = Require(businessPartnerCode, nameof(businessPartnerCode));
        Currency = Require(currency, nameof(currency));
        Total = Require(total, nameof(total));
    }

    public string CompanyDatabase { get; }

    public string DocumentEntry { get; }

    public string DocumentNumber { get; }

    public string BusinessPartnerCode { get; }

    public string Currency { get; }

    public string Total { get; }

    public static SapInvoiceSnapshot FromActiveForm(Application application)
    {
        if (application is null)
        {
            throw new ArgumentNullException(nameof(application));
        }

        Form form = application.Forms.ActiveForm;
        if (form.TypeEx != "133")
        {
            throw new InvalidOperationException("Open an A/R invoice before sending it through InvoiceKit.");
        }

        string companyDatabase = Environment.GetEnvironmentVariable("INVOICEKIT_TENANT_ID") ?? "sap-business-one";

        return new SapInvoiceSnapshot(
            companyDatabase,
            ReadItem(form, "8"),
            ReadItem(form, "8"),
            ReadItem(form, "4"),
            ReadItem(form, "63"),
            ReadItem(form, "29"));
    }

    private static string ReadItem(Form form, string itemUid)
    {
        object specific = form.Items.Item(itemUid).Specific;
        return specific switch
        {
            EditText editText => editText.Value,
            ComboBox comboBox => comboBox.Value,
            StaticText staticText => staticText.Caption,
            _ => throw new InvalidOperationException($"Unsupported SAP B1 invoice field item {itemUid}.")
        };
    }

    private static string Require(string value, string name)
    {
        if (string.IsNullOrWhiteSpace(value))
        {
            throw new ArgumentException($"{name} is required.", name);
        }

        return value;
    }
}
