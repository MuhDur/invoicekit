// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

package dev.invoicekit;

import java.net.URI;
import java.util.Objects;

/** Factory methods for InvoiceKit Java SDK clients. */
public final class InvoiceKit {
    /** Engine ABI version implemented by this SDK generation. */
    public static final int ENGINE_ABI_VERSION = 1;

    private InvoiceKit() {}

    /**
     * Create the native client for the current process.
     *
     * <p>On Java 22 and newer this uses the Foreign Function and Memory API to call the
     * InvoiceKit C ABI. On older runtimes, or when the native library cannot be found,
     * this method throws with a remediation hint. Use {@link #nativeOrSidecar(URI)}
     * when a REST sidecar fallback should be used automatically.</p>
     *
     * @return native Engine ABI client.
     * @throws InvoiceKitException when the native runtime is unavailable.
     */
    public static EngineClient nativeClient() throws InvoiceKitException {
        return nativeClient(NativeLibraryConfig.fromEnvironment());
    }

    static EngineClient nativeClient(NativeLibraryConfig config) throws InvoiceKitException {
        return NativeEngineClients.create(config);
    }

    /**
     * Create a REST sidecar client for a full Engine ABI process endpoint URI.
     *
     * @param processEndpoint full sidecar endpoint that accepts Engine ABI JSON bytes.
     * @return REST sidecar client.
     */
    public static EngineClient restSidecar(URI processEndpoint) {
        return new RestSidecarEngineClient(processEndpoint);
    }

    /**
     * Prefer the native client and fall back to a REST sidecar when native loading fails.
     *
     * @param processEndpoint full sidecar endpoint that accepts Engine ABI JSON bytes.
     * @return native client when available, otherwise a REST sidecar client.
     */
    public static EngineClient nativeOrSidecar(URI processEndpoint) {
        return nativeOrSidecar(processEndpoint, NativeLibraryConfig.fromEnvironment());
    }

    static EngineClient nativeOrSidecar(URI processEndpoint, NativeLibraryConfig config) {
        Objects.requireNonNull(processEndpoint, "processEndpoint");
        try {
            return nativeClient(config);
        } catch (InvoiceKitException unavailable) {
            return restSidecar(processEndpoint);
        }
    }

    /**
     * Process an Engine ABI JSON request and return UTF-8 response text.
     *
     * @param client selected Engine ABI client.
     * @param requestJson Engine ABI JSON request text.
     * @return response bytes decoded as UTF-8 text.
     * @throws InvoiceKitException when the selected client cannot process the request.
     */
    public static String processEngineAbiJson(EngineClient client, String requestJson) throws InvoiceKitException {
        Objects.requireNonNull(client, "client");
        return client.process(requestJson).responseText();
    }
}
