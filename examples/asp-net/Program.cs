// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors
//
// T-1404 reference ASP.NET Core minimal-API demo. Canonicalises
// three German XRechnung fixtures through the InvoiceKit Rust
// engine using the InvoiceKit .NET SDK's native client.

using System.Text;
using System.Text.Json;
using InvoiceKit;
using InvoiceKit.Examples.AspNet;

var builder = WebApplication.CreateBuilder(args);
builder.Services.AddSingleton<InvoiceKitBridge>();

var app = builder.Build();

app.MapGet("/", () => Results.Json(new
{
    title = "InvoiceKit ASP.NET demo",
    fixtures = Fixtures.Names,
    usage = "POST /canonicalize/{fixtureName}",
}));

app.MapGet("/healthz", () => Results.Json(new { status = "ok" }));

app.MapPost("/canonicalize/{fixtureName}", (string fixtureName, InvoiceKitBridge bridge) =>
{
    var document = Fixtures.Get(fixtureName);
    if (document is null)
    {
        return Results.Json(new
        {
            error = new
            {
                code = "UNKNOWN_FIXTURE",
                available = Fixtures.Names,
            },
        }, statusCode: 404);
    }
    var canonicalized = bridge.Canonicalize(document);
    return Results.Json(canonicalized);
});

app.Run();

/// <summary>Top-level Program type for WebApplicationFactory in tests.</summary>
public partial class Program { }
