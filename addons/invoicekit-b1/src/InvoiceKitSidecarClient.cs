// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

using System;
using System.IO;
using System.Net;
using System.Text;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace InvoiceKit.B1Addon;

public sealed class InvoiceKitSidecarClient : IDisposable
{
    private static readonly JsonSerializerOptions JsonOptions = new()
    {
        PropertyNamingPolicy = JsonNamingPolicy.SnakeCaseLower,
        DefaultIgnoreCondition = JsonIgnoreCondition.WhenWritingNull
    };

    private readonly InvoiceKitSettings settings;

    public InvoiceKitSidecarClient(InvoiceKitSettings settings)
    {
        this.settings = settings ?? throw new ArgumentNullException(nameof(settings));
    }

    public static InvoiceKitSidecarClient FromEnvironment()
    {
        InvoiceKitSettings settings = InvoiceKitSettings.FromEnvironment();
        return new InvoiceKitSidecarClient(settings);
    }

    public InvoiceKitReceipt Transmit(SapInvoiceSnapshot invoice)
    {
        if (invoice is null)
        {
            throw new ArgumentNullException(nameof(invoice));
        }

        object payload = new
        {
            tenant_id = invoice.CompanyDatabase,
            trace_id = Guid.NewGuid().ToString("N"),
            idempotency_key = $"sap-b1:{invoice.CompanyDatabase}:{invoice.DocumentEntry}",
            gateway_attempt_id = Guid.NewGuid().ToString("N"),
            source_system = "sap-business-one",
            document = invoice
        };

        string json = JsonSerializer.Serialize(payload, JsonOptions);
        byte[] requestBody = Encoding.UTF8.GetBytes(json);

        HttpWebRequest request = (HttpWebRequest)WebRequest.Create(new Uri(settings.SidecarBaseUrl, "/v1/transmit"));
        request.Method = "POST";
        request.Accept = "application/json";
        request.ContentType = "application/json";
        request.ContentLength = requestBody.Length;
        request.Timeout = 30000;
        request.ReadWriteTimeout = 30000;

        if (!string.IsNullOrWhiteSpace(settings.ApiKey))
        {
            request.Headers[HttpRequestHeader.Authorization] = "Bearer " + settings.ApiKey;
        }

        using (Stream requestStream = request.GetRequestStream())
        {
            requestStream.Write(requestBody, 0, requestBody.Length);
        }

        string responseBody;
        try
        {
            using HttpWebResponse response = (HttpWebResponse)request.GetResponse();
            responseBody = ReadResponseBody(response);
        }
        catch (WebException ex) when (ex.Response is HttpWebResponse)
        {
            using HttpWebResponse response = (HttpWebResponse)ex.Response;
            throw new InvalidOperationException($"InvoiceKit sidecar refused the SAP B1 invoice: HTTP {(int)response.StatusCode}.");
        }

        InvoiceKitReceipt? receipt = JsonSerializer.Deserialize<InvoiceKitReceipt>(responseBody, JsonOptions);
        if (receipt is null)
        {
            throw new InvalidOperationException("InvoiceKit sidecar returned an invalid receipt.");
        }

        if (string.IsNullOrWhiteSpace(receipt.SubmissionId) || string.IsNullOrWhiteSpace(receipt.State))
        {
            throw new InvalidOperationException("InvoiceKit sidecar returned an invalid receipt.");
        }

        return receipt;
    }

    public void Dispose()
    {
    }

    private static string ReadResponseBody(HttpWebResponse response)
    {
        using Stream responseStream = response.GetResponseStream();
        using StreamReader reader = new(responseStream, Encoding.UTF8);
        return reader.ReadToEnd();
    }
}

public sealed class InvoiceKitReceipt
{
    public string SubmissionId { get; init; } = string.Empty;

    public string State { get; init; } = string.Empty;

    public string? EvidenceBundleUrl { get; init; }
}
