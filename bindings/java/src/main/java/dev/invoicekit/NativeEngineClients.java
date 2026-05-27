// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

package dev.invoicekit;

import java.lang.reflect.InvocationTargetException;
import java.lang.reflect.Method;

final class NativeEngineClients {
    private NativeEngineClients() {}

    static EngineClient create(NativeLibraryConfig config) throws InvoiceKitException {
        if (config.nativeDisabled()) {
            throw unavailable("native loading disabled by configuration");
        }
        if (Runtime.version().feature() < 22) {
            throw unavailable("Java " + Runtime.version().feature() + " does not expose the final FFM API");
        }

        try {
            Class<?> provider = Class.forName("dev.invoicekit.FfmEngineClient");
            Method create = provider.getDeclaredMethod("create", NativeLibraryConfig.class);
            EngineClient client = (EngineClient) create.invoke(null, config);
            int abiVersion = client.abiVersion();
            if (abiVersion != InvoiceKit.ENGINE_ABI_VERSION) {
                throw new InvoiceKitException(
                        "native_abi_mismatch",
                        "InvoiceKit native engine ABI version "
                                + abiVersion
                                + " does not match Java SDK ABI version "
                                + InvoiceKit.ENGINE_ABI_VERSION,
                        "Use an invoicekit-ffi shared library built from the same InvoiceKit release, "
                                + "or use InvoiceKit.nativeOrSidecar with a compatible REST sidecar.");
            }
            return client;
        } catch (ClassNotFoundException error) {
            throw unavailable("Java 22 FFM provider is not present in this artifact", error);
        } catch (NoSuchMethodException | IllegalAccessException error) {
            throw unavailable("Java 22 FFM provider has an incompatible shape", error);
        } catch (InvocationTargetException error) {
            Throwable cause = error.getCause();
            if (cause instanceof InvoiceKitException invoiceKitException) {
                throw invoiceKitException;
            }
            throw unavailable("Java 22 FFM provider failed during native binding", cause);
        }
    }

    private static InvoiceKitException unavailable(String detail) {
        return unavailable(detail, null);
    }

    private static InvoiceKitException unavailable(String detail, Throwable cause) {
        return new InvoiceKitException(
                "native_unavailable",
                "InvoiceKit native engine is unavailable: " + detail,
                "Set INVOICEKIT_FFI_LIB to the invoicekit-ffi shared library path, run on Java 22 or newer, "
                        + "or use InvoiceKit.nativeOrSidecar with a running REST sidecar.",
                cause);
    }
}
