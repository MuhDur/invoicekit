// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

using System.Text.Json;
using System.Text.Json.Serialization;
using InvoiceKit;

var root = FindRepoRoot();
var fixturePath = Path.Combine(root, "conformance-corpus", "golden", "engine-abi-v1-commercial-document.json");
var fixture = JsonSerializer.Deserialize<GoldenFixture>(
    File.ReadAllText(fixturePath),
    new JsonSerializerOptions { PropertyNameCaseInsensitive = true })
    ?? throw new InvalidOperationException("golden fixture was empty");

using var client = EngineClients.NativeClient();
if (client.AbiVersion != EngineClients.EngineAbiVersion)
{
    throw new InvalidOperationException(
        $"expected ABI version {EngineClients.EngineAbiVersion}, got {client.AbiVersion}");
}

var result = client.Process(fixture.RequestBytes);
if (result.StatusCode != EngineStatusCode.Ok)
{
    throw new InvalidOperationException($"expected status {EngineStatusCode.Ok}, got {result.StatusCode}");
}

if (result.ResponseText() != fixture.ExpectedResponseBytes)
{
    throw new InvalidOperationException(".NET SDK ABI response did not match golden bytes");
}

static string FindRepoRoot()
{
    var current = new DirectoryInfo(Environment.CurrentDirectory);
    while (current is not null)
    {
        if (File.Exists(Path.Combine(current.FullName, "Cargo.toml"))
            && Directory.Exists(Path.Combine(current.FullName, "conformance-corpus")))
        {
            return current.FullName;
        }

        current = current.Parent;
    }

    throw new InvalidOperationException("could not locate repository root");
}

internal sealed record GoldenFixture(
    [property: JsonPropertyName("request_bytes")] string RequestBytes,
    [property: JsonPropertyName("expected_response_bytes")] string ExpectedResponseBytes);
