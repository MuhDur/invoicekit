// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

using System.Text;

namespace InvoiceKit;

/// <summary>
/// Client for the InvoiceKit Engine ABI JSON byte contract.
/// </summary>
public interface IEngineClient : IDisposable
{
    /// <summary>
    /// Engine ABI version implemented by the backing runtime.
    /// </summary>
    uint AbiVersion { get; }

    /// <summary>
    /// Process Engine ABI JSON request bytes.
    /// </summary>
    /// <param name="requestBytes">UTF-8 Engine ABI JSON request bytes.</param>
    /// <returns>Copied Engine ABI response bytes plus status code.</returns>
    /// <exception cref="InvoiceKitException">Thrown when the backing runtime cannot process the request.</exception>
    EngineResult Process(byte[] requestBytes);

    /// <summary>
    /// Process an Engine ABI JSON request string.
    /// </summary>
    /// <param name="requestJson">Engine ABI JSON request text.</param>
    /// <returns>Copied Engine ABI response bytes plus status code.</returns>
    /// <exception cref="InvoiceKitException">Thrown when the backing runtime cannot process the request.</exception>
    EngineResult Process(string requestJson)
    {
        ArgumentNullException.ThrowIfNull(requestJson);
        return Process(Encoding.UTF8.GetBytes(requestJson));
    }
}
