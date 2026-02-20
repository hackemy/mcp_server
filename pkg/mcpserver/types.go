package mcpserver

import "encoding/json"

// JSON-RPC 2.0 error codes.
const (
	ErrCodeParse      = -32700
	ErrCodeInvalidReq = -32600
	ErrCodeNoMethod   = -32601
	ErrCodeBadParams  = -32602
	ErrCodeInternal   = -32603
)

// Logger abstracts structured logging. Compatible with *slog.Logger.
type Logger interface {
	Debug(msg string, args ...any)
	Info(msg string, args ...any)
	Error(msg string, args ...any)
}

// JSONRPCRequest models inbound MCP traffic.
type JSONRPCRequest struct {
	JSONRPC string          `json:"jsonrpc"`
	ID      any             `json:"id"`
	Method  string          `json:"method"`
	Params  json.RawMessage `json:"params,omitempty"`
}

// JSONRPCResponse is the envelope for outbound MCP replies.
type JSONRPCResponse struct {
	JSONRPC string          `json:"jsonrpc"`
	ID      any             `json:"id"`
	Result  json.RawMessage `json:"result,omitempty"`
	Error   *RPCError       `json:"error,omitempty"`
}

// RPCError aligns with JSON-RPC 2.0 error format.
type RPCError struct {
	Code    int    `json:"code"`
	Message string `json:"message"`
	Data    any    `json:"data,omitempty"`
}

// Tool describes an MCP tool definition loaded from config.
type Tool struct {
	Name        string          `json:"name"`
	Description string          `json:"description"`
	InputSchema json.RawMessage `json:"inputSchema"`
	schemaMeta  schemaMeta
}

// Resource describes an available MCP resource.
type Resource struct {
	Name        string `json:"name"`
	Description string `json:"description"`
	URI         string `json:"uri"`
	MimeType    string `json:"mimeType"`
}

// ToolResult is the MCP-spec tool call result returned by tool handlers.
type ToolResult struct {
	Content []ContentBlock `json:"content"`
	IsError bool           `json:"isError,omitempty"`
}

// ContentBlock represents a single content item in a tool result.
type ContentBlock struct {
	Type string `json:"type"`
	Text string `json:"text,omitempty"`
}

// ResourceContent is the MCP-spec resource content returned by resource handlers.
type ResourceContent struct {
	URI      string `json:"uri"`
	MimeType string `json:"mimeType,omitempty"`
	Text     string `json:"text,omitempty"`
	Blob     string `json:"blob,omitempty"`
}

// TextResult is a convenience constructor for a simple text tool result.
func TextResult(text string) ToolResult {
	return ToolResult{
		Content: []ContentBlock{{Type: "text", Text: text}},
	}
}

// ErrorResult is a convenience constructor for a tool error result.
func ErrorResult(text string) ToolResult {
	return ToolResult{
		Content: []ContentBlock{{Type: "text", Text: text}},
		IsError: true,
	}
}

// NewErrorResponse builds a JSON-RPC error envelope.
func NewErrorResponse(id any, code int, message string) JSONRPCResponse {
	return JSONRPCResponse{
		JSONRPC: "2.0",
		ID:      id,
		Error: &RPCError{
			Code:    code,
			Message: message,
		},
	}
}

// IsNotification returns true when the response signals a notification
// that requires no response body (HTTP 202).
func (r JSONRPCResponse) IsNotification() bool {
	return r.ID == nil && r.Result == nil && r.Error == nil
}

// Internal schema metadata types used for validation.

type schemaMeta struct {
	Required     []string              `json:"required"`
	OneOf        []schemaRequirementSet `json:"oneOf"`
	Dependencies map[string][]string   `json:"dependencies"`
}

type schemaRequirementSet struct {
	Required []string `json:"required"`
}

type toolCallParams struct {
	Name      string         `json:"name"`
	Arguments map[string]any `json:"arguments"`
}

type resourceReadParams struct {
	Name string `json:"name"`
	URI  string `json:"uri"`
}

type initializeParams struct {
	ProtocolVersion string `json:"protocolVersion"`
	Capabilities    any    `json:"capabilities"`
	ClientInfo      struct {
		Name    string `json:"name"`
		Version string `json:"version"`
	} `json:"clientInfo"`
}
