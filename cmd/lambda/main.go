package main

import (
	"log/slog"
	"os"

	"github.com/aws/aws-lambda-go/lambda"

	"mcpserver"

	"marketplace/internal/tools"
	"marketplace/internal/transport"
)

func main() {
	logger := slog.New(slog.NewJSONHandler(os.Stdout, &slog.HandlerOptions{Level: slog.LevelInfo}))

	srv := mcpserver.New(
		mcpserver.WithToolsFile("config/tools.json"),
		mcpserver.WithResourcesFile("config/resources.json"),
		mcpserver.WithServerInfo("marketplace-mcp", "1.0.0"),
		mcpserver.WithLogger(logger),
	)

	tools.RegisterAll(srv)

	adapter := transport.NewLambdaAdapter(srv)
	lambda.Start(adapter.Handle)
}
