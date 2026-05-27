// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

using System;

namespace InvoiceKit.B1Addon;

public sealed class InvoiceKitSettings
{
    public InvoiceKitSettings(Uri sidecarBaseUrl, string? apiKey)
    {
        SidecarBaseUrl = sidecarBaseUrl ?? throw new ArgumentNullException(nameof(sidecarBaseUrl));
        ApiKey = apiKey;
    }

    public Uri SidecarBaseUrl { get; }

    public string? ApiKey { get; }

    public static InvoiceKitSettings FromEnvironment()
    {
        string rawUrl = Environment.GetEnvironmentVariable("INVOICEKIT_SIDECAR_URL") ?? "http://127.0.0.1:8088";
        string? apiKey = Environment.GetEnvironmentVariable("INVOICEKIT_API_KEY");

        return new InvoiceKitSettings(new Uri(rawUrl, UriKind.Absolute), apiKey);
    }
}
