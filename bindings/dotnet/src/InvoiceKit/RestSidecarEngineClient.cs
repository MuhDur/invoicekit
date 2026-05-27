// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

using System.Net.Http.Headers;
using System.Text;
using System.Text.Json;

namespace InvoiceKit;

/// <summary>
/// REST sidecar fallback for environments where native loading is unavailable.
/// </summary>
public sealed class RestSidecarEngineClient : IEngineClient
{
    /// <summary>
    /// Header used by the sidecar to preserve the C ABI status code.
    /// </summary>
    public const string StatusHeader = "X-InvoiceKit-Status-Code";

    private readonly HttpClient httpClient;
    private readonly Uri processEndpoint;

    /// <summary>
    /// Create a sidecar client for a full Engine ABI process endpoint.
    /// </summary>
    /// <param name="processEndpoint">Full sidecar endpoint that accepts Engine ABI JSON bytes.</param>
    public RestSidecarEngineClient(Uri processEndpoint)
        : this(new HttpClient(), processEndpoint)
    {
    }

    internal RestSidecarEngineClient(HttpClient httpClient, Uri processEndpoint)
    {
        this.httpClient = httpClient ?? throw new ArgumentNullException(nameof(httpClient));
        this.processEndpoint = RequireAbsolute(processEndpoint);
    }

    /// <inheritdoc />
    public uint AbiVersion => EngineClients.EngineAbiVersion;

    /// <inheritdoc />
    public EngineResult Process(byte[] requestBytes)
    {
        ArgumentNullException.ThrowIfNull(requestBytes);

        using var request = new HttpRequestMessage(HttpMethod.Post, processEndpoint)
        {
            Content = new ByteArrayContent(requestBytes),
        };
        request.Content.Headers.ContentType = new MediaTypeHeaderValue("application/json");
        request.Headers.Accept.Add(new MediaTypeWithQualityHeaderValue("application/json"));

        HttpResponseMessage response;
        try
        {
            response = httpClient.Send(request);
        }
        catch (HttpRequestException error)
        {
            throw new InvoiceKitException(
                "sidecar_io_error",
                "InvoiceKit REST sidecar request failed",
                "Start the InvoiceKit REST sidecar and pass its Engine ABI endpoint URI.",
                error);
        }
        catch (TaskCanceledException error)
        {
            throw new InvoiceKitException(
                "sidecar_timeout",
                "InvoiceKit REST sidecar request timed out",
                "Check that the sidecar is reachable and increase the caller timeout if needed.",
                error);
        }

        using (response)
        {
            using var responseStream = response.Content.ReadAsStream();
            using var responseMemory = new MemoryStream();
            responseStream.CopyTo(responseMemory);
            var responseBytes = responseMemory.ToArray();
            if (!response.IsSuccessStatusCode)
            {
                throw new InvoiceKitException(
                    "sidecar_http_error",
                    $"InvoiceKit REST sidecar returned HTTP {(int)response.StatusCode}",
                    "Check the sidecar URL, health endpoint, and server logs.");
            }

            var statusCode = response.Headers.TryGetValues(StatusHeader, out var values)
                ? ParseStatusHeader(values.FirstOrDefault())
                : DeriveStatusCode(responseBytes);
            return new EngineResult(statusCode, responseBytes);
        }
    }

    /// <summary>
    /// Process an Engine ABI JSON request string.
    /// </summary>
    /// <param name="requestJson">Engine ABI JSON request text.</param>
    /// <returns>Copied Engine ABI response bytes plus status code.</returns>
    /// <exception cref="InvoiceKitException">Thrown when the sidecar cannot process the request.</exception>
    public EngineResult Process(string requestJson)
    {
        ArgumentNullException.ThrowIfNull(requestJson);
        return Process(Encoding.UTF8.GetBytes(requestJson));
    }

    /// <summary>
    /// Release the HTTP client owned by this sidecar client.
    /// </summary>
    public void Dispose()
    {
        httpClient.Dispose();
    }

    private static Uri RequireAbsolute(Uri endpoint)
    {
        ArgumentNullException.ThrowIfNull(endpoint);
        if (!endpoint.IsAbsoluteUri)
        {
            throw new ArgumentException("processEndpoint must be an absolute URI.", nameof(endpoint));
        }

        return endpoint;
    }

    private static EngineStatusCode ParseStatusHeader(string? value)
    {
        return uint.TryParse(value, out var parsed)
            ? (EngineStatusCode)parsed
            : EngineStatusCode.Error;
    }

    private static EngineStatusCode DeriveStatusCode(byte[] responseBytes)
    {
        try
        {
            using var document = JsonDocument.Parse(responseBytes);
            if (!document.RootElement.TryGetProperty("status", out var status))
            {
                return EngineStatusCode.Error;
            }

            return status.GetString() == "ok"
                ? EngineStatusCode.Ok
                : EngineStatusCode.Error;
        }
        catch (JsonException)
        {
            return EngineStatusCode.Error;
        }
    }
}
