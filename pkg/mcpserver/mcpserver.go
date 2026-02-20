// Package mcpserver provides a reusable MCP (Model Context Protocol) server
// compliant with the 2025-03-26 specification. Pass in tools and resources
// config, register handlers, and run over HTTP or AWS Lambda.
package mcpserver

import (
	"context"
	"encoding/json"
	"fmt"
	"log/slog"
	"net/http"
)

const (
	// ProtocolVersion is the MCP protocol version this server implements.
	ProtocolVersion = "2025-03-26"
	defaultName     = "mcpserver"
	defaultVersion  = "1.0.0"
)

// ToolHandlerFunc is the signature for tool execution handlers.
type ToolHandlerFunc func(ctx context.Context, args map[string]any) (ToolResult, error)

// ResourceHandlerFunc is the signature for resource read handlers.
type ResourceHandlerFunc func(ctx context.Context, uri string) (ResourceContent, error)

// Server is a reusable MCP server. Create with New(), register handlers,
// then run via HTTPHandler(). For Lambda, use the transport adapter in your app.
type Server struct {
	logger           Logger
	serverName       string
	serverVersion    string
	tools            map[string]Tool
	toolList         []Tool
	resources        map[string]Resource
	resList          []Resource
	toolHandlers     map[string]ToolHandlerFunc
	resourceHandlers map[string]ResourceHandlerFunc
}

// Option configures the MCP server.
type Option func(*serverConfig)

type serverConfig struct {
	tools         []Tool
	resources     []Resource
	serverName    string
	serverVersion string
	logger        Logger
}

// WithToolsFile loads tool definitions from a JSON file on disk.
func WithToolsFile(path string) Option {
	return func(cfg *serverConfig) {
		tools, err := LoadTools(path)
		if err != nil {
			if cfg.logger != nil {
				cfg.logger.Error("load tools file", "path", path, "err", err)
			}
			return
		}
		cfg.tools = append(cfg.tools, tools...)
	}
}

// WithTools passes tool definitions directly.
func WithTools(tools []Tool) Option {
	return func(cfg *serverConfig) {
		cfg.tools = append(cfg.tools, tools...)
	}
}

// WithToolsJSON parses tool definitions from raw JSON bytes (useful with go:embed).
func WithToolsJSON(data []byte) Option {
	return func(cfg *serverConfig) {
		tools, err := ParseTools(data)
		if err != nil {
			if cfg.logger != nil {
				cfg.logger.Error("parse tools json", "err", err)
			}
			return
		}
		cfg.tools = append(cfg.tools, tools...)
	}
}

// WithResourcesFile loads resource definitions from a JSON file on disk.
func WithResourcesFile(path string) Option {
	return func(cfg *serverConfig) {
		resources, err := LoadResources(path)
		if err != nil {
			if cfg.logger != nil {
				cfg.logger.Error("load resources file", "path", path, "err", err)
			}
			return
		}
		cfg.resources = append(cfg.resources, resources...)
	}
}

// WithResources passes resource definitions directly.
func WithResources(resources []Resource) Option {
	return func(cfg *serverConfig) {
		cfg.resources = append(cfg.resources, resources...)
	}
}

// WithResourcesJSON parses resource definitions from raw JSON bytes (useful with go:embed).
func WithResourcesJSON(data []byte) Option {
	return func(cfg *serverConfig) {
		resources, err := ParseResources(data)
		if err != nil {
			if cfg.logger != nil {
				cfg.logger.Error("parse resources json", "err", err)
			}
			return
		}
		cfg.resources = append(cfg.resources, resources...)
	}
}

// WithServerInfo sets the server name and version returned in initialize.
func WithServerInfo(name, version string) Option {
	return func(cfg *serverConfig) {
		cfg.serverName = name
		cfg.serverVersion = version
	}
}

// WithLogger sets a custom logger. Defaults to slog.Default().
func WithLogger(l Logger) Option {
	return func(cfg *serverConfig) {
		cfg.logger = l
	}
}

// New creates a new MCP server with the given options.
func New(opts ...Option) *Server {
	cfg := serverConfig{
		serverName:    defaultName,
		serverVersion: defaultVersion,
	}

	// First pass: set logger so options that load files can log errors.
	for _, opt := range opts {
		opt(&cfg)
	}
	if cfg.logger == nil {
		cfg.logger = slog.Default()
	}

	tMap := make(map[string]Tool, len(cfg.tools))
	for _, tool := range cfg.tools {
		tMap[tool.Name] = tool
	}
	rMap := make(map[string]Resource, len(cfg.resources))
	for _, res := range cfg.resources {
		rMap[res.Name] = res
	}

	return &Server{
		logger:           cfg.logger,
		serverName:       cfg.serverName,
		serverVersion:    cfg.serverVersion,
		tools:            tMap,
		toolList:         cfg.tools,
		resources:        rMap,
		resList:          cfg.resources,
		toolHandlers:     make(map[string]ToolHandlerFunc),
		resourceHandlers: make(map[string]ResourceHandlerFunc),
	}
}

// HandleTool registers a handler function for the named tool.
func (s *Server) HandleTool(name string, fn ToolHandlerFunc) {
	s.toolHandlers[name] = fn
}

// HandleResource registers a handler function for the named resource.
func (s *Server) HandleResource(name string, fn ResourceHandlerFunc) {
	s.resourceHandlers[name] = fn
}

// HTTPHandler returns an http.Handler that serves MCP over Streamable HTTP.
func (s *Server) HTTPHandler() http.Handler {
	return newHTTPHandler(s)
}

// Handle routes a JSON-RPC request to the appropriate MCP handler.
// A response with IsNotification() == true signals the transport layer
// to return HTTP 202 with no body.
func (s *Server) Handle(ctx context.Context, req JSONRPCRequest) JSONRPCResponse {
	if req.JSONRPC != "2.0" {
		return NewErrorResponse(req.ID, ErrCodeInvalidReq, "jsonrpc must be '2.0'")
	}

	switch req.Method {
	case "initialize":
		return s.handleInitialize(req)
	case "ping":
		return s.handlePing(req)
	case "notifications/initialized", "notifications/cancelled":
		return JSONRPCResponse{} // sentinel: no body
	case "tools/list":
		return s.handleToolsList(req)
	case "tools/call":
		return s.handleToolsCall(ctx, req)
	case "resources/list":
		return s.handleResourcesList(req)
	case "resources/read":
		return s.handleResourcesRead(ctx, req)
	default:
		return NewErrorResponse(req.ID, ErrCodeNoMethod, fmt.Sprintf("Method not found: %s", req.Method))
	}
}

func (s *Server) handleInitialize(req JSONRPCRequest) JSONRPCResponse {
	var params initializeParams
	if len(req.Params) > 0 {
		if err := json.Unmarshal(req.Params, &params); err != nil {
			return NewErrorResponse(req.ID, ErrCodeBadParams, fmt.Sprintf("invalid params: %v", err))
		}
	}

	s.logger.Info("initialize",
		"clientName", params.ClientInfo.Name,
		"clientVersion", params.ClientInfo.Version,
		"protocolVersion", params.ProtocolVersion,
	)

	result := map[string]any{
		"protocolVersion": ProtocolVersion,
		"capabilities": map[string]any{
			"tools":     map[string]any{"listChanged": false},
			"resources": map[string]any{"subscribe": false, "listChanged": false},
		},
		"serverInfo": map[string]any{
			"name":    s.serverName,
			"version": s.serverVersion,
		},
	}
	buf, _ := json.Marshal(result)
	return JSONRPCResponse{JSONRPC: "2.0", ID: req.ID, Result: buf}
}

func (s *Server) handlePing(req JSONRPCRequest) JSONRPCResponse {
	buf, _ := json.Marshal(struct{}{})
	return JSONRPCResponse{JSONRPC: "2.0", ID: req.ID, Result: buf}
}

func (s *Server) handleToolsList(req JSONRPCRequest) JSONRPCResponse {
	payload := struct {
		Tools []Tool `json:"tools"`
	}{Tools: s.toolList}
	buf, _ := json.Marshal(payload)
	return JSONRPCResponse{JSONRPC: "2.0", ID: req.ID, Result: buf}
}

func (s *Server) handleToolsCall(ctx context.Context, req JSONRPCRequest) JSONRPCResponse {
	var params toolCallParams
	if len(req.Params) > 0 {
		if err := json.Unmarshal(req.Params, &params); err != nil {
			return NewErrorResponse(req.ID, ErrCodeBadParams, fmt.Sprintf("invalid params: %v", err))
		}
	}
	if params.Arguments == nil {
		params.Arguments = map[string]any{}
	}

	tool, ok := s.tools[params.Name]
	if !ok {
		return NewErrorResponse(req.ID, ErrCodeNoMethod, fmt.Sprintf("Unknown tool: %s", params.Name))
	}
	if err := tool.ValidateArguments(params.Arguments); err != nil {
		return NewErrorResponse(req.ID, ErrCodeBadParams, err.Error())
	}

	handler, ok := s.toolHandlers[params.Name]
	if !ok {
		return NewErrorResponse(req.ID, ErrCodeInternal, fmt.Sprintf("no handler for tool: %s", params.Name))
	}

	result, err := handler(ctx, params.Arguments)
	if err != nil {
		result = ErrorResult(err.Error())
	}

	buf, _ := json.Marshal(result)
	return JSONRPCResponse{JSONRPC: "2.0", ID: req.ID, Result: buf}
}

func (s *Server) handleResourcesList(req JSONRPCRequest) JSONRPCResponse {
	payload := struct {
		Resources []Resource `json:"resources"`
	}{Resources: s.resList}
	buf, _ := json.Marshal(payload)
	return JSONRPCResponse{JSONRPC: "2.0", ID: req.ID, Result: buf}
}

func (s *Server) handleResourcesRead(ctx context.Context, req JSONRPCRequest) JSONRPCResponse {
	var params resourceReadParams
	if len(req.Params) > 0 {
		if err := json.Unmarshal(req.Params, &params); err != nil {
			return NewErrorResponse(req.ID, ErrCodeBadParams, fmt.Sprintf("invalid params: %v", err))
		}
	}
	if params.Name == "" && params.URI == "" {
		return NewErrorResponse(req.ID, ErrCodeBadParams, "either name or uri must be provided")
	}

	// Resolve resource by name or URI.
	var target Resource
	var found bool
	if params.Name != "" {
		target, found = s.resources[params.Name]
	} else {
		for _, res := range s.resList {
			if res.URI == params.URI {
				target = res
				found = true
				break
			}
		}
	}
	if !found {
		return NewErrorResponse(req.ID, ErrCodeBadParams, "resource not found")
	}

	// Check for a registered handler.
	handler, hasHandler := s.resourceHandlers[target.Name]
	if !hasHandler {
		// Fallback: return metadata only.
		payload := map[string]any{
			"contents": []map[string]any{
				{"uri": target.URI, "mimeType": target.MimeType, "text": ""},
			},
		}
		buf, _ := json.Marshal(payload)
		return JSONRPCResponse{JSONRPC: "2.0", ID: req.ID, Result: buf}
	}

	content, err := handler(ctx, target.URI)
	if err != nil {
		return NewErrorResponse(req.ID, ErrCodeInternal, fmt.Sprintf("read resource: %v", err))
	}

	payload := map[string]any{
		"contents": []ResourceContent{content},
	}
	buf, _ := json.Marshal(payload)
	return JSONRPCResponse{JSONRPC: "2.0", ID: req.ID, Result: buf}
}
