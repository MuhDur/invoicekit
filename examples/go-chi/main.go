// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors
//
// T-1407 reference Go (chi) demo: canonicalises three German
// XRechnung fixtures through the InvoiceKit Rust engine using
// the @invoicekit/go SDK from T-107 (cgo path).

package main

import (
	"encoding/json"
	"fmt"
	"log"
	"net/http"
	"sort"

	invoicekit "github.com/MuhDur/invoicekit/bindings/go"
	"github.com/go-chi/chi/v5"
	"github.com/go-chi/chi/v5/middleware"
)

func main() {
	r := newRouter()
	addr := ":8080"
	log.Printf("invoicekit go-chi demo listening on %s (transport=%s)", addr, invoicekit.TransportMode())
	if err := http.ListenAndServe(addr, r); err != nil {
		log.Fatal(err)
	}
}

func newRouter() *chi.Mux {
	r := chi.NewRouter()
	r.Use(middleware.Logger)
	r.Use(middleware.Recoverer)
	r.Get("/", indexHandler)
	r.Get("/healthz", healthzHandler)
	r.Post("/canonicalize/{fixture}", canonicalizeHandler)
	return r
}

func indexHandler(w http.ResponseWriter, _ *http.Request) {
	names := make([]string, 0, len(fixtures()))
	for name := range fixtures() {
		names = append(names, name)
	}
	sort.Strings(names)
	writeJSON(w, http.StatusOK, map[string]any{
		"title":     "InvoiceKit Go (chi) demo",
		"fixtures":  names,
		"transport": invoicekit.TransportMode(),
		"usage":     "POST /canonicalize/{fixture}",
	})
}

func healthzHandler(w http.ResponseWriter, _ *http.Request) {
	writeJSON(w, http.StatusOK, map[string]string{"status": "ok"})
}

func canonicalizeHandler(w http.ResponseWriter, r *http.Request) {
	name := chi.URLParam(r, "fixture")
	doc, ok := fixtures()[name]
	if !ok {
		available := make([]string, 0, len(fixtures()))
		for n := range fixtures() {
			available = append(available, n)
		}
		sort.Strings(available)
		writeJSON(w, http.StatusNotFound, map[string]any{
			"error": map[string]any{
				"code":      "UNKNOWN_FIXTURE",
				"available": available,
			},
		})
		return
	}
	payload, err := json.Marshal(map[string]any{
		"abi_version": 1,
		"operation":   "commercial_document.canonicalize",
		"payload":     doc,
	})
	if err != nil {
		writeJSON(w, http.StatusInternalServerError, map[string]string{"error": err.Error()})
		return
	}
	response, status, err := invoicekit.Process(payload)
	if err != nil {
		writeJSON(w, http.StatusBadGateway, map[string]any{
			"error": map[string]any{
				"code":    "ENGINE_FAILURE",
				"message": err.Error(),
			},
		})
		return
	}
	var parsed map[string]any
	if err := json.Unmarshal(response, &parsed); err != nil {
		writeJSON(w, http.StatusInternalServerError, map[string]string{"error": err.Error()})
		return
	}
	parsed["_engine_status"] = status
	writeJSON(w, http.StatusOK, parsed)
}

func writeJSON(w http.ResponseWriter, status int, body any) {
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(status)
	if err := json.NewEncoder(w).Encode(body); err != nil {
		log.Printf("writeJSON encode error: %v", err)
		fmt.Fprintln(w, `{"error":"internal"}`)
	}
}
