// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

package dev.invoicekit;

import java.nio.charset.StandardCharsets;
import java.util.Arrays;
import java.util.Objects;

/** Copied Engine ABI result bytes plus the C ABI status code. */
public final class EngineResult {
    /** C ABI status for successful Engine ABI responses. */
    public static final int STATUS_OK = 0;
    /** C ABI status for canonical Engine ABI error responses. */
    public static final int STATUS_ERROR = 1;
    /** C ABI status for invalid native handles. */
    public static final int STATUS_INVALID_HANDLE = 2;

    private final int statusCode;
    private final byte[] responseBytes;

    /**
     * Create a result from copied response bytes.
     *
     * @param statusCode C ABI status code.
     * @param responseBytes response bytes copied from native memory or the REST sidecar.
     */
    public EngineResult(int statusCode, byte[] responseBytes) {
        this.statusCode = statusCode;
        this.responseBytes = Objects.requireNonNull(responseBytes, "responseBytes").clone();
    }

    /**
     * Return the C ABI status code.
     *
     * @return numeric status code reported by the backing runtime.
     */
    public int statusCode() {
        return statusCode;
    }

    /**
     * Return true when the backing runtime reported a successful Engine ABI response.
     *
     * @return true when {@link #statusCode()} is {@link #STATUS_OK}.
     */
    public boolean isOk() {
        return statusCode == STATUS_OK;
    }

    /**
     * Return a defensive copy of the response bytes.
     *
     * @return copied response bytes.
     */
    public byte[] responseBytes() {
        return responseBytes.clone();
    }

    /**
     * Decode response bytes as UTF-8 text.
     *
     * @return response bytes decoded as UTF-8.
     */
    public String responseText() {
        return new String(responseBytes, StandardCharsets.UTF_8);
    }

    @Override
    public boolean equals(Object other) {
        if (this == other) {
            return true;
        }
        if (!(other instanceof EngineResult that)) {
            return false;
        }
        return statusCode == that.statusCode && Arrays.equals(responseBytes, that.responseBytes);
    }

    @Override
    public int hashCode() {
        return 31 * Integer.hashCode(statusCode) + Arrays.hashCode(responseBytes);
    }

    @Override
    public String toString() {
        return "EngineResult{statusCode=" + statusCode + ", responseBytes=" + responseBytes.length + " bytes}";
    }
}
