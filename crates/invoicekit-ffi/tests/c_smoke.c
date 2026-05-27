// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

#include "invoicekit.h"

#include <stdint.h>
#include <stdio.h>
#include <string.h>

static int check(int condition, const char *message) {
  if (condition) {
    return 0;
  }
  fprintf(stderr, "invoicekit C ABI smoke failed: %s\n", message);
  return 1;
}

static int contains_bytes(const unsigned char *haystack,
                          size_t haystack_len,
                          const char *needle) {
  const size_t needle_len = strlen(needle);
  if (needle_len == 0 || haystack_len < needle_len) {
    return 0;
  }

  for (size_t offset = 0; offset <= haystack_len - needle_len; ++offset) {
    if (memcmp(haystack + offset, needle, needle_len) == 0) {
      return 1;
    }
  }
  return 0;
}

int main(void) {
  int failures = 0;

  failures += check(invoicekit_engine_abi_version() == 1U,
                    "invoicekit_engine_abi_version returns v1");
  failures += check(invoicekit_engine_result_status(NULL) == (uint32_t)InvalidHandle,
                    "null result status reports InvalidHandle");
  failures += check(invoicekit_engine_result_bytes(NULL) == NULL,
                    "null result bytes pointer is null");
  failures += check(invoicekit_engine_result_len(NULL) == 0U,
                    "null result length is zero");
  invoicekit_engine_result_free(NULL);

  const unsigned char request[] =
      "{\"abi_version\":1,\"operation\":\"unknown\",\"payload\":{}}";
  InvoiceKitEngineResult *result =
      invoicekit_engine_process_json(request, sizeof(request) - 1U);
  failures += check(result != NULL, "process_json returns a result handle");

  if (result != NULL) {
    const uint32_t status = invoicekit_engine_result_status(result);
    const size_t len = invoicekit_engine_result_len(result);
    const unsigned char *bytes = invoicekit_engine_result_bytes(result);

    failures += check(status == (uint32_t)Error,
                      "unknown operation returns Error status");
    failures += check(len > 0U, "result length is nonzero");
    failures += check(bytes != NULL, "result bytes pointer is non-null");
    if (bytes != NULL) {
      failures += check(contains_bytes(bytes, len, "\"status\":\"error\""),
                        "result bytes contain canonical error status");
    }

    invoicekit_engine_result_free(result);
  }

  InvoiceKitEngineResult *null_input_result =
      invoicekit_engine_process_json(NULL, 1U);
  failures += check(null_input_result != NULL,
                    "null nonzero request returns an error handle");
  if (null_input_result != NULL) {
    failures += check(invoicekit_engine_result_status(null_input_result) ==
                          (uint32_t)Error,
                      "null nonzero request reports Error status");
    failures += check(invoicekit_engine_result_len(null_input_result) > 0U,
                      "null nonzero request has error bytes");
    invoicekit_engine_result_free(null_input_result);
  }

  return failures == 0 ? 0 : 1;
}
