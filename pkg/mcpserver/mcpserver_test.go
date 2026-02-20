package mcpserver

import (
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"testing"
)

// testLogger discards log output.
type testLogger struct{}

func (testLogger) Debug(string, ...any) {}
func (testLogger) Info(string, ...any)  {}
func (testLogger) Error(string, ...any) {}

func setupTestServer(t *testing.T) *Server {
	t.Helper()
	dir := t.TempDir()

	tools := `[
		{"name":"echo","description":"echoes input","inputSchema":{"type":"object","properties":{"msg":{"type":"string"}},"required":["msg"]}},
		{"name":"optional","description":"no required fields","inputSchema":{"type":"object","properties":{"x":{"type":"string"}}}}
	]`
	resources := `[
		{"name":"test-data","description":"test resource","uri":"file:///test.csv","mimeType":"text/csv"}
	]`

	os.WriteFile(filepath.Join(dir, "tools.json"), []byte(tools), 0644)
	os.WriteFile(filepath.Join(dir, "resources.json"), []byte(resources), 0644)

	srv := New(
		WithToolsFile(filepath.Join(dir, "tools.json")),
		WithResourcesFile(filepath.Join(dir, "resources.json")),
		WithServerInfo("test-server", "0.1.0"),
		WithLogger(testLogger{}),
	)

	srv.HandleTool("echo", func(_ context.Context, args map[string]any) (ToolResult, error) {
		return TextResult("echoed: " + args["msg"].(string)), nil
	})
	srv.HandleTool("optional", func(_ context.Context, _ map[string]any) (ToolResult, error) {
		return TextResult("ok"), nil
	})
	srv.HandleResource("test-data", func(_ context.Context, uri string) (ResourceContent, error) {
		return ResourceContent{URI: uri, MimeType: "text/csv", Text: "a,b,c"}, nil
	})

	return srv
}

func TestNew_WithToolsJSON(t *testing.T) {
	toolsJSON := `[{"name":"t1","description":"d","inputSchema":{"type":"object","properties":{}}}]`
	srv := New(WithToolsJSON([]byte(toolsJSON)), WithLogger(testLogger{}))
	resp := srv.Handle(context.Background(), JSONRPCRequest{
		JSONRPC: "2.0", ID: 1, Method: "tools/list",
	})
	var result struct{ Tools []Tool }
	json.Unmarshal(resp.Result, &result)
	if len(result.Tools) != 1 || result.Tools[0].Name != "t1" {
		t.Errorf("unexpected tools: %+v", result.Tools)
	}
}

func TestNew_WithResourcesJSON(t *testing.T) {
	resJSON := `[{"name":"r1","description":"d","uri":"s3://b/k","mimeType":"text/csv"}]`
	srv := New(WithResourcesJSON([]byte(resJSON)), WithLogger(testLogger{}))
	resp := srv.Handle(context.Background(), JSONRPCRequest{
		JSONRPC: "2.0", ID: 1, Method: "resources/list",
	})
	var result struct{ Resources []Resource }
	json.Unmarshal(resp.Result, &result)
	if len(result.Resources) != 1 || result.Resources[0].Name != "r1" {
		t.Errorf("unexpected resources: %+v", result.Resources)
	}
}

func TestHandle_BadJSONRPCVersion(t *testing.T) {
	srv := setupTestServer(t)
	resp := srv.Handle(context.Background(), JSONRPCRequest{
		JSONRPC: "1.0", ID: 1, Method: "ping",
	})
	if resp.Error == nil || resp.Error.Code != ErrCodeInvalidReq {
		t.Fatalf("expected invalid request error, got %+v", resp)
	}
}

func TestHandle_UnknownMethod(t *testing.T) {
	srv := setupTestServer(t)
	resp := srv.Handle(context.Background(), JSONRPCRequest{
		JSONRPC: "2.0", ID: 1, Method: "foo/bar",
	})
	if resp.Error == nil || resp.Error.Code != ErrCodeNoMethod {
		t.Fatalf("expected method not found, got %+v", resp)
	}
}

func TestHandle_Initialize(t *testing.T) {
	srv := setupTestServer(t)
	params, _ := json.Marshal(map[string]any{
		"protocolVersion": "2025-03-26",
		"capabilities":    map[string]any{},
		"clientInfo":      map[string]any{"name": "test", "version": "0.1"},
	})
	resp := srv.Handle(context.Background(), JSONRPCRequest{
		JSONRPC: "2.0", ID: 1, Method: "initialize", Params: params,
	})
	if resp.Error != nil {
		t.Fatalf("unexpected error: %+v", resp.Error)
	}

	var result map[string]any
	json.Unmarshal(resp.Result, &result)
	if result["protocolVersion"] != ProtocolVersion {
		t.Errorf("expected protocol %s, got %v", ProtocolVersion, result["protocolVersion"])
	}
	info := result["serverInfo"].(map[string]any)
	if info["name"] != "test-server" {
		t.Errorf("expected name test-server, got %v", info["name"])
	}
	if info["version"] != "0.1.0" {
		t.Errorf("expected version 0.1.0, got %v", info["version"])
	}
}

func TestHandle_Ping(t *testing.T) {
	srv := setupTestServer(t)
	resp := srv.Handle(context.Background(), JSONRPCRequest{
		JSONRPC: "2.0", ID: 1, Method: "ping",
	})
	if resp.Error != nil {
		t.Fatalf("unexpected error: %+v", resp.Error)
	}
	if string(resp.Result) != "{}" {
		t.Errorf("expected {}, got %s", resp.Result)
	}
}

func TestHandle_NotificationsReturnSentinel(t *testing.T) {
	srv := setupTestServer(t)
	for _, method := range []string{"notifications/initialized", "notifications/cancelled"} {
		resp := srv.Handle(context.Background(), JSONRPCRequest{
			JSONRPC: "2.0", Method: method,
		})
		if !resp.IsNotification() {
			t.Errorf("expected notification sentinel for %s", method)
		}
	}
}

func TestHandle_ToolsList(t *testing.T) {
	srv := setupTestServer(t)
	resp := srv.Handle(context.Background(), JSONRPCRequest{
		JSONRPC: "2.0", ID: 1, Method: "tools/list",
	})
	if resp.Error != nil {
		t.Fatalf("unexpected error: %+v", resp.Error)
	}
	var result struct{ Tools []Tool }
	json.Unmarshal(resp.Result, &result)
	if len(result.Tools) != 2 {
		t.Errorf("expected 2 tools, got %d", len(result.Tools))
	}
}

func TestHandle_ToolsCall_Success(t *testing.T) {
	srv := setupTestServer(t)
	params, _ := json.Marshal(toolCallParams{
		Name:      "echo",
		Arguments: map[string]any{"msg": "hello"},
	})
	resp := srv.Handle(context.Background(), JSONRPCRequest{
		JSONRPC: "2.0", ID: 1, Method: "tools/call", Params: params,
	})
	if resp.Error != nil {
		t.Fatalf("unexpected error: %+v", resp.Error)
	}

	var result ToolResult
	json.Unmarshal(resp.Result, &result)
	if len(result.Content) == 0 || result.Content[0].Text != "echoed: hello" {
		t.Errorf("unexpected result: %+v", result)
	}
}

func TestHandle_ToolsCall_MissingRequired(t *testing.T) {
	srv := setupTestServer(t)
	params, _ := json.Marshal(toolCallParams{
		Name:      "echo",
		Arguments: map[string]any{},
	})
	resp := srv.Handle(context.Background(), JSONRPCRequest{
		JSONRPC: "2.0", ID: 1, Method: "tools/call", Params: params,
	})
	if resp.Error == nil || resp.Error.Code != ErrCodeBadParams {
		t.Fatalf("expected bad params, got %+v", resp)
	}
}

func TestHandle_ToolsCall_UnknownTool(t *testing.T) {
	srv := setupTestServer(t)
	params, _ := json.Marshal(toolCallParams{Name: "nonexistent"})
	resp := srv.Handle(context.Background(), JSONRPCRequest{
		JSONRPC: "2.0", ID: 1, Method: "tools/call", Params: params,
	})
	if resp.Error == nil || resp.Error.Code != ErrCodeNoMethod {
		t.Fatalf("expected method not found, got %+v", resp)
	}
}

func TestHandle_ToolsCall_NoHandler(t *testing.T) {
	srv := New(
		WithToolsJSON([]byte(`[{"name":"t","description":"d","inputSchema":{"type":"object","properties":{}}}]`)),
		WithLogger(testLogger{}),
	)
	params, _ := json.Marshal(toolCallParams{Name: "t"})
	resp := srv.Handle(context.Background(), JSONRPCRequest{
		JSONRPC: "2.0", ID: 1, Method: "tools/call", Params: params,
	})
	if resp.Error == nil || resp.Error.Code != ErrCodeInternal {
		t.Fatalf("expected internal error for no handler, got %+v", resp)
	}
}

func TestHandle_ResourcesList(t *testing.T) {
	srv := setupTestServer(t)
	resp := srv.Handle(context.Background(), JSONRPCRequest{
		JSONRPC: "2.0", ID: 1, Method: "resources/list",
	})
	if resp.Error != nil {
		t.Fatalf("unexpected error: %+v", resp.Error)
	}
	var result struct{ Resources []Resource }
	json.Unmarshal(resp.Result, &result)
	if len(result.Resources) != 1 {
		t.Errorf("expected 1 resource, got %d", len(result.Resources))
	}
}

func TestHandle_ResourcesRead_ByName(t *testing.T) {
	srv := setupTestServer(t)
	params, _ := json.Marshal(resourceReadParams{Name: "test-data"})
	resp := srv.Handle(context.Background(), JSONRPCRequest{
		JSONRPC: "2.0", ID: 1, Method: "resources/read", Params: params,
	})
	if resp.Error != nil {
		t.Fatalf("unexpected error: %+v", resp.Error)
	}
	var result struct{ Contents []ResourceContent }
	json.Unmarshal(resp.Result, &result)
	if len(result.Contents) == 0 || result.Contents[0].Text != "a,b,c" {
		t.Errorf("unexpected content: %+v", result)
	}
}

func TestHandle_ResourcesRead_ByURI(t *testing.T) {
	srv := setupTestServer(t)
	params, _ := json.Marshal(resourceReadParams{URI: "file:///test.csv"})
	resp := srv.Handle(context.Background(), JSONRPCRequest{
		JSONRPC: "2.0", ID: 1, Method: "resources/read", Params: params,
	})
	if resp.Error != nil {
		t.Fatalf("unexpected error: %+v", resp.Error)
	}
}

func TestHandle_ResourcesRead_NotFound(t *testing.T) {
	srv := setupTestServer(t)
	params, _ := json.Marshal(resourceReadParams{Name: "nope"})
	resp := srv.Handle(context.Background(), JSONRPCRequest{
		JSONRPC: "2.0", ID: 1, Method: "resources/read", Params: params,
	})
	if resp.Error == nil {
		t.Fatal("expected error for missing resource")
	}
}

func TestHandle_ResourcesRead_NoHandler_FallbackMetadata(t *testing.T) {
	srv := New(
		WithResourcesJSON([]byte(`[{"name":"r","description":"d","uri":"s3://b/k","mimeType":"text/csv"}]`)),
		WithLogger(testLogger{}),
	)
	params, _ := json.Marshal(resourceReadParams{Name: "r"})
	resp := srv.Handle(context.Background(), JSONRPCRequest{
		JSONRPC: "2.0", ID: 1, Method: "resources/read", Params: params,
	})
	if resp.Error != nil {
		t.Fatalf("unexpected error: %+v", resp.Error)
	}
	// Should return metadata fallback with empty text.
	var result struct {
		Contents []map[string]any `json:"contents"`
	}
	json.Unmarshal(resp.Result, &result)
	if len(result.Contents) == 0 {
		t.Fatal("expected fallback content")
	}
	if result.Contents[0]["uri"] != "s3://b/k" {
		t.Errorf("unexpected uri: %v", result.Contents[0]["uri"])
	}
}

func TestHandle_ResourcesRead_MissingParams(t *testing.T) {
	srv := setupTestServer(t)
	resp := srv.Handle(context.Background(), JSONRPCRequest{
		JSONRPC: "2.0", ID: 1, Method: "resources/read",
	})
	if resp.Error == nil || resp.Error.Code != ErrCodeBadParams {
		t.Fatalf("expected bad params, got %+v", resp)
	}
}

func TestTextResult(t *testing.T) {
	r := TextResult("hello")
	if len(r.Content) != 1 || r.Content[0].Type != "text" || r.Content[0].Text != "hello" {
		t.Errorf("unexpected: %+v", r)
	}
	if r.IsError {
		t.Error("TextResult should not be an error")
	}
}

func TestErrorResult(t *testing.T) {
	r := ErrorResult("fail")
	if !r.IsError || r.Content[0].Text != "fail" {
		t.Errorf("unexpected: %+v", r)
	}
}

// --- Loader tests ---

func TestLoadTools_Valid(t *testing.T) {
	dir := t.TempDir()
	data := `[{"name":"t1","description":"d","inputSchema":{"type":"object","properties":{},"required":["x"]}}]`
	os.WriteFile(filepath.Join(dir, "tools.json"), []byte(data), 0644)

	tools, err := LoadTools(filepath.Join(dir, "tools.json"))
	if err != nil {
		t.Fatal(err)
	}
	if len(tools) != 1 || tools[0].Name != "t1" {
		t.Errorf("unexpected: %+v", tools)
	}
}

func TestLoadTools_MissingFile(t *testing.T) {
	_, err := LoadTools("/nonexistent/tools.json")
	if err == nil {
		t.Fatal("expected error")
	}
}

func TestLoadTools_MalformedJSON(t *testing.T) {
	dir := t.TempDir()
	os.WriteFile(filepath.Join(dir, "tools.json"), []byte("{bad"), 0644)
	_, err := LoadTools(filepath.Join(dir, "tools.json"))
	if err == nil {
		t.Fatal("expected error")
	}
}

func TestParseTools(t *testing.T) {
	data := `[{"name":"t","description":"d","inputSchema":{"type":"object","properties":{}}}]`
	tools, err := ParseTools([]byte(data))
	if err != nil {
		t.Fatal(err)
	}
	if len(tools) != 1 {
		t.Errorf("expected 1 tool, got %d", len(tools))
	}
}

func TestParseResources(t *testing.T) {
	data := `[{"name":"r","description":"d","uri":"s3://b/k","mimeType":"text/csv"}]`
	res, err := ParseResources([]byte(data))
	if err != nil {
		t.Fatal(err)
	}
	if len(res) != 1 {
		t.Errorf("expected 1 resource, got %d", len(res))
	}
}

// --- Validate tests ---

func makeTool(schema string) Tool {
	t := Tool{Name: "test", InputSchema: json.RawMessage(schema)}
	var meta schemaMeta
	json.Unmarshal(t.InputSchema, &meta)
	t.schemaMeta = meta
	return t
}

func TestValidate_RequiredPresent(t *testing.T) {
	tool := makeTool(`{"type":"object","required":["token"]}`)
	if err := tool.ValidateArguments(map[string]any{"token": "abc"}); err != nil {
		t.Errorf("unexpected error: %v", err)
	}
}

func TestValidate_RequiredMissing(t *testing.T) {
	tool := makeTool(`{"type":"object","required":["token"]}`)
	if err := tool.ValidateArguments(map[string]any{}); err == nil {
		t.Error("expected error")
	}
}

func TestValidate_OneOf_Match(t *testing.T) {
	tool := makeTool(`{"type":"object","oneOf":[{"required":["phone"]},{"required":["email"]}]}`)
	if err := tool.ValidateArguments(map[string]any{"phone": "+1"}); err != nil {
		t.Errorf("unexpected error: %v", err)
	}
	if err := tool.ValidateArguments(map[string]any{"email": "a@b"}); err != nil {
		t.Errorf("unexpected error: %v", err)
	}
}

func TestValidate_OneOf_NoneMatch(t *testing.T) {
	tool := makeTool(`{"type":"object","oneOf":[{"required":["phone"]},{"required":["email"]}]}`)
	if err := tool.ValidateArguments(map[string]any{}); err == nil {
		t.Error("expected error")
	}
}

func TestValidate_Dependencies_Satisfied(t *testing.T) {
	tool := makeTool(`{"type":"object","dependencies":{"geo_lat":["geo_lon"],"geo_lon":["geo_lat"]}}`)
	if err := tool.ValidateArguments(map[string]any{"geo_lat": 40.0, "geo_lon": -74.0}); err != nil {
		t.Errorf("unexpected error: %v", err)
	}
}

func TestValidate_Dependencies_Missing(t *testing.T) {
	tool := makeTool(`{"type":"object","dependencies":{"geo_lat":["geo_lon"]}}`)
	if err := tool.ValidateArguments(map[string]any{"geo_lat": 40.0}); err == nil {
		t.Error("expected error")
	}
}

func TestValidate_CombinedRequiredAndOneOf(t *testing.T) {
	tool := makeTool(`{
		"type":"object",
		"required":["code"],
		"oneOf":[{"required":["phone","code"]},{"required":["email","code"]}]
	}`)
	if err := tool.ValidateArguments(map[string]any{"code": "123", "phone": "+1"}); err != nil {
		t.Errorf("unexpected error: %v", err)
	}
	if err := tool.ValidateArguments(map[string]any{"phone": "+1"}); err == nil {
		t.Error("expected error for missing code")
	}
	if err := tool.ValidateArguments(map[string]any{"code": "123"}); err == nil {
		t.Error("expected error for missing oneOf")
	}
}
