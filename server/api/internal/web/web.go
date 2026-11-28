// Package web holds the API's response shapes: JSON bodies, and errors as {"error": "<code>"}.
package web

import (
	"encoding/json"
	"log/slog"
	"net/http"
)

// JSON writes v with status. Nothing the API answers may be cached: exports, rows, machines.
func JSON(w http.ResponseWriter, status int, v any) {
	w.Header().Set("Content-Type", "application/json")
	w.Header().Set("Cache-Control", "no-store")
	w.WriteHeader(status)
	if err := json.NewEncoder(w).Encode(v); err != nil {
		slog.Warn("write response", "err", err)
	}
}

// Error writes {"error": code}. Codes are stable and part of the API contract (consent_required…).
func Error(w http.ResponseWriter, status int, code string) {
	JSON(w, status, map[string]string{"error": code})
}

// InternalError logs err with what failed and answers 500 internal: the detail stays in the log.
func InternalError(w http.ResponseWriter, what string, err error) {
	slog.Error(what, "err", err)
	Error(w, http.StatusInternalServerError, "internal")
}
