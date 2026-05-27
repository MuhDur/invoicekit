// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors
//
// T-1407 demo fixtures: three German XRechnung-shaped
// CommercialDocuments. Identical canonical shape to the FastAPI
// / Django / Next.js / Spring Boot demos so the cross-language
// gates exercise the same canonicalize output.

package main

func sellerParty() map[string]any {
	return map[string]any{
		"name": "Acme GmbH",
		"tax_ids": []map[string]string{
			{"scheme": "vat", "value": "DE123456789"},
		},
		"address": map[string]any{
			"lines":       []string{"Hauptstraße 42"},
			"city":        "Berlin",
			"postal_code": "10115",
			"country":     "DE",
		},
	}
}

func buyerParty() map[string]any {
	return map[string]any{
		"name": "Beispielkunde AG",
		"tax_ids": []map[string]string{
			{"scheme": "vat", "value": "DE987654321"},
		},
		"address": map[string]any{
			"lines":       []string{"Friedrichstraße 10"},
			"city":        "München",
			"postal_code": "80331",
			"country":     "DE",
		},
	}
}

func atBuyerParty() map[string]any {
	return map[string]any{
		"name": "Beispielkunde AG",
		"tax_ids": []map[string]string{
			{"scheme": "vat", "value": "ATU12345678"},
		},
		"address": map[string]any{
			"lines":       []string{"Stephansplatz 1"},
			"city":        "Wien",
			"postal_code": "1010",
			"country":     "AT",
		},
	}
}

func meta(name string) map[string]string {
	return map[string]string{
		"tenant_id": "tenant-demo-go-chi",
		"trace_id":  "trace-go-chi-" + name,
	}
}

// fixtures returns three German XRechnung shapes by name.
func fixtures() map[string]map[string]any {
	return map[string]map[string]any{
		"basic": {
			"schema_version":       "1.0",
			"id":                   "doc-de-gochi-basic-2026-0001",
			"document_type":        "invoice",
			"issue_date":           "2026-05-27",
			"due_date":             "2026-06-26",
			"document_number":      "RE-GO-2026-0001",
			"currency":             "EUR",
			"supplier":             sellerParty(),
			"customer":             buyerParty(),
			"payment_instructions": []any{},
			"lines": []map[string]any{
				{
					"id":                    "L1",
					"description":           "Software-Lizenz Q3/2026",
					"quantity":              "1",
					"unit_price":            "1000.00",
					"line_extension_amount": "1000.00",
					"tax_category":          "S",
					"extensions":            []any{},
				},
			},
			"tax_summary": []map[string]string{
				{
					"category_code":  "S",
					"taxable_amount": "1000.00",
					"tax_amount":     "190.00",
					"tax_rate":       "19.00",
				},
			},
			"monetary_total": map[string]string{
				"line_extension_amount": "1000.00",
				"tax_exclusive_amount":  "1000.00",
				"tax_inclusive_amount":  "1190.00",
				"payable_amount":        "1190.00",
			},
			"extensions": []any{},
			"meta":       meta("basic"),
		},
		"with-allowance": {
			"schema_version":       "1.0",
			"id":                   "doc-de-gochi-allowance-2026-0002",
			"document_type":        "invoice",
			"issue_date":           "2026-05-27",
			"due_date":             "2026-06-26",
			"document_number":      "RE-GO-2026-0002",
			"currency":             "EUR",
			"supplier":             sellerParty(),
			"customer":             buyerParty(),
			"payment_instructions": []any{},
			"lines": []map[string]any{
				{
					"id":                    "L1",
					"description":           "Beratungsleistung März 2026",
					"quantity":              "10",
					"unit_price":            "150.00",
					"line_extension_amount": "1500.00",
					"tax_category":          "S",
					"extensions":            []any{},
				},
				{
					"id":                    "L2",
					"description":           "Mengenrabatt 10%",
					"quantity":              "-1",
					"unit_price":            "150.00",
					"line_extension_amount": "-150.00",
					"tax_category":          "S",
					"extensions":            []any{},
				},
			},
			"tax_summary": []map[string]string{
				{
					"category_code":  "S",
					"taxable_amount": "1350.00",
					"tax_amount":     "256.50",
					"tax_rate":       "19.00",
				},
			},
			"monetary_total": map[string]string{
				"line_extension_amount": "1350.00",
				"tax_exclusive_amount":  "1350.00",
				"tax_inclusive_amount":  "1606.50",
				"payable_amount":        "1606.50",
			},
			"extensions": []any{},
			"meta":       meta("with-allowance"),
		},
		"reverse-charge": {
			"schema_version":       "1.0",
			"id":                   "doc-de-gochi-rc-2026-0003",
			"document_type":        "invoice",
			"issue_date":           "2026-05-27",
			"due_date":             "2026-06-26",
			"document_number":      "RE-GO-2026-0003",
			"currency":             "EUR",
			"supplier":             sellerParty(),
			"customer":             atBuyerParty(),
			"payment_instructions": []any{},
			"lines": []map[string]any{
				{
					"id":                    "L1",
					"description":           "Wartungsvertrag Q3/2026",
					"quantity":              "1",
					"unit_price":            "5000.00",
					"line_extension_amount": "5000.00",
					"tax_category":          "AE",
					"extensions":            []any{},
				},
			},
			"tax_summary": []map[string]string{
				{
					"category_code":  "AE",
					"taxable_amount": "5000.00",
					"tax_amount":     "0.00",
					"tax_rate":       "0.00",
				},
			},
			"monetary_total": map[string]string{
				"line_extension_amount": "5000.00",
				"tax_exclusive_amount":  "5000.00",
				"tax_inclusive_amount":  "5000.00",
				"payable_amount":        "5000.00",
			},
			"extensions": []any{},
			"meta":       meta("reverse-charge"),
		},
	}
}
