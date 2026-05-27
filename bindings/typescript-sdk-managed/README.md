# @invoicekit/managed

Typed REST client for the InvoiceKit managed-API gateway.
Currently covers:

- `GET /v1/audit/events` — paginated audit-log query.

Additional endpoints (`/v1/reconcile`, `/v1/events/sse`) land as
their gateway routes wire up.

```ts
import { createManagedApiClient, ManagedApiError } from "@invoicekit/managed";

const client = createManagedApiClient({
  baseUrl: "https://api.invoicekit.example",
  apiKey: process.env.INVOICEKIT_API_KEY!,
});

try {
  const page = await client.getAuditEvents({ limit: 50 });
  console.log(page.events.length, "events");
} catch (err) {
  if (err instanceof ManagedApiError) {
    console.error(err.code, err.status, err.message);
  } else {
    throw err;
  }
}
```

Pass a `fetch` implementation to swap in undici / a polyfill /
a test mock. Defaults to `globalThis.fetch`.

## License

Apache-2.0.
