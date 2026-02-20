module marketplace

go 1.22

require (
	github.com/aws/aws-lambda-go v1.47.0
	mcpserver v0.0.0
)

require github.com/google/uuid v1.6.0 // indirect

replace mcpserver => ./pkg/mcpserver
