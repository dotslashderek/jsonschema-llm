module github.com/dotslashderek/json-schema-llm/examples/stress-test-bot-go

go 1.22

require (
	github.com/dotslashderek/json-schema-llm/bindings/go v0.0.0
	github.com/openai/openai-go v0.1.0-alpha.41
	github.com/santhosh-tekuri/jsonschema/v6 v6.0.1
)

require (
	github.com/tetratelabs/wazero v1.8.2 // indirect
	github.com/tidwall/gjson v1.14.4 // indirect
	github.com/tidwall/match v1.1.1 // indirect
	github.com/tidwall/pretty v1.2.1 // indirect
	github.com/tidwall/sjson v1.2.5 // indirect
	golang.org/x/text v0.16.0 // indirect
)

replace github.com/dotslashderek/json-schema-llm/bindings/go => ../../bindings/go
