// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

namespace InvoiceKit.Examples.AspNet;

/// <summary>
/// T-1404 demo fixtures: three German XRechnung shapes that share
/// the same canonical schema as the FastAPI / Django / Next.js /
/// Spring Boot / Go chi demos.
/// </summary>
public static class Fixtures
{
    private static readonly Dictionary<string, Dictionary<string, object>> _all = BuildAll();

    public static IReadOnlyList<string> Names => _all.Keys.OrderBy(n => n).ToList();

    public static Dictionary<string, object>? Get(string name) =>
        _all.TryGetValue(name, out var doc) ? doc : null;

    private static Dictionary<string, Dictionary<string, object>> BuildAll() =>
        new()
        {
            ["basic"] = Basic(),
            ["with-allowance"] = WithAllowance(),
            ["reverse-charge"] = ReverseCharge(),
        };

    private static Dictionary<string, object> Seller() => new()
    {
        ["name"] = "Acme GmbH",
        ["tax_ids"] = new[]
        {
            new Dictionary<string, string> { ["scheme"] = "vat", ["value"] = "DE123456789" },
        },
        ["address"] = new Dictionary<string, object>
        {
            ["lines"] = new[] { "Hauptstraße 42" },
            ["city"] = "Berlin",
            ["postal_code"] = "10115",
            ["country"] = "DE",
        },
    };

    private static Dictionary<string, object> Buyer() => new()
    {
        ["name"] = "Beispielkunde AG",
        ["tax_ids"] = new[]
        {
            new Dictionary<string, string> { ["scheme"] = "vat", ["value"] = "DE987654321" },
        },
        ["address"] = new Dictionary<string, object>
        {
            ["lines"] = new[] { "Friedrichstraße 10" },
            ["city"] = "München",
            ["postal_code"] = "80331",
            ["country"] = "DE",
        },
    };

    private static Dictionary<string, object> Base(string id, string number, string traceSuffix) => new()
    {
        ["schema_version"] = "1.0",
        ["id"] = id,
        ["document_type"] = "invoice",
        ["issue_date"] = "2026-05-27",
        ["due_date"] = "2026-06-26",
        ["document_number"] = number,
        ["currency"] = "EUR",
        ["supplier"] = Seller(),
        ["customer"] = Buyer(),
        ["payment_instructions"] = Array.Empty<object>(),
        ["extensions"] = Array.Empty<object>(),
        ["meta"] = new Dictionary<string, string>
        {
            ["tenant_id"] = "tenant-demo-asp-net",
            ["trace_id"] = $"trace-asp-net-{traceSuffix}",
        },
    };

    private static Dictionary<string, object> Basic()
    {
        var doc = Base("doc-de-aspnet-basic-2026-0001", "RE-AN-2026-0001", "basic");
        doc["lines"] = new[]
        {
            Line("L1", "Software-Lizenz Q3/2026", "1", "1000.00", "1000.00", "S"),
        };
        doc["tax_summary"] = new[]
        {
            TaxSummary("S", "1000.00", "190.00", "19.00"),
        };
        doc["monetary_total"] = MonetaryTotal("1000.00", "1000.00", "1190.00", "1190.00");
        return doc;
    }

    private static Dictionary<string, object> WithAllowance()
    {
        var doc = Base("doc-de-aspnet-allowance-2026-0002", "RE-AN-2026-0002", "with-allowance");
        doc["lines"] = new[]
        {
            Line("L1", "Beratungsleistung März 2026", "10", "150.00", "1500.00", "S"),
            Line("L2", "Mengenrabatt 10%", "-1", "150.00", "-150.00", "S"),
        };
        doc["tax_summary"] = new[]
        {
            TaxSummary("S", "1350.00", "256.50", "19.00"),
        };
        doc["monetary_total"] = MonetaryTotal("1350.00", "1350.00", "1606.50", "1606.50");
        return doc;
    }

    private static Dictionary<string, object> ReverseCharge()
    {
        var doc = Base("doc-de-aspnet-rc-2026-0003", "RE-AN-2026-0003", "reverse-charge");
        doc["customer"] = new Dictionary<string, object>
        {
            ["name"] = "Beispielkunde AG",
            ["tax_ids"] = new[]
            {
                new Dictionary<string, string> { ["scheme"] = "vat", ["value"] = "ATU12345678" },
            },
            ["address"] = new Dictionary<string, object>
            {
                ["lines"] = new[] { "Stephansplatz 1" },
                ["city"] = "Wien",
                ["postal_code"] = "1010",
                ["country"] = "AT",
            },
        };
        doc["lines"] = new[]
        {
            Line("L1", "Wartungsvertrag Q3/2026", "1", "5000.00", "5000.00", "AE"),
        };
        doc["tax_summary"] = new[]
        {
            TaxSummary("AE", "5000.00", "0.00", "0.00"),
        };
        doc["monetary_total"] = MonetaryTotal("5000.00", "5000.00", "5000.00", "5000.00");
        return doc;
    }

    private static Dictionary<string, object> Line(
        string id, string description, string quantity,
        string unitPrice, string lineExtensionAmount, string taxCategory) => new()
    {
        ["id"] = id,
        ["description"] = description,
        ["quantity"] = quantity,
        ["unit_price"] = unitPrice,
        ["line_extension_amount"] = lineExtensionAmount,
        ["tax_category"] = taxCategory,
        ["extensions"] = Array.Empty<object>(),
    };

    private static Dictionary<string, string> TaxSummary(
        string categoryCode, string taxableAmount, string taxAmount, string taxRate) => new()
    {
        ["category_code"] = categoryCode,
        ["taxable_amount"] = taxableAmount,
        ["tax_amount"] = taxAmount,
        ["tax_rate"] = taxRate,
    };

    private static Dictionary<string, string> MonetaryTotal(
        string lineExtension, string taxExclusive, string taxInclusive, string payable) => new()
    {
        ["line_extension_amount"] = lineExtension,
        ["tax_exclusive_amount"] = taxExclusive,
        ["tax_inclusive_amount"] = taxInclusive,
        ["payable_amount"] = payable,
    };
}
