package tools

import (
	"context"

	"mcpserver"
)

// ToolNames lists all tools defined in config/tools.json.
var ToolNames = []string{
	"account-delete",
	"web-push-enable",
	"web-push-disable",
	"otp-request",
	"otp-verify",
	"channel-notify",
	"channel-messages",
	"channel-put",
	"channel-delete",
	"channels-list",
	"channels-for-category",
	"channel-subscribe",
	"channel-unsubscribe",
	"subscriptions-list",
}

// RegisterAll registers stub handlers for all known tools on the server.
func RegisterAll(srv *mcpserver.Server) {
	for _, name := range ToolNames {
		n := name // capture for closure
		srv.HandleTool(n, func(_ context.Context, _ map[string]any) (mcpserver.ToolResult, error) {
			return mcpserver.TextResult("stub: " + n + " accepted"), nil
		})
	}
}
