// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

package dev.invoicekit;

import java.nio.file.Path;
import java.util.Locale;
import java.util.Optional;

/** Native library lookup settings for the InvoiceKit C ABI. */
public final class NativeLibraryConfig {
    private static final String DEFAULT_LIBRARY_NAME = "invoicekit_ffi";

    private final Path libraryPath;
    private final String libraryName;
    private final boolean nativeDisabled;

    private NativeLibraryConfig(Path libraryPath, String libraryName, boolean nativeDisabled) {
        this.libraryPath = libraryPath;
        this.libraryName = libraryName;
        this.nativeDisabled = nativeDisabled;
    }

    /**
     * Build lookup settings from environment variables.
     *
     * @return lookup settings derived from {@code INVOICEKIT_FFI_LIB},
     * {@code INVOICEKIT_FFI_LIBRARY_NAME}, and {@code INVOICEKIT_DISABLE_NATIVE}.
     */
    public static NativeLibraryConfig fromEnvironment() {
        String libraryPath = System.getenv("INVOICEKIT_FFI_LIB");
        String libraryName = System.getenv("INVOICEKIT_FFI_LIBRARY_NAME");
        String disabled = System.getenv("INVOICEKIT_DISABLE_NATIVE");
        return new NativeLibraryConfig(
                isBlank(libraryPath) ? null : Path.of(libraryPath),
                isBlank(libraryName) ? DEFAULT_LIBRARY_NAME : libraryName,
                isTruthy(disabled));
    }

    static NativeLibraryConfig disabledForTests() {
        return new NativeLibraryConfig(null, DEFAULT_LIBRARY_NAME, true);
    }

    Optional<Path> libraryPath() {
        return Optional.ofNullable(libraryPath);
    }

    String libraryName() {
        return libraryName;
    }

    boolean nativeDisabled() {
        return nativeDisabled;
    }

    private static boolean isBlank(String value) {
        return value == null || value.isBlank();
    }

    private static boolean isTruthy(String value) {
        if (isBlank(value)) {
            return false;
        }
        String normalized = value.trim().toLowerCase(Locale.ROOT);
        return normalized.equals("1") || normalized.equals("true") || normalized.equals("yes");
    }
}
