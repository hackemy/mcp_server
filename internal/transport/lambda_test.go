package transport

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"net/http"
	"os"
	"path/filepath"
	"testing"

	"github.com/aws/aws-lambda-go/events"

	"mcpserver"
)

type testLogger struct{}

func (testLogger) Debug(string, ...any) {}
func (testLogger) Info(string, ...any)  {}
func (testLogger) Error(string, ...any) {}

func setupLambdaAdapter(t *testing.T) *LambdaAdapter {
	t.Helper()
	dir := t.TempDir()

	tools := `[{"name":"echo","description":"echo","inputSchema":{"type":"object","properties":{"msg":{"type":"string"}},"required":["msg"]}}]`
	resources := `[{"name":"test","description":"test","uri":"file:///test.csv","mimeType":"text/csv"}]`

	os.WriteFile(filepath.Join(dir, "tools.json"), []byte(tools), 0644)
	os.WriteFile(filepath.Join(dir, "resources.json"), []byte(resources), 0644)

	srv := mcpserver.New(
		mcpserver.WithToolsFile(filepath.Join(dir, "tools.json")),
		mcpserver.WithResourcesFile(filepath.Join(dir, "resources.json")),
		mcpserver.WithLogger(testLogger{}),
	)
	srv.HandleTool("echo", func(_ context.Context, args map[string]any) (mcpserver.ToolResult, error) {
		return mcpserver.TextResult("echoed"), nil
	})

	return NewLambdaAdapter(srv)
}

func makeAPIGWRequest(method, path, body string, base64Encoded bool) events.APIGatewayV2HTTPRequest {
	req := events.APIGatewayV2HTTPRequest{
		RawPath:         path,
		Body:            body,
		IsBase64Encoded: base64Encoded,
	}
	req.RequestContext.HTTP.Method = method
	return req
}

func TestLambda_HealthCheck(t *testing.T) {
	adapter := setupLambdaAdapter(t)
	resp, err := adapter.Handle(context.Background(), makeAPIGWRequest(http.MethodGet, "/healthz", "", false))
	if err != nil {
		t.Fatal(err)
	}
	if resp.StatusCode != http.StatusOK {
		t.Errorf("expected 200, got %d", resp.StatusCode)
	}
}

func TestLambda_RouteNotFound(t *testing.T) {
	adapter := setupLambdaAdapter(t)
	resp, err := adapter.Handle(context.Background(), makeAPIGWRequest(http.MethodGet, "/unknown", "", false))
	if err != nil {
		t.Fatal(err)
	}
	if resp.StatusCode != http.StatusNotFound {
		t.Errorf("expected 404, got %d", resp.StatusCode)
	}
}

func TestLambda_Initialize(t *testing.T) {
	adapter := setupLambdaAdapter(t)
	body, _ := json.Marshal(map[string]any{
		"jsonrpc": "2.0", "id": 1, "method": "initialize",
		"params": map[string]any{
			"protocolVersion": "2025-03-26",
			"capabilities":    map[string]any{},
			"clientInfo":      map[string]any{"name": "test", "version": "0.1"},
		},
	})
	resp, err := adapter.Handle(context.Background(), makeAPIGWRequest(http.MethodPost, "/mcp", string(body), false))
	if err != nil {
		t.Fatal(err)
	}
	if resp.StatusCode != http.StatusOK {
		t.Errorf("expected 200, got %d", resp.StatusCode)
	}
}

func TestLambda_Base64Encoded(t *testing.T) {
	adapter := setupLambdaAdapter(t)
	body, _ := json.Marshal(map[string]any{"jsonrpc": "2.0", "id": 1, "method": "tools/list"})
	encoded := base64.StdEncoding.EncodeToString(body)
	resp, err := adapter.Handle(context.Background(), makeAPIGWRequest(http.MethodPost, "/mcp", encoded, true))
	if err != nil {
		t.Fatal(err)
	}
	if resp.StatusCode != http.StatusOK {
		t.Errorf("expected 200, got %d", resp.StatusCode)
	}
}

func TestLambda_InvalidJSON(t *testing.T) {
	adapter := setupLambdaAdapter(t)
	resp, err := adapter.Handle(context.Background(), makeAPIGWRequest(http.MethodPost, "/mcp", "{bad", false))
	if err != nil {
		t.Fatal(err)
	}
	if resp.StatusCode != http.StatusBadRequest {
		t.Errorf("expected 400, got %d", resp.StatusCode)
	}
}

func TestLambda_Notification(t *testing.T) {
	adapter := setupLambdaAdapter(t)
	body, _ := json.Marshal(map[string]any{"jsonrpc": "2.0", "method": "notifications/initialized"})
	resp, err := adapter.Handle(context.Background(), makeAPIGWRequest(http.MethodPost, "/mcp", string(body), false))
	if err != nil {
		t.Fatal(err)
	}
	if resp.StatusCode != http.StatusAccepted {
		t.Errorf("expected 202, got %d", resp.StatusCode)
	}
}

func TestLambda_ToolsList(t *testing.T) {
	adapter := setupLambdaAdapter(t)
	body, _ := json.Marshal(map[string]any{"jsonrpc": "2.0", "id": 1, "method": "tools/list"})
	resp, err := adapter.Handle(context.Background(), makeAPIGWRequest(http.MethodPost, "/mcp", string(body), false))
	if err != nil {
		t.Fatal(err)
	}
	if resp.StatusCode != http.StatusOK {
		t.Errorf("expected 200, got %d", resp.StatusCode)
	}
}
