package main

import (
	"log"
	"log/slog"
	"net/http"
	"os"

	"mcpserver"

	"marketplace/internal/tools"
)

func main() {
	logger := slog.New(slog.NewJSONHandler(os.Stdout, &slog.HandlerOptions{Level: slog.LevelDebug}))

	srv := mcpserver.New(
		mcpserver.WithToolsFile("config/tools.json"),
		mcpserver.WithResourcesFile("config/resources.json"),
		mcpserver.WithServerInfo("marketplace-mcp", "1.0.0"),
		mcpserver.WithLogger(logger),
	)

	tools.RegisterAll(srv)

	addr := ":8080"
	if port := os.Getenv("PORT"); port != "" {
		addr = ":" + port
	}

	logger.Info("starting MCP server", "addr", addr)
	log.Fatal(http.ListenAndServe(addr, srv.HTTPHandler()))
}
