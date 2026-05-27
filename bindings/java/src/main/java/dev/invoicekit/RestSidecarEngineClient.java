// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

package dev.invoicekit;

import java.io.IOException;
import java.net.URI;
import java.net.http.HttpClient;
import java.net.http.HttpRequest;
import java.net.http.HttpResponse;
import java.time.Duration;
import java.util.Objects;

/** REST sidecar fallback for runtimes where the native library is unavailable. */
public final class RestSidecarEngineClient implements EngineClient {
    /** Header used by the sidecar to preserve the C ABI status code. */
    public static final String STATUS_HEADER = "X-InvoiceKit-Status-Code";

    private final HttpClient httpClient;
    private final URI processEndpoint;

    /**
     * Create a sidecar client for a full Engine ABI process endpoint.
     *
     * @param processEndpoint full sidecar endpoint that accepts Engine ABI JSON bytes.
     */
    public RestSidecarEngineClient(URI processEndpoint) {
        this(HttpClient.newHttpClient(), processEndpoint);
    }

    RestSidecarEngineClient(HttpClient httpClient, URI processEndpoint) {
        this.httpClient = Objects.requireNonNull(httpClient, "httpClient");
        this.processEndpoint = requireAbsolute(processEndpoint);
    }

    @Override
    public int abiVersion() {
        return InvoiceKit.ENGINE_ABI_VERSION;
    }

    @Override
    public EngineResult process(byte[] requestBytes) throws InvoiceKitException {
        Objects.requireNonNull(requestBytes, "requestBytes");
        HttpRequest request = HttpRequest.newBuilder(processEndpoint)
                .timeout(Duration.ofSeconds(30))
                .header("Content-Type", "application/json")
                .header("Accept", "application/json")
                .POST(HttpRequest.BodyPublishers.ofByteArray(requestBytes))
                .build();
        HttpResponse<byte[]> response;
        try {
            response = httpClient.send(request, HttpResponse.BodyHandlers.ofByteArray());
        } catch (IOException error) {
            throw new InvoiceKitException(
                    "sidecar_io_error",
                    "InvoiceKit REST sidecar request failed",
                    "Start the InvoiceKit REST sidecar and pass its Engine ABI endpoint URI.",
                    error);
        } catch (InterruptedException error) {
            Thread.currentThread().interrupt();
            throw new InvoiceKitException(
                    "sidecar_interrupted",
                    "InvoiceKit REST sidecar request was interrupted",
                    "Retry the request after the calling thread is allowed to run.",
                    error);
        }
        if (response.statusCode() < 200 || response.statusCode() >= 300) {
            throw new InvoiceKitException(
                    "sidecar_http_error",
                    "InvoiceKit REST sidecar returned HTTP " + response.statusCode(),
                    "Check the sidecar URL, health endpoint, and server logs.");
        }
        int statusCode = response.headers()
                .firstValue(STATUS_HEADER)
                .map(RestSidecarEngineClient::parseStatusHeader)
                .orElse(EngineResult.STATUS_OK);
        return new EngineResult(statusCode, response.body());
    }

    private static URI requireAbsolute(URI endpoint) {
        Objects.requireNonNull(endpoint, "processEndpoint");
        if (!endpoint.isAbsolute()) {
            throw new IllegalArgumentException("processEndpoint must be an absolute URI");
        }
        return endpoint;
    }

    private static int parseStatusHeader(String value) {
        try {
            return Integer.parseUnsignedInt(value);
        } catch (NumberFormatException error) {
            return EngineResult.STATUS_ERROR;
        }
    }
}
