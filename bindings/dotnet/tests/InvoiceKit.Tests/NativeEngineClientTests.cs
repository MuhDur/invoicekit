// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

using System.Text.Json;
using System.Text.Json.Serialization;
using Xunit;

namespace InvoiceKit.Tests;

public sealed class NativeEngineClientTests
{
    [Fact]
    public void DisabledNativeConfigThrowsTypedException()
    {
        var error = Assert.Throws<InvoiceKitException>(
            () => EngineClients.NativeClient(NativeLibraryConfig.DisabledForTests()));

        Assert.Equal("native_disabled", error.Code);
    }

    [Fact]
    public void NativeClientMatchesGoldenFixtureWhenLibraryConfigured()
    {
        var nativeLibrary = Environment.GetEnvironmentVariable(NativeLibraryConfig.LibraryPathEnvironmentVariable);
        if (string.IsNullOrWhiteSpace(nativeLibrary))
        {
            return;
        }

        var fixture = LoadGoldenFixture();
        using var client = EngineClients.NativeClient(NativeLibraryConfig.FromLibraryPath(nativeLibrary));

        var result = client.Process(fixture.RequestBytes);

        Assert.Equal(EngineClients.EngineAbiVersion, client.AbiVersion);
        Assert.Equal(EngineStatusCode.Ok, result.StatusCode);
        Assert.Equal(fixture.ExpectedResponseBytes, result.ResponseText());
    }

    private static GoldenFixture LoadGoldenFixture()
    {
        var root = FindRepoRoot();
        var fixturePath = Path.Combine(root, "conformance-corpus", "golden", "engine-abi-v1-commercial-document.json");
        return JsonSerializer.Deserialize<GoldenFixture>(
            File.ReadAllText(fixturePath),
            new JsonSerializerOptions { PropertyNameCaseInsensitive = true })
            ?? throw new InvalidOperationException("golden fixture was empty");
    }

    private static string FindRepoRoot()
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

    private sealed record GoldenFixture(
        [property: JsonPropertyName("request_bytes")] string RequestBytes,
        [property: JsonPropertyName("expected_response_bytes")] string ExpectedResponseBytes);
}
