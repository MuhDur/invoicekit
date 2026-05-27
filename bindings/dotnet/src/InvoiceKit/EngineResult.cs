// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

using System.Text;

namespace InvoiceKit;

/// <summary>
/// Copied Engine ABI response bytes plus the C ABI status code.
/// </summary>
public sealed class EngineResult : IEquatable<EngineResult>
{
    private readonly byte[] responseBytes;

    /// <summary>
    /// Create an Engine ABI result from copied response bytes.
    /// </summary>
    /// <param name="statusCode">Status code returned by the C ABI or REST sidecar.</param>
    /// <param name="responseBytes">Response bytes copied from native memory or HTTP response content.</param>
    public EngineResult(EngineStatusCode statusCode, byte[] responseBytes)
    {
        StatusCode = statusCode;
        this.responseBytes = (responseBytes ?? throw new ArgumentNullException(nameof(responseBytes))).ToArray();
    }

    /// <summary>
    /// Status code returned by the C ABI or REST sidecar.
    /// </summary>
    public EngineStatusCode StatusCode { get; }

    /// <summary>
    /// True when <see cref="StatusCode"/> is <see cref="EngineStatusCode.Ok"/>.
    /// </summary>
    public bool IsOk => StatusCode == EngineStatusCode.Ok;

    /// <summary>
    /// Return a defensive copy of the Engine ABI response bytes.
    /// </summary>
    /// <returns>Copied response bytes.</returns>
    public byte[] ResponseBytes() => responseBytes.ToArray();

    /// <summary>
    /// Decode response bytes as UTF-8 text.
    /// </summary>
    /// <returns>Response bytes decoded as UTF-8.</returns>
    public string ResponseText() => Encoding.UTF8.GetString(responseBytes);

    /// <inheritdoc />
    public bool Equals(EngineResult? other)
    {
        return other is not null
            && StatusCode == other.StatusCode
            && responseBytes.SequenceEqual(other.responseBytes);
    }

    /// <inheritdoc />
    public override bool Equals(object? obj) => Equals(obj as EngineResult);

    /// <inheritdoc />
    public override int GetHashCode()
    {
        var hash = new HashCode();
        hash.Add(StatusCode);
        foreach (var b in responseBytes)
        {
            hash.Add(b);
        }

        return hash.ToHashCode();
    }

    /// <inheritdoc />
    public override string ToString() => $"EngineResult {{ StatusCode = {StatusCode}, ResponseBytes = {responseBytes.Length} bytes }}";
}
