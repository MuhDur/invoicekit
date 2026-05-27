// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

package dev.invoicekit;

import java.nio.charset.StandardCharsets;
import java.util.Objects;

/** Client for the stable InvoiceKit Engine ABI byte contract. */
public interface EngineClient {
    /**
     * Return the engine ABI version implemented by the backing runtime.
     *
     * @return engine ABI version number.
     * @throws InvoiceKitException when the backing runtime cannot report its ABI version.
     */
    int abiVersion() throws InvoiceKitException;

    /**
     * Process one Engine ABI JSON request.
     *
     * @param requestBytes UTF-8 JSON bytes matching the Engine ABI envelope.
     * @return status code and response bytes copied out of the backing runtime.
     * @throws InvoiceKitException when the native runtime or REST sidecar cannot process the request.
     */
    EngineResult process(byte[] requestBytes) throws InvoiceKitException;

    /**
     * Process one Engine ABI JSON request encoded as a Java string.
     *
     * @param requestJson Engine ABI JSON request text.
     * @return status code and response text bytes.
     * @throws InvoiceKitException when the backing runtime cannot process the request.
     */
    default EngineResult process(String requestJson) throws InvoiceKitException {
        Objects.requireNonNull(requestJson, "requestJson");
        return process(requestJson.getBytes(StandardCharsets.UTF_8));
    }
}
