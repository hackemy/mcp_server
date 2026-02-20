package tools

import (
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"testing"

	"mcpserver"
)

type testLogger struct{}

func (testLogger) Debug(string, ...any) {}
func (testLogger) Info(string, ...any)  {}
func (testLogger) Error(string, ...any) {}

func TestRegisterAll(t *testing.T) {
	dir := t.TempDir()
	// Build a minimal tools.json with all 14 tools so the server knows about them.
	var toolDefs []map[string]any
	for _, name := range ToolNames {
		toolDefs = append(toolDefs, map[string]any{
			"name":        name,
			"description": "test",
			"inputSchema": map[string]any{"type": "object", "properties": map[string]any{}},
		})
	}
	data, _ := json.Marshal(toolDefs)
	os.WriteFile(filepath.Join(dir, "tools.json"), data, 0644)
	os.WriteFile(filepath.Join(dir, "resources.json"), []byte("[]"), 0644)

	srv := mcpserver.New(
		mcpserver.WithToolsFile(filepath.Join(dir, "tools.json")),
		mcpserver.WithResourcesFile(filepath.Join(dir, "resources.json")),
		mcpserver.WithLogger(testLogger{}),
	)
	RegisterAll(srv)

	// Verify each tool executes via the server Handle.
	for _, name := range ToolNames {
		params, _ := json.Marshal(map[string]any{
			"name":      name,
			"arguments": map[string]any{},
		})
		resp := srv.Handle(context.Background(), mcpserver.JSONRPCRequest{
			JSONRPC: "2.0", ID: 1, Method: "tools/call", Params: params,
		})
		if resp.Error != nil {
			t.Errorf("tool %q: unexpected error: %+v", name, resp.Error)
			continue
		}

		var result mcpserver.ToolResult
		json.Unmarshal(resp.Result, &result)
		expected := "stub: " + name + " accepted"
		if len(result.Content) == 0 || result.Content[0].Text != expected {
			t.Errorf("tool %q: expected %q, got %+v", name, expected, result)
		}
	}
}

func TestToolNames_Count(t *testing.T) {
	if len(ToolNames) != 14 {
		t.Errorf("expected 14 tool names, got %d", len(ToolNames))
	}
}
