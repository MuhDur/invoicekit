# InvoiceKit TypeScript Template Language

This package is Layer B of the rendering stack: TypeScript authors build invoice
templates with a typed DSL, and InvoiceKit compiles those templates to Typst.
Template authors do not write Typst syntax directly.

Run the local checks:

```bash
bun install
bun run verify
```

The five example templates live in `examples/`:

- `basic-invoice.ts`
- `credit-note.ts`
- `factur-x-summary.ts`
- `payment-reminder.ts`
- `tax-breakdown.ts`

Golden Typst outputs live in `golden/`. To intentionally update them:

```bash
UPDATE_GOLDENS=1 bun test
bun test
```

Type safety is checked by `tests/type-safety.ts`. The negative cases use
`@ts-expect-error`, so `bun run typecheck` fails if missing invoice fields stop
being caught at compile time.
