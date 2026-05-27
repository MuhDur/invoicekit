// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

using System.Text;
using System.Text.Json;
using System.Text.Json.Nodes;
using InvoiceKit;

namespace InvoiceKit.Examples.AspNet;

/// <summary>
/// T-1404 thin wrapper around the InvoiceKit .NET SDK's native
/// engine client. Holds one client for the lifetime of the ASP.NET
/// app. Fails fast at boot if the native library cannot be loaded
/// (avoids the silent fall-through that
/// <see cref="EngineClients.NativeOrSidecar"/> would offer).
/// </summary>
public sealed class InvoiceKitBridge : IDisposable
{
    private readonly NativeEngineClient _client;

    public InvoiceKitBridge()
    {
        try
        {
            _client = EngineClients.NativeClient();
        }
        catch (InvoiceKitException ex)
        {
            throw new InvalidOperationException(
                "invoicekit native client unavailable: " + ex.Message
                    + " (set INVOICEKIT_FFI_LIB to the absolute path of "
                    + "libinvoicekit_ffi.so/.dylib/.dll)",
                ex);
        }
    }

    public JsonObject Canonicalize(IReadOnlyDictionary<string, object> document)
    {
        var request = new JsonObject
        {
            ["abi_version"] = 1,
            ["operation"] = "commercial_document.canonicalize",
            ["payload"] = JsonNode.Parse(JsonSerializer.Serialize(document))!,
        };
        var requestBytes = Encoding.UTF8.GetBytes(request.ToJsonString());
        var result = _client.Process(requestBytes);
        var responseBytes = result.ResponseBytes();
        var parsed = JsonNode.Parse(Encoding.UTF8.GetString(responseBytes))!.AsObject();
        parsed["_engine_status"] = (int)result.StatusCode;
        return parsed;
    }

    public void Dispose() => _client.Dispose();
}
