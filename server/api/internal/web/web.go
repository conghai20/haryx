// Package web holds the API's response shapes: JSON bodies, and errors as {"error": "<code>"}.
package web

import (
	"encoding/json"
	"log/slog"
	"net/http"
)

// JSON writes v with status.
func JSON(w http.ResponseWriter, status int, v any) {
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(status)
	if err := json.NewEncoder(w).Encode(v); err != nil {
		slog.Warn("write response", "err", err)
	}
}

// Error writes {"error": code}. Codes are stable and part of the API contract (consent_required…).
func Error(w http.ResponseWriter, status int, code string) {
	JSON(w, status, map[string]string{"error": code})
}
