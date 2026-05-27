// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

namespace InvoiceKit;

/// <summary>
/// Status code returned by the InvoiceKit C ABI.
/// </summary>
public enum EngineStatusCode : uint
{
    /// <summary>
    /// The engine operation completed successfully.
    /// </summary>
    Ok = 0,

    /// <summary>
    /// The engine returned a canonical JSON error response.
    /// </summary>
    Error = 1,

    /// <summary>
    /// A native result accessor received a null result handle.
    /// </summary>
    InvalidHandle = 2,
}
