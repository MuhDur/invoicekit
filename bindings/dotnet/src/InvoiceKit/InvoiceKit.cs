// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

namespace InvoiceKit;

/// <summary>
/// Factory methods for InvoiceKit .NET SDK clients.
/// </summary>
public static class EngineClients
{
    /// <summary>
    /// Engine ABI version implemented by this SDK generation.
    /// </summary>
    public const uint EngineAbiVersion = 1;

    /// <summary>
    /// Create a native Engine ABI client using <see cref="NativeLibraryConfig.FromEnvironment"/>.
    /// </summary>
    /// <returns>Native Engine ABI client.</returns>
    /// <exception cref="InvoiceKitException">Thrown when the native runtime is unavailable.</exception>
    public static NativeEngineClient NativeClient() => NativeClient(NativeLibraryConfig.FromEnvironment());

    /// <summary>
    /// Create a native Engine ABI client with an explicit native library configuration.
    /// </summary>
    /// <param name="config">Native library loading configuration.</param>
    /// <returns>Native Engine ABI client.</returns>
    /// <exception cref="InvoiceKitException">Thrown when the native runtime is unavailable.</exception>
    public static NativeEngineClient NativeClient(NativeLibraryConfig config) => new(config);

    /// <summary>
    /// Create a REST sidecar client for a full Engine ABI process endpoint URI.
    /// </summary>
    /// <param name="processEndpoint">Full sidecar endpoint that accepts Engine ABI JSON bytes.</param>
    /// <returns>REST sidecar client.</returns>
    public static RestSidecarEngineClient RestSidecar(Uri processEndpoint) => new(processEndpoint);

    /// <summary>
    /// Prefer the native client and fall back to a REST sidecar when native loading fails.
    /// </summary>
    /// <param name="processEndpoint">Full sidecar endpoint that accepts Engine ABI JSON bytes.</param>
    /// <returns>Native client when available, otherwise a REST sidecar client.</returns>
    public static IEngineClient NativeOrSidecar(Uri processEndpoint)
    {
        return NativeOrSidecar(processEndpoint, NativeLibraryConfig.FromEnvironment());
    }

    /// <summary>
    /// Prefer the native client with an explicit configuration and fall back to a REST sidecar.
    /// </summary>
    /// <param name="processEndpoint">Full sidecar endpoint that accepts Engine ABI JSON bytes.</param>
    /// <param name="config">Native library loading configuration.</param>
    /// <returns>Native client when available, otherwise a REST sidecar client.</returns>
    public static IEngineClient NativeOrSidecar(Uri processEndpoint, NativeLibraryConfig config)
    {
        ArgumentNullException.ThrowIfNull(processEndpoint);
        ArgumentNullException.ThrowIfNull(config);

        try
        {
            return NativeClient(config);
        }
        catch (InvoiceKitException)
        {
            return RestSidecar(processEndpoint);
        }
    }

    /// <summary>
    /// Process an Engine ABI JSON request and return UTF-8 response text.
    /// </summary>
    /// <param name="client">Selected Engine ABI client.</param>
    /// <param name="requestJson">Engine ABI JSON request text.</param>
    /// <returns>Response bytes decoded as UTF-8 text.</returns>
    /// <exception cref="InvoiceKitException">Thrown when the selected client cannot process the request.</exception>
    public static string ProcessEngineAbiJson(IEngineClient client, string requestJson)
    {
        ArgumentNullException.ThrowIfNull(client);
        return client.Process(requestJson).ResponseText();
    }
}
