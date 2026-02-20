package mcpserver

import (
	"bytes"
	"context"
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"testing"
)

func setupHTTPHandler(t *testing.T) http.Handler {
	t.Helper()
	dir := t.TempDir()

	tools := `[{"name":"echo","description":"echo","inputSchema":{"type":"object","properties":{"msg":{"type":"string"}},"required":["msg"]}}]`
	resources := `[{"name":"test","description":"test","uri":"file:///test.csv","mimeType":"text/csv"}]`

	os.WriteFile(filepath.Join(dir, "tools.json"), []byte(tools), 0644)
	os.WriteFile(filepath.Join(dir, "resources.json"), []byte(resources), 0644)

	srv := New(
		WithToolsFile(filepath.Join(dir, "tools.json")),
		WithResourcesFile(filepath.Join(dir, "resources.json")),
		WithLogger(testLogger{}),
	)
	srv.HandleTool("echo", func(_ context.Context, args map[string]any) (ToolResult, error) {
		return TextResult("echoed"), nil
	})

	return srv.HTTPHandler()
}

func rpcBody(method string, id any, params any) []byte {
	req := map[string]any{"jsonrpc": "2.0", "method": method}
	if id != nil {
		req["id"] = id
	}
	if params != nil {
		req["params"] = params
	}
	b, _ := json.Marshal(req)
	return b
}

func TestHTTP_HealthCheck(t *testing.T) {
	handler := setupHTTPHandler(t)
	req := httptest.NewRequest(http.MethodGet, "/healthz", nil)
	w := httptest.NewRecorder()
	handler.ServeHTTP(w, req)

	if w.Code != http.StatusOK {
		t.Errorf("expected 200, got %d", w.Code)
	}
	body, _ := io.ReadAll(w.Body)
	if string(body) != `{"status":"ok"}` {
		t.Errorf("unexpected body: %s", body)
	}
}

func TestHTTP_Initialize_ReturnsSessionID(t *testing.T) {
	handler := setupHTTPHandler(t)
	body := rpcBody("initialize", 1, map[string]any{
		"protocolVersion": "2025-03-26",
		"capabilities":    map[string]any{},
		"clientInfo":      map[string]any{"name": "test", "version": "0.1"},
	})
	req := httptest.NewRequest(http.MethodPost, "/mcp", bytes.NewReader(body))
	req.Header.Set("Content-Type", "application/json")
	w := httptest.NewRecorder()
	handler.ServeHTTP(w, req)

	if w.Code != http.StatusOK {
		t.Errorf("expected 200, got %d", w.Code)
	}
	if sid := w.Header().Get("Mcp-Session-Id"); sid == "" {
		t.Error("expected Mcp-Session-Id header")
	}
}

func TestHTTP_Notification_Returns202(t *testing.T) {
	handler := setupHTTPHandler(t)
	body := rpcBody("notifications/initialized", nil, nil)
	req := httptest.NewRequest(http.MethodPost, "/mcp", bytes.NewReader(body))
	req.Header.Set("Content-Type", "application/json")
	w := httptest.NewRecorder()
	handler.ServeHTTP(w, req)

	if w.Code != http.StatusAccepted {
		t.Errorf("expected 202, got %d", w.Code)
	}
}

func TestHTTP_ToolsList(t *testing.T) {
	handler := setupHTTPHandler(t)
	body := rpcBody("tools/list", 1, nil)
	req := httptest.NewRequest(http.MethodPost, "/mcp", bytes.NewReader(body))
	req.Header.Set("Content-Type", "application/json")
	w := httptest.NewRecorder()
	handler.ServeHTTP(w, req)

	if w.Code != http.StatusOK {
		t.Errorf("expected 200, got %d", w.Code)
	}
}

func TestHTTP_ToolsCall(t *testing.T) {
	handler := setupHTTPHandler(t)
	body := rpcBody("tools/call", 2, map[string]any{
		"name": "echo", "arguments": map[string]any{"msg": "hi"},
	})
	req := httptest.NewRequest(http.MethodPost, "/mcp", bytes.NewReader(body))
	req.Header.Set("Content-Type", "application/json")
	w := httptest.NewRecorder()
	handler.ServeHTTP(w, req)

	if w.Code != http.StatusOK {
		t.Errorf("expected 200, got %d", w.Code)
	}
}

func TestHTTP_InvalidJSON(t *testing.T) {
	handler := setupHTTPHandler(t)
	req := httptest.NewRequest(http.MethodPost, "/mcp", bytes.NewReader([]byte("{bad")))
	req.Header.Set("Content-Type", "application/json")
	w := httptest.NewRecorder()
	handler.ServeHTTP(w, req)

	if w.Code != http.StatusBadRequest {
		t.Errorf("expected 400, got %d", w.Code)
	}
}

func TestHTTP_SessionFlow(t *testing.T) {
	handler := setupHTTPHandler(t)

	// Initialize → get session.
	initBody := rpcBody("initialize", 1, map[string]any{
		"protocolVersion": "2025-03-26",
		"capabilities":    map[string]any{},
		"clientInfo":      map[string]any{"name": "test", "version": "0.1"},
	})
	req := httptest.NewRequest(http.MethodPost, "/mcp", bytes.NewReader(initBody))
	req.Header.Set("Content-Type", "application/json")
	w := httptest.NewRecorder()
	handler.ServeHTTP(w, req)

	sessionID := w.Header().Get("Mcp-Session-Id")
	if sessionID == "" {
		t.Fatal("no session ID")
	}

	// Valid session.
	listBody := rpcBody("tools/list", 2, nil)
	req = httptest.NewRequest(http.MethodPost, "/mcp", bytes.NewReader(listBody))
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("Mcp-Session-Id", sessionID)
	w = httptest.NewRecorder()
	handler.ServeHTTP(w, req)
	if w.Code != http.StatusOK {
		t.Errorf("expected 200, got %d", w.Code)
	}

	// Invalid session.
	req = httptest.NewRequest(http.MethodPost, "/mcp", bytes.NewReader(listBody))
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("Mcp-Session-Id", "invalid")
	w = httptest.NewRecorder()
	handler.ServeHTTP(w, req)
	if w.Code != http.StatusNotFound {
		t.Errorf("expected 404, got %d", w.Code)
	}
}

func TestHTTP_MethodNotAllowed(t *testing.T) {
	handler := setupHTTPHandler(t)
	req := httptest.NewRequest(http.MethodGet, "/mcp", nil)
	w := httptest.NewRecorder()
	handler.ServeHTTP(w, req)
	if w.Code != http.StatusMethodNotAllowed {
		t.Errorf("expected 405, got %d", w.Code)
	}
}
