# InvoiceKit Engine C ABI v1

The C ABI is a small ownership layer around the engine's canonical JSON byte
contract. All structured inputs and outputs remain JSON bytes; C callers never
receive borrowed Rust objects or language-specific invoice structs.

## Versioning

`invoicekit_engine_abi_version()` returns `1`. The v1 ABI is append-only:
existing function names, argument order, result ownership, and status-code
values must not change without a major ABI version.

## Request Contract

`invoicekit_engine_process_json(request_ptr, request_len)` accepts UTF-8 JSON
bytes with this envelope:

```json
{
  "abi_version": 1,
  "operation": "commercial_document.canonicalize",
  "payload": {}
}
```

The engine canonicalizes the request before processing, rejects duplicate object
members, validates `payload` as an `invoicekit-ir` `CommercialDocument`, and
returns canonical JSON bytes. Unknown top-level envelope fields are rejected in
ABI v1 so every accepted request has an explicit contract.

## Response Contract

Successful responses have this canonical JSON shape:

```json
{
  "abi_version": 1,
  "operation": "commercial_document.canonicalize",
  "payload": {
    "canonical_document_json": "...",
    "document": {}
  },
  "status": "ok"
}
```

Error responses have this canonical JSON shape:

```json
{
  "abi_version": 1,
  "error": {
    "code": "invalid_request_json",
    "message": "...",
    "remediation": "..."
  },
  "operation": null,
  "status": "error"
}
```

## Ownership

- `invoicekit_engine_process_json` returns an owned opaque
  `InvoiceKitEngineResult *`.
- `invoicekit_engine_result_bytes` returns a borrowed pointer that remains valid
  until `invoicekit_engine_result_free`.
- Callers must copy the bytes before freeing the result.
- `invoicekit_engine_result_free` accepts null and otherwise must be called
  exactly once for each result handle.

## Functions

```c
uint32_t invoicekit_engine_abi_version(void);

InvoiceKitEngineResult *invoicekit_engine_process_json(
    const uint8_t *request_ptr,
    size_t request_len
);

uint32_t invoicekit_engine_result_status(const InvoiceKitEngineResult *result);
const uint8_t *invoicekit_engine_result_bytes(const InvoiceKitEngineResult *result);
size_t invoicekit_engine_result_len(const InvoiceKitEngineResult *result);
void invoicekit_engine_result_free(InvoiceKitEngineResult *result);
```

## Status Codes

| Code | Name | Meaning |
| --- | --- | --- |
| 0 | `Ok` | Response bytes contain a successful `status:"ok"` engine response. |
| 1 | `Error` | Response bytes contain a canonical `status:"error"` engine response. |
| 2 | `InvalidHandle` | A result accessor received a null handle. |

Passing a freed non-null handle is undefined behavior; the ABI can only identify
null handles.
