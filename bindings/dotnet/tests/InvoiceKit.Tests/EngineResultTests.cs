// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

using System.Text;
using Xunit;

namespace InvoiceKit.Tests;

public sealed class EngineResultTests
{
    [Fact]
    public void EngineResultCopiesBytesDefensively()
    {
        var bytes = Encoding.UTF8.GetBytes("{\"status\":\"ok\"}");
        var result = new EngineResult(EngineStatusCode.Ok, bytes);

        bytes[0] = (byte)'!';
        var returned = result.ResponseBytes();
        returned[1] = (byte)'!';

        Assert.Equal("{\"status\":\"ok\"}", result.ResponseText());
        Assert.True(result.IsOk);
    }

    [Fact]
    public void EngineResultEqualityIncludesStatusAndBytes()
    {
        var left = new EngineResult(EngineStatusCode.Error, Encoding.UTF8.GetBytes("error"));
        var right = new EngineResult(EngineStatusCode.Error, Encoding.UTF8.GetBytes("error"));
        var different = new EngineResult(EngineStatusCode.Ok, Encoding.UTF8.GetBytes("error"));

        Assert.Equal(left, right);
        Assert.NotEqual(left, different);
    }
}
