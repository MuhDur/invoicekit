# invoicekit/validate-action

[![Marketplace](https://img.shields.io/badge/Marketplace-invoicekit%2Fvalidate--action-blue)](https://github.com/marketplace/actions/invoicekit-validate)

Validate UBL, CII, and Peppol invoices in any GitHub repo
against EN 16931 + Peppol BIS rules — directly in CI.

## Usage

```yaml
name: invoices
on: [pull_request, push]

jobs:
  validate:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: invoicekit/validate-action@v1
        with:
          path: "invoices/**/*.xml"
          rule-pack: "en16931-2017+peppol-bis-3.0"
          fail-on: error
```

The action installs the [InvoiceKit CLI](https://invoicekit.org/cli),
runs `invoicekit validate` against each matched file, prints
findings as GitHub annotations on the offending lines, uploads
a JSON summary as an artefact, and fails the job when any
finding meets or exceeds `fail-on`.

## Inputs

| name | default | description |
| --- | --- | --- |
| `path` | `invoices/**/*.{xml,ubl,cii}` | Glob of files to validate. |
| `rule-pack` | `en16931-2017+peppol-bis-3.0` | One of: `en16931-2017`, `en16931-2017+peppol-bis-3.0`, `xrechnung-3.0`. |
| `fail-on` | `error` | Highest severity that does NOT fail the job: `fatal`, `error`, `warning`, `info`. |
| `invoicekit-version` | `latest` | Pin the CLI version (or `latest`). |
| `summary-artifact` | `invoicekit-validate-summary` | Name of the uploaded JSON artefact (empty disables upload). |

## Outputs

| name | description |
| --- | --- |
| `fatal-count` | Number of fatal findings. |
| `error-count` | Number of error findings. |
| `warning-count` | Number of warning findings. |
| `files-validated` | How many files matched the glob. |
| `summary-path` | Path (relative to `$GITHUB_WORKSPACE`) of the JSON summary. |

## Example: gate releases on zero findings

```yaml
- uses: invoicekit/validate-action@v1
  id: ik
  with:
    path: "release/*.xml"
    rule-pack: "xrechnung-3.0"
    fail-on: warning
- name: Tag release
  if: steps.ik.outputs.error-count == '0' && steps.ik.outputs.warning-count == '0'
  run: gh release create v${{ github.run_number }}
```

## Example: post a PR comment with the summary

```yaml
- uses: invoicekit/validate-action@v1
  id: ik
- uses: actions/github-script@v7
  if: always() && github.event_name == 'pull_request'
  with:
    script: |
      const fs = require('fs');
      const summary = JSON.parse(fs.readFileSync('${{ steps.ik.outputs.summary-path }}'));
      const body = [
        '## InvoiceKit validate',
        '',
        '| Severity | Count |',
        '| --- | ---: |',
        `| fatal | ${summary.findings.filter(f => f.severity === 'fatal').length} |`,
        `| error | ${summary.findings.filter(f => f.severity === 'error').length} |`,
        `| warning | ${summary.findings.filter(f => f.severity === 'warning').length} |`,
      ].join('\n');
      await github.rest.issues.createComment({
        issue_number: context.issue.number,
        owner: context.repo.owner,
        repo: context.repo.repo,
        body,
      });
```

## Publishing to GitHub Marketplace

This action is shipped under `invoicekit/validate-action`.
Tag a release `v1.x.y` from the
[InvoiceKit repo](https://github.com/MuhDur/invoicekit) and
publish via the GitHub Marketplace flow on that release. See
`docs/operators/VALIDATE-ACTION.md` for the full publishing
runbook.

## License

Apache 2.0.
