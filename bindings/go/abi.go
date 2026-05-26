// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

package invoicekit

/*
#cgo CFLAGS: -I../../crates/invoicekit-ffi/include
#include "invoicekit.h"
*/
import "C"

import (
	"errors"
	"math"
	"unsafe"
)

func processEngineABI(request []byte) ([]byte, uint32, error) {
	var requestPtr *C.uchar
	if len(request) > 0 {
		requestPtr = (*C.uchar)(unsafe.Pointer(&request[0]))
	}
	result := C.invoicekit_engine_process_json(requestPtr, C.size_t(len(request)))
	if result == nil {
		return nil, 0, errors.New("invoicekit_engine_process_json returned nil")
	}
	defer C.invoicekit_engine_result_free(result)

	status := uint32(C.invoicekit_engine_result_status(result))
	responseLen := C.invoicekit_engine_result_len(result)
	if uint64(responseLen) > math.MaxInt32 {
		return nil, status, errors.New("response too large for C.GoBytes")
	}
	responsePtr := C.invoicekit_engine_result_bytes(result)
	if responsePtr == nil {
		return nil, status, errors.New("invoicekit_engine_result_bytes returned nil")
	}
	actual := C.GoBytes(unsafe.Pointer(responsePtr), C.int(responseLen))
	return actual, status, nil
}
