// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

package dev.invoicekit;

import java.util.Objects;

/** Checked exception carrying a stable code and a user-facing remediation hint. */
public final class InvoiceKitException extends Exception {
    private static final long serialVersionUID = 1L;

    /** Stable machine-readable error code. */
    private final String code;
    /** User-facing remediation hint. */
    private final String remediation;

    /**
     * Create an exception with a stable code and remediation.
     *
     * @param code stable machine-readable error code.
     * @param message user-facing error message.
     * @param remediation user-facing remediation hint.
     */
    public InvoiceKitException(String code, String message, String remediation) {
        super(message);
        this.code = Objects.requireNonNull(code, "code");
        this.remediation = Objects.requireNonNull(remediation, "remediation");
    }

    /**
     * Create an exception with a cause, stable code, and remediation.
     *
     * @param code stable machine-readable error code.
     * @param message user-facing error message.
     * @param remediation user-facing remediation hint.
     * @param cause underlying cause.
     */
    public InvoiceKitException(String code, String message, String remediation, Throwable cause) {
        super(message, cause);
        this.code = Objects.requireNonNull(code, "code");
        this.remediation = Objects.requireNonNull(remediation, "remediation");
    }

    /**
     * Return a stable machine-readable error code.
     *
     * @return stable machine-readable error code.
     */
    public String code() {
        return code;
    }

    /**
     * Return a user-facing remediation hint.
     *
     * @return user-facing remediation hint.
     */
    public String remediation() {
        return remediation;
    }
}
