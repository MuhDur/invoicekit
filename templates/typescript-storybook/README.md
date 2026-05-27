# @invoicekit/template-storybook

T-114 Storybook showcase for every InvoiceKit Typst template plus the
allowance / reverse-charge variants the strict gate calls out.

## Stories

| Template            | Stories                                |
|---------------------|----------------------------------------|
| `basic-invoice`     | Base · With volume rebate · Reverse charge |
| `tax-breakdown`     | Base · With allowance · Reverse charge     |
| `credit-note`       | Base                                   |
| `factur-x-summary`  | Base                                   |
| `payment-reminder`  | Base                                   |

Each story renders the compiled Typst source that the template
language emits for the given data.

## Run locally

```bash
cd templates/typescript-storybook
bun install
bun run storybook       # http://localhost:6006
```

## Production build

```bash
bun run build-storybook
# → templates/typescript-storybook/storybook-static/
```

The CI workflow `.github/workflows/storybook-templates.yml`
runs `bun test` (4 unit assertions on the story metadata
shape) plus `bun run build-storybook` so every PR proves the
showcase compiles.

## Why no @invoicekit/template-typst package import?

The templates package does not yet expose a `main` / `exports`
entry (it's `private: true` and consumed by file path inside the
repo). The stories import via the relative path `../../typescript/src/index.ts`
so the workspace link is self-contained.

## License

Apache-2.0.

Implemented by `invoices-t-114-storybook-templates-e8fe`.
