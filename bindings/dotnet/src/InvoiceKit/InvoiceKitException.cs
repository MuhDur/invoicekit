// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

namespace InvoiceKit;

/// <summary>
/// Exception raised by InvoiceKit SDK clients.
/// </summary>
public sealed class InvoiceKitException : Exception
{
    /// <summary>
    /// Create an InvoiceKit exception.
    /// </summary>
    /// <param name="code">Stable machine-readable error code.</param>
    /// <param name="message">Human-readable error message.</param>
    /// <param name="remediation">Human-readable remediation hint.</param>
    public InvoiceKitException(string code, string message, string remediation)
        : base(message)
    {
        Code = RequireNonBlank(code, nameof(code));
        Remediation = RequireNonBlank(remediation, nameof(remediation));
    }

    /// <summary>
    /// Create an InvoiceKit exception with an inner exception.
    /// </summary>
    /// <param name="code">Stable machine-readable error code.</param>
    /// <param name="message">Human-readable error message.</param>
    /// <param name="remediation">Human-readable remediation hint.</param>
    /// <param name="innerException">Underlying exception.</param>
    public InvoiceKitException(string code, string message, string remediation, Exception innerException)
        : base(message, innerException)
    {
        Code = RequireNonBlank(code, nameof(code));
        Remediation = RequireNonBlank(remediation, nameof(remediation));
    }

    /// <summary>
    /// Stable machine-readable error code.
    /// </summary>
    public string Code { get; }

    /// <summary>
    /// Human-readable remediation hint.
    /// </summary>
    public string Remediation { get; }

    private static string RequireNonBlank(string value, string parameterName)
    {
        if (string.IsNullOrWhiteSpace(value))
        {
            throw new ArgumentException("Value must not be blank.", parameterName);
        }

        return value;
    }
}
