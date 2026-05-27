// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

using System.Net;
using System.Net.Sockets;
using System.Text;
using Xunit;

namespace InvoiceKit.Tests;

public sealed class RestSidecarEngineClientTests
{
    [Fact]
    public void NativeOrSidecarFallsBackToRestWhenNativeIsUnavailable()
    {
        var response = Encoding.UTF8.GetBytes("{\"status\":\"ok\"}");
        using var sidecar = TestSidecar.Responding(response, EngineStatusCode.Ok);

        using var client = EngineClients.NativeOrSidecar(sidecar.Endpoint, NativeLibraryConfig.DisabledForTests());
        var result = client.Process("{\"abi_version\":1,\"operation\":\"unknown\",\"payload\":{}}");

        Assert.Equal(EngineStatusCode.Ok, result.StatusCode);
        Assert.Equal(response, result.ResponseBytes());
        Assert.Equal("{\"abi_version\":1,\"operation\":\"unknown\",\"payload\":{}}", sidecar.CapturedRequestText);
    }

    [Fact]
    public void RestSidecarPreservesCanonicalErrorStatusHeader()
    {
        using var sidecar = TestSidecar.Responding(Encoding.UTF8.GetBytes("{\"status\":\"error\"}"), EngineStatusCode.Error);

        using var client = EngineClients.RestSidecar(sidecar.Endpoint);
        var result = client.Process(Array.Empty<byte>());

        Assert.Equal(EngineStatusCode.Error, result.StatusCode);
        Assert.Equal("{\"status\":\"error\"}", result.ResponseText());
    }

    [Fact]
    public void RestSidecarMapsInvalidStatusHeaderToError()
    {
        using var sidecar = TestSidecar.Responding(Encoding.UTF8.GetBytes("{\"status\":\"ok\"}"), "not-a-number");

        using var client = EngineClients.RestSidecar(sidecar.Endpoint);
        var result = client.Process(Array.Empty<byte>());

        Assert.Equal(EngineStatusCode.Error, result.StatusCode);
    }

    [Fact]
    public void RestSidecarTurnsHttpErrorsIntoTypedException()
    {
        using var sidecar = TestSidecar.WithHttpStatus(HttpStatusCode.ServiceUnavailable);

        var error = Assert.Throws<InvoiceKitException>(
            () =>
            {
                using var client = EngineClients.RestSidecar(sidecar.Endpoint);
                client.Process(Array.Empty<byte>());
            });

        Assert.Equal("sidecar_http_error", error.Code);
    }

    [Fact]
    public void RestSidecarRejectsRelativeEndpoint()
    {
        var error = Assert.Throws<ArgumentException>(
            () => EngineClients.RestSidecar(new Uri("/engine/process", UriKind.Relative)));

        Assert.Equal("endpoint", error.ParamName);
    }

    private sealed class TestSidecar : IDisposable
    {
        private readonly TcpListener listener;
        private readonly Thread worker;
        private readonly byte[] response;
        private readonly string? statusHeader;
        private readonly HttpStatusCode statusCode;

        private TestSidecar(byte[] response, string? statusHeader, HttpStatusCode statusCode)
        {
            this.response = response;
            this.statusHeader = statusHeader;
            this.statusCode = statusCode;

            listener = new TcpListener(IPAddress.Loopback, 0);
            listener.Start();
            var port = ((IPEndPoint)listener.LocalEndpoint).Port;
            Endpoint = new Uri($"http://127.0.0.1:{port}/engine/process");
            worker = new Thread(HandleOneRequest) { IsBackground = true };
            worker.Start();
        }

        public Uri Endpoint { get; }

        public string? CapturedRequestText { get; private set; }

        public static TestSidecar Responding(byte[] response, EngineStatusCode engineStatus)
        {
            return new TestSidecar(response, ((uint)engineStatus).ToString(), HttpStatusCode.OK);
        }

        public static TestSidecar Responding(byte[] response, string statusHeader)
        {
            return new TestSidecar(response, statusHeader, HttpStatusCode.OK);
        }

        public static TestSidecar WithHttpStatus(HttpStatusCode statusCode)
        {
            return new TestSidecar(Encoding.UTF8.GetBytes("{}"), ((uint)EngineStatusCode.Error).ToString(), statusCode);
        }

        public void Dispose()
        {
            listener.Stop();
            if (!worker.Join(TimeSpan.FromSeconds(2)))
            {
                throw new TimeoutException("test sidecar thread did not stop");
            }
        }

        private void HandleOneRequest()
        {
            try
            {
                using var client = listener.AcceptTcpClient();
                using var stream = client.GetStream();
                CapturedRequestText = Encoding.UTF8.GetString(ReadRequestBody(stream));

                var header = new StringBuilder();
                header.Append("HTTP/1.1 ");
                header.Append((int)statusCode);
                header.Append(' ');
                header.Append(statusCode);
                header.Append("\r\nContent-Type: application/json\r\nContent-Length: ");
                header.Append(response.Length);
                header.Append("\r\nConnection: close\r\n");
                if (statusHeader is not null)
                {
                    header.Append(RestSidecarEngineClient.StatusHeader);
                    header.Append(": ");
                    header.Append(statusHeader);
                    header.Append("\r\n");
                }

                header.Append("\r\n");
                stream.Write(Encoding.ASCII.GetBytes(header.ToString()));
                stream.Write(response);
            }
            catch (SocketException)
            {
            }
            catch (ObjectDisposedException)
            {
            }
            catch (IOException)
            {
            }
        }

        private static byte[] ReadRequestBody(NetworkStream stream)
        {
            using var memory = new MemoryStream();
            var buffer = new byte[4096];
            while (true)
            {
                var read = stream.Read(buffer, 0, buffer.Length);
                if (read == 0)
                {
                    break;
                }

                memory.Write(buffer, 0, read);
                var bytes = memory.ToArray();
                var headerEnd = IndexOf(bytes, "\r\n\r\n"u8.ToArray());
                if (headerEnd < 0)
                {
                    continue;
                }

                var headers = Encoding.ASCII.GetString(bytes, 0, headerEnd);
                var contentLength = ParseContentLength(headers);
                var bodyStart = headerEnd + 4;
                if (bytes.Length >= bodyStart + contentLength)
                {
                    return bytes.Skip(bodyStart).Take(contentLength).ToArray();
                }
            }

            return Array.Empty<byte>();
        }

        private static int ParseContentLength(string headers)
        {
            foreach (var line in headers.Split("\r\n"))
            {
                var separator = line.IndexOf(':', StringComparison.Ordinal);
                if (separator <= 0)
                {
                    continue;
                }

                var name = line[..separator];
                if (name.Equals("Content-Length", StringComparison.OrdinalIgnoreCase))
                {
                    return int.TryParse(
                        line[(separator + 1)..].Trim(),
                        System.Globalization.NumberStyles.None,
                        System.Globalization.CultureInfo.InvariantCulture,
                        out var contentLength)
                        ? contentLength
                        : 0;
                }
            }

            return 0;
        }

        private static int IndexOf(byte[] haystack, byte[] needle)
        {
            for (var i = 0; i <= haystack.Length - needle.Length; i++)
            {
                if (haystack.AsSpan(i, needle.Length).SequenceEqual(needle))
                {
                    return i;
                }
            }

            return -1;
        }
    }
}
