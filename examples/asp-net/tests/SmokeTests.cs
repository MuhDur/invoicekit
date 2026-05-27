// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

using System.Net;
using System.Net.Http.Json;
using System.Text.Json;
using Microsoft.AspNetCore.Mvc.Testing;
using Xunit;

namespace InvoiceKit.Examples.AspNet.Tests;

/// <summary>
/// T-1404 smoke suite. Uses WebApplicationFactory so the app boots
/// in-process and we exercise the same handlers the server would
/// without spinning up Kestrel.
/// </summary>
public sealed class SmokeTests : IClassFixture<WebApplicationFactory<Program>>
{
    private readonly WebApplicationFactory<Program> _factory;

    public SmokeTests(WebApplicationFactory<Program> factory)
    {
        _factory = factory;
    }

    [Fact]
    public async Task RootListsFixtures()
    {
        var client = _factory.CreateClient();
        var response = await client.GetAsync("/");
        Assert.Equal(HttpStatusCode.OK, response.StatusCode);
        var body = await response.Content.ReadFromJsonAsync<JsonElement>();
        Assert.Equal("InvoiceKit ASP.NET demo", body.GetProperty("title").GetString());
        Assert.Equal(3, body.GetProperty("fixtures").GetArrayLength());
    }

    [Fact]
    public async Task Healthz()
    {
        var client = _factory.CreateClient();
        var response = await client.GetAsync("/healthz");
        Assert.Equal(HttpStatusCode.OK, response.StatusCode);
        var body = await response.Content.ReadFromJsonAsync<JsonElement>();
        Assert.Equal("ok", body.GetProperty("status").GetString());
    }

    [Theory]
    [InlineData("basic")]
    [InlineData("with-allowance")]
    [InlineData("reverse-charge")]
    public async Task CanonicalizeFixture(string fixtureName)
    {
        var client = _factory.CreateClient();
        var response = await client.PostAsync($"/canonicalize/{fixtureName}", new StringContent(string.Empty));
        Assert.Equal(HttpStatusCode.OK, response.StatusCode);
        var body = await response.Content.ReadFromJsonAsync<JsonElement>();
        Assert.Equal(0, body.GetProperty("_engine_status").GetInt32());
    }

    [Fact]
    public async Task UnknownFixtureReturns404()
    {
        var client = _factory.CreateClient();
        var response = await client.PostAsync("/canonicalize/does-not-exist", new StringContent(string.Empty));
        Assert.Equal(HttpStatusCode.NotFound, response.StatusCode);
        var body = await response.Content.ReadFromJsonAsync<JsonElement>();
        Assert.Equal("UNKNOWN_FIXTURE", body.GetProperty("error").GetProperty("code").GetString());
    }
}
