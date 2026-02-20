package mcpserver

import "fmt"

// ValidateArguments checks tool arguments against the schema metadata.
func (t Tool) ValidateArguments(args map[string]any) error {
	if len(t.schemaMeta.Required) > 0 {
		for _, field := range t.schemaMeta.Required {
			if _, ok := args[field]; !ok {
				return fmt.Errorf("missing required field %q", field)
			}
		}
	}

	if len(t.schemaMeta.OneOf) > 0 {
		matched := false
		for _, set := range t.schemaMeta.OneOf {
			allPresent := true
			for _, field := range set.Required {
				if _, ok := args[field]; !ok {
					allPresent = false
					break
				}
			}
			if allPresent {
				matched = true
				break
			}
		}
		if !matched {
			return fmt.Errorf("arguments must satisfy oneOf requirements")
		}
	}

	if len(t.schemaMeta.Dependencies) > 0 {
		for field, deps := range t.schemaMeta.Dependencies {
			if _, ok := args[field]; ok {
				for _, depField := range deps {
					if _, depOK := args[depField]; !depOK {
						return fmt.Errorf("field %q requires %q", field, depField)
					}
				}
			}
		}
	}

	return nil
}
