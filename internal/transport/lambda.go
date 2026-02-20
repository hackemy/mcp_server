package transport

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"net/http"
	"strings"

	"github.com/aws/aws-lambda-go/events"

	"mcpserver"
)

// LambdaAdapter wraps an MCP Server for AWS Lambda API Gateway V2.
type LambdaAdapter struct {
	server *mcpserver.Server
}

// NewLambdaAdapter creates a Lambda adapter for the given MCP server.
func NewLambdaAdapter(server *mcpserver.Server) *LambdaAdapter {
	return &LambdaAdapter{server: server}
}

// Handle satisfies the aws-lambda-go handler signature for API Gateway V2 HTTP APIs.
func (a *LambdaAdapter) Handle(ctx context.Context, request events.APIGatewayV2HTTPRequest) (events.APIGatewayV2HTTPResponse, error) {
	method := request.RequestContext.HTTP.Method
	path := request.RawPath

	switch {
	case strings.EqualFold(method, http.MethodGet) && path == "/healthz":
		return events.APIGatewayV2HTTPResponse{
			StatusCode: http.StatusOK,
			Headers:    map[string]string{"content-type": "application/json"},
			Body:       `{"status":"ok"}`,
		}, nil

	case strings.EqualFold(method, http.MethodPost) && path == "/mcp":
		return a.handleJSONRPC(ctx, request)

	default:
		return events.APIGatewayV2HTTPResponse{
			StatusCode: http.StatusNotFound,
			Headers:    map[string]string{"content-type": "application/json"},
			Body:       `{"error":"route_not_found"}`,
		}, nil
	}
}

func (a *LambdaAdapter) handleJSONRPC(ctx context.Context, request events.APIGatewayV2HTTPRequest) (events.APIGatewayV2HTTPResponse, error) {
	body := request.Body
	if request.IsBase64Encoded {
		decoded, err := base64.StdEncoding.DecodeString(body)
		if err != nil {
			return lambdaError(http.StatusBadRequest, fmt.Errorf("base64 decode: %w", err)), nil
		}
		body = string(decoded)
	}

	var rpcReq mcpserver.JSONRPCRequest
	if err := json.Unmarshal([]byte(body), &rpcReq); err != nil {
		resp := mcpserver.NewErrorResponse(nil, mcpserver.ErrCodeParse, "invalid JSON: "+err.Error())
		return lambdaJSON(resp, http.StatusBadRequest), nil
	}

	// Notifications produce no response body.
	if strings.HasPrefix(rpcReq.Method, "notifications/") {
		return events.APIGatewayV2HTTPResponse{StatusCode: http.StatusAccepted}, nil
	}

	resp := a.server.Handle(ctx, rpcReq)

	if resp.IsNotification() {
		return events.APIGatewayV2HTTPResponse{StatusCode: http.StatusAccepted}, nil
	}

	return lambdaJSON(resp, http.StatusOK), nil
}

func lambdaJSON(payload any, status int) events.APIGatewayV2HTTPResponse {
	buf, err := json.Marshal(payload)
	if err != nil {
		return events.APIGatewayV2HTTPResponse{
			StatusCode: http.StatusInternalServerError,
			Headers:    map[string]string{"content-type": "application/json"},
			Body:       fmt.Sprintf(`{"error":"%s"}`, err),
		}
	}
	return events.APIGatewayV2HTTPResponse{
		StatusCode: status,
		Headers:    map[string]string{"content-type": "application/json"},
		Body:       string(buf),
	}
}

func lambdaError(status int, err error) events.APIGatewayV2HTTPResponse {
	return events.APIGatewayV2HTTPResponse{
		StatusCode: status,
		Headers:    map[string]string{"content-type": "application/json"},
		Body:       fmt.Sprintf(`{"error":"%s"}`, err),
	}
}
