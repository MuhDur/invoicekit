// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

namespace InvoiceKit;

/// <summary>
/// Configuration for locating the InvoiceKit native C ABI library.
/// </summary>
public sealed class NativeLibraryConfig
{
    /// <summary>
    /// Environment variable used to point at a specific native library path.
    /// </summary>
    public const string LibraryPathEnvironmentVariable = "INVOICEKIT_FFI_LIB";

    private NativeLibraryConfig(string? libraryPath, bool disabled)
    {
        LibraryPath = string.IsNullOrWhiteSpace(libraryPath) ? null : libraryPath;
        Disabled = disabled;
    }

    /// <summary>
    /// Full native library path, or null to use the runtime's library resolver.
    /// </summary>
    public string? LibraryPath { get; }

    /// <summary>
    /// True when native loading should be skipped.
    /// </summary>
    public bool Disabled { get; }

    /// <summary>
    /// Create a configuration from <see cref="LibraryPathEnvironmentVariable"/>.
    /// </summary>
    /// <returns>Native library configuration.</returns>
    public static NativeLibraryConfig FromEnvironment()
    {
        return FromLibraryPath(Environment.GetEnvironmentVariable(LibraryPathEnvironmentVariable));
    }

    /// <summary>
    /// Create a configuration that uses an explicit native library path.
    /// </summary>
    /// <param name="libraryPath">Full native library path. Blank values fall back to runtime resolution.</param>
    /// <returns>Native library configuration.</returns>
    public static NativeLibraryConfig FromLibraryPath(string? libraryPath) => new(libraryPath, disabled: false);

    /// <summary>
    /// Create a configuration that intentionally disables native loading.
    /// </summary>
    /// <returns>Disabled native library configuration.</returns>
    public static NativeLibraryConfig DisabledForTests() => new(null, disabled: true);
}
