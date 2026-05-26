// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

using System.Runtime.InteropServices;
using System.Text;
using System.Text.Json;
using System.Text.Json.Serialization;

var root = FindRepoRoot();
var fixturePath = Path.Combine(root, "conformance-corpus", "golden", "engine-abi-v1-commercial-document.json");
var fixture = JsonSerializer.Deserialize<GoldenFixture>(
    File.ReadAllText(fixturePath),
    new JsonSerializerOptions { PropertyNameCaseInsensitive = true })
    ?? throw new InvalidOperationException("golden fixture was empty");

var request = Encoding.UTF8.GetBytes(fixture.RequestBytes);
var expected = Encoding.UTF8.GetBytes(fixture.ExpectedResponseBytes);

unsafe
{
    fixed (byte* requestPtr = request)
    {
        var result = NativeMethods.Process(requestPtr, (UIntPtr)request.Length);
        if (result == IntPtr.Zero)
        {
            throw new InvalidOperationException("invoicekit_engine_process_json returned null");
        }

        try
        {
            var status = NativeMethods.Status(result);
            if (status != 0)
            {
                throw new InvalidOperationException($"expected status 0, got {status}");
            }

            var responseLength = NativeMethods.Length(result).ToUInt64();
            if (responseLength > int.MaxValue)
            {
                throw new InvalidOperationException($"response too large for .NET test: {responseLength}");
            }
            var len = (int)responseLength;
            var responsePtr = NativeMethods.Bytes(result);
            if (responsePtr == IntPtr.Zero)
            {
                throw new InvalidOperationException("invoicekit_engine_result_bytes returned null");
            }

            var actual = new byte[len];
            Marshal.Copy(responsePtr, actual, 0, len);
            if (!actual.AsSpan().SequenceEqual(expected))
            {
                throw new InvalidOperationException(".NET P/Invoke ABI response did not match golden bytes");
            }
        }
        finally
        {
            NativeMethods.Free(result);
        }
    }
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

internal static unsafe partial class NativeMethods
{
    static NativeMethods()
    {
        NativeLibrary.SetDllImportResolver(typeof(NativeMethods).Assembly, ResolveInvoiceKit);
    }

    [DllImport("invoicekit_ffi", EntryPoint = "invoicekit_engine_process_json", CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr Process(byte* requestPtr, UIntPtr requestLen);

    [DllImport("invoicekit_ffi", EntryPoint = "invoicekit_engine_result_status", CallingConvention = CallingConvention.Cdecl)]
    internal static extern uint Status(IntPtr result);

    [DllImport("invoicekit_ffi", EntryPoint = "invoicekit_engine_result_bytes", CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr Bytes(IntPtr result);

    [DllImport("invoicekit_ffi", EntryPoint = "invoicekit_engine_result_len", CallingConvention = CallingConvention.Cdecl)]
    internal static extern UIntPtr Length(IntPtr result);

    [DllImport("invoicekit_ffi", EntryPoint = "invoicekit_engine_result_free", CallingConvention = CallingConvention.Cdecl)]
    internal static extern void Free(IntPtr result);

    private static IntPtr ResolveInvoiceKit(string libraryName, System.Reflection.Assembly assembly, DllImportSearchPath? searchPath)
    {
        if (libraryName != "invoicekit_ffi")
        {
            return IntPtr.Zero;
        }

        var overridePath = Environment.GetEnvironmentVariable("INVOICEKIT_FFI_LIB");
        return string.IsNullOrWhiteSpace(overridePath)
            ? IntPtr.Zero
            : NativeLibrary.Load(overridePath);
    }
}
