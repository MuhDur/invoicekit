// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

using System.Runtime.InteropServices;
using System.Text;

namespace InvoiceKit;

/// <summary>
/// Native Engine ABI client over the <c>invoicekit-ffi</c> C ABI.
/// </summary>
public sealed unsafe class NativeEngineClient : IEngineClient, IDisposable
{
    private const string LibraryName = "invoicekit_ffi";

    private readonly IntPtr libraryHandle;
    private readonly AbiVersionDelegate abiVersion;
    private readonly ProcessJsonDelegate processJson;
    private readonly ResultStatusDelegate resultStatus;
    private readonly ResultBytesDelegate resultBytes;
    private readonly ResultLenDelegate resultLen;
    private readonly ResultFreeDelegate resultFree;
    private bool disposed;

    /// <summary>
    /// Create a native client using <see cref="NativeLibraryConfig.FromEnvironment"/>.
    /// </summary>
    /// <exception cref="InvoiceKitException">Thrown when the native library cannot be loaded.</exception>
    public NativeEngineClient()
        : this(NativeLibraryConfig.FromEnvironment())
    {
    }

    /// <summary>
    /// Create a native client with an explicit native library configuration.
    /// </summary>
    /// <param name="config">Native library loading configuration.</param>
    /// <exception cref="InvoiceKitException">Thrown when the native library cannot be loaded.</exception>
    public NativeEngineClient(NativeLibraryConfig config)
    {
        ArgumentNullException.ThrowIfNull(config);
        if (config.Disabled)
        {
            throw new InvoiceKitException(
                "native_disabled",
                "InvoiceKit native engine client is disabled by configuration",
                "Use a REST sidecar client or pass a NativeLibraryConfig that points at invoicekit-ffi.");
        }

        libraryHandle = LoadLibrary(config);
        abiVersion = LoadExport<AbiVersionDelegate>("invoicekit_engine_abi_version");
        processJson = LoadExport<ProcessJsonDelegate>("invoicekit_engine_process_json");
        resultStatus = LoadExport<ResultStatusDelegate>("invoicekit_engine_result_status");
        resultBytes = LoadExport<ResultBytesDelegate>("invoicekit_engine_result_bytes");
        resultLen = LoadExport<ResultLenDelegate>("invoicekit_engine_result_len");
        resultFree = LoadExport<ResultFreeDelegate>("invoicekit_engine_result_free");

        AbiVersion = abiVersion();
        if (AbiVersion != EngineClients.EngineAbiVersion)
        {
            Dispose();
            throw new InvoiceKitException(
                "native_abi_version_mismatch",
                $"InvoiceKit native library reports ABI version {AbiVersion}; SDK expects {EngineClients.EngineAbiVersion}",
                "Use matching versions of the InvoiceKit .NET package and invoicekit-ffi native library.");
        }
    }

    /// <inheritdoc />
    public uint AbiVersion { get; }

    /// <inheritdoc />
    public EngineResult Process(byte[] requestBytes)
    {
        ThrowIfDisposed();
        ArgumentNullException.ThrowIfNull(requestBytes);

        IntPtr resultHandle;
        fixed (byte* requestPtr = requestBytes)
        {
            resultHandle = processJson(requestPtr, (nuint)requestBytes.Length);
        }

        if (resultHandle == IntPtr.Zero)
        {
            throw new InvoiceKitException(
                "native_null_result",
                "InvoiceKit native engine returned a null result handle",
                "Verify that invoicekit-ffi matches the documented Engine ABI contract.");
        }

        try
        {
            var status = (EngineStatusCode)resultStatus(resultHandle);
            var nativeLength = resultLen(resultHandle);
            if (nativeLength > int.MaxValue)
            {
                throw new InvoiceKitException(
                    "native_response_too_large",
                    "InvoiceKit native engine returned response bytes larger than this SDK can copy",
                    "Reduce the request size or process the document through a streaming sidecar endpoint.");
            }

            var length = (int)nativeLength;
            var bytesPtr = resultBytes(resultHandle);
            var responseBytes = new byte[length];
            if (length > 0)
            {
                if (bytesPtr == IntPtr.Zero)
                {
                    throw new InvoiceKitException(
                        "native_null_response_bytes",
                        "InvoiceKit native engine returned null response bytes with a non-zero length",
                        "Verify that invoicekit-ffi matches the documented Engine ABI contract.");
                }

                Marshal.Copy(bytesPtr, responseBytes, 0, length);
            }

            return new EngineResult(status, responseBytes);
        }
        finally
        {
            resultFree(resultHandle);
        }
    }

    /// <summary>
    /// Process an Engine ABI JSON request string.
    /// </summary>
    /// <param name="requestJson">Engine ABI JSON request text.</param>
    /// <returns>Copied Engine ABI response bytes plus status code.</returns>
    /// <exception cref="InvoiceKitException">Thrown when the native runtime cannot process the request.</exception>
    public EngineResult Process(string requestJson)
    {
        ArgumentNullException.ThrowIfNull(requestJson);
        return Process(Encoding.UTF8.GetBytes(requestJson));
    }

    /// <summary>
    /// Release the native library handle owned by this client.
    /// </summary>
    public void Dispose()
    {
        if (disposed)
        {
            return;
        }

        disposed = true;
        if (libraryHandle != IntPtr.Zero)
        {
            NativeLibrary.Free(libraryHandle);
        }
    }

    private static IntPtr LoadLibrary(NativeLibraryConfig config)
    {
        try
        {
            return config.LibraryPath is { Length: > 0 } path
                ? NativeLibrary.Load(path)
                : NativeLibrary.Load(LibraryName);
        }
        catch (Exception error) when (error is DllNotFoundException or BadImageFormatException or FileNotFoundException)
        {
            throw new InvoiceKitException(
                "native_library_load_error",
                "InvoiceKit native library could not be loaded",
                "Set INVOICEKIT_FFI_LIB to the full path of libinvoicekit_ffi.so, libinvoicekit_ffi.dylib, or invoicekit_ffi.dll, or use the REST sidecar fallback.",
                error);
        }
    }

    private T LoadExport<T>(string symbolName)
        where T : Delegate
    {
        try
        {
            var export = NativeLibrary.GetExport(libraryHandle, symbolName);
            return Marshal.GetDelegateForFunctionPointer<T>(export);
        }
        catch (Exception error) when (error is EntryPointNotFoundException or ArgumentNullException)
        {
            throw new InvoiceKitException(
                "native_symbol_missing",
                $"InvoiceKit native library is missing required C ABI symbol {symbolName}",
                "Use an invoicekit-ffi native library built from the same InvoiceKit release as this .NET package.",
                error);
        }
    }

    private void ThrowIfDisposed()
    {
        if (disposed)
        {
            throw new ObjectDisposedException(nameof(NativeEngineClient));
        }
    }

    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate uint AbiVersionDelegate();

    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate IntPtr ProcessJsonDelegate(byte* requestPtr, nuint requestLen);

    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate uint ResultStatusDelegate(IntPtr result);

    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate IntPtr ResultBytesDelegate(IntPtr result);

    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate nuint ResultLenDelegate(IntPtr result);

    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate void ResultFreeDelegate(IntPtr result);
}
