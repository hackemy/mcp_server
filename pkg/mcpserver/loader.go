package mcpserver

import (
	"encoding/json"
	"fmt"
	"os"
)

// LoadTools reads and parses a tools JSON file from disk.
func LoadTools(path string) ([]Tool, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, err
	}
	return ParseTools(data)
}

// ParseTools parses tools from raw JSON bytes (useful with go:embed).
func ParseTools(data []byte) ([]Tool, error) {
	var tools []Tool
	if err := json.Unmarshal(data, &tools); err != nil {
		return nil, err
	}
	for i := range tools {
		var meta schemaMeta
		if err := json.Unmarshal(tools[i].InputSchema, &meta); err != nil {
			return nil, fmt.Errorf("tool %s schema: %w", tools[i].Name, err)
		}
		tools[i].schemaMeta = meta
	}
	return tools, nil
}

// LoadResources reads and parses a resources JSON file from disk.
func LoadResources(path string) ([]Resource, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, err
	}
	return ParseResources(data)
}

// ParseResources parses resources from raw JSON bytes (useful with go:embed).
func ParseResources(data []byte) ([]Resource, error) {
	var resources []Resource
	if err := json.Unmarshal(data, &resources); err != nil {
		return nil, err
	}
	return resources, nil
}
